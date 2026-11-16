//! 目的：turn 侧取「一席事实」的地方——沙箱清单、权限链与生效权限、外部工作区授权粒度、工具环境事实（含 `env::api::member`）、上下文预算，都从这里从端口读齐。
//! 管：`sandboxes` / `permission_chain` / `permissions_for` / `workspace_grant` / `permission_for_agent` 的取数与派生，
//!   `member_env` 的入参收口与逐字段拷贝、`member_face` 的薄入口、`build_single` 的整场装配，`context_budget` 的窗口与阈值。
//! 不管：装配规则（在 `session::service::assemble`）、登记处里有什么（`registry`）、会话怎么落盘（`service/admit.rs`）。
//! 联动：`runtime/service/env.rs` 的薄入口转调这里；`service/rebuild.rs` 重建会话时也取这里的事实。

use std::collections::BTreeMap;

use crate::capabilities::llm::api::Channel;
#[cfg(test)]
use crate::capabilities::llm::api::ToolMode;
use crate::capabilities::modules::api::{Module, Roster};
use crate::capabilities::packages::api::PackagePlan;
use crate::capabilities::permission::api::PermissionSet;
use crate::capabilities::registry::api::Registry;
use crate::capabilities::session::api::{
    AgentMeta, AgentSession, MemberSpec, SessionEvent, SessionMeta, SingleSpec, WorkspaceGrant,
    work_root_of,
};
use crate::capabilities::turn::service::session::Turn;
use crate::capabilities::vm::api::GuestOs;
use crate::capabilities::workspace::api::{Sandbox, Sandboxes};
use crate::kernel::api::Tier;
use crate::kernel::ports::HostProbe;

/// 目的：一席事实的取用：沙箱、权限、工作区授权、工具面入参与上下文预算。
impl Turn {
    /// 目的：组装本次工作的沙箱清单：工作根来自 Workspace 端口，模块目录来自清单。
    /// 参数：`registry` = 登记处；`probe` = 宿主探针（虚拟机档据此给 guest↔宿主翻译器）。
    /// 约束：模块目录只对使用它的 agent 可达；同一模块可被多个 agent 共用（各自独立沙箱与工具面）。
    pub(crate) fn sandboxes(
        &self,
        registry: &dyn Registry,
        probe: &dyn HostProbe,
        meta: &SessionMeta,
        roster: &Roster,
    ) -> Result<Sandboxes, String> {
        let names: Vec<String> = meta.agents.iter().map(|a| a.name.clone()).collect();
        // 沙箱锚在**顶层工作**上：整棵树不论嵌套多少层只有一个 work/。
        let work = work_root_of(self.history.as_ref(), meta)?;
        let roots = self.workspace.roots(&work, &names)?;
        // 虚拟机档给 guest↔宿主翻译器：system 工具跑在宿主侧，但收模型给的 guest 写法；
        // **同一份**也交给 @ 改写（挂载点、命令装配与用户引用用同一座桥，三处不会各给一种写法）。
        let path_codec = crate::capabilities::vm::api::path_codec_of(&self.root, probe, &meta.exec);
        // 外部工作区的授权粒度：只影响围栏派生（共享区是否要每会话重授），不改权限面。
        let shared_grant = self.workspace_grant(meta)?;
        // 逐席组装的规则在 env：模块目录只对使用它的 agent 可达，私有根缺一份就如实报缺哪个 agent。
        crate::capabilities::env::api::sandboxes(
            &crate::capabilities::env::api::SandboxesFacts {
                work: &work,
                session: &meta.name,
                agents: &meta.agents,
                roots: &roots,
                roster,
                codec: path_codec,
                grant: shared_grant,
            },
            &|agent| self.permission_for_agent(registry, meta, agent),
        )
    }

    /// 目的：一条会话的权限链——根在前，自己最后（沿 parent 走到顶）。
    fn permission_chain(&self, meta: &SessionMeta) -> Result<Vec<SessionMeta>, String> {
        let mut chain = vec![meta.clone()];
        let mut cur = meta.clone();
        while let Some(parent) = cur.parent.clone() {
            let pm = self.history.meta(&parent)?;
            chain.push(pm.clone());
            cur = pm;
        }
        chain.reverse();
        Ok(chain)
    }

    /// 目的：一条会话的**生效权限**：全局默认 → 根 ceiling → 每层 narrowing → 全局硬黑名单。
    pub(crate) fn permissions_for(
        &self,
        registry: &dyn Registry,
        meta: &SessionMeta,
    ) -> Result<PermissionSet, String> {
        let chain = self.permission_chain(meta)?;
        // 合成规则（默认 → 根 ceiling → 每层 narrowing → 硬黑名单 → 无共享区授权就收紧）在 env。
        let app = registry.app();
        crate::capabilities::env::api::permissions(&chain, app)
    }

    /// 目的：这条会话生效的外部工作区授权粒度：沿权限链找最近一份绑定（创建时只写在根），
    ///   没有绑定就是 `session` 缺省。
    pub(crate) fn workspace_grant(&self, meta: &SessionMeta) -> Result<WorkspaceGrant, String> {
        let chain = self.permission_chain(meta)?;
        Ok(crate::capabilities::env::api::workspace_grant(&chain))
    }

    /// 目的：某个 agent 的生效权限：协作主会话里的发言席优先取它自己的子会话 narrowing。
    fn permission_for_agent(
        &self,
        registry: &dyn Registry,
        meta: &SessionMeta,
        agent: &str,
    ) -> Result<PermissionSet, String> {
        if meta.agents.len() == 1 && meta.agents[0].name == agent {
            return self.permissions_for(registry, meta);
        }
        let child_name = format!("{}--{}", meta.name, agent);
        if let Ok(child) = self.history.meta(&child_name)
            && child.parent.as_deref() == Some(meta.name.as_str())
        {
            return self.permissions_for(registry, &child);
        }
        self.permissions_for(registry, meta)
    }

    /// 目的：一席的工具环境事实：模块注入项 + 宿主可读根 / 网络 / 档位 / 包计划，交给 `env` 派生后**逐字段拷成值结构**。
    /// 参数：`registry` = 登记处（宿主可读根）；`host_plan` = 本席的包计划。
    /// 约束：`env` 的调用留在这里——`session` 是第 4 层，拿不到 `env`；它只收 `MemberEnvParts` 这个值。
    pub(crate) fn member_env(
        &self,
        registry: &dyn Registry,
        modules: &[Module],
        sb: &Sandbox,
        net: bool,
        tier: Tier,
        host_plan: &PackagePlan,
    ) -> crate::capabilities::session::api::MemberEnvParts {
        let module_vars: BTreeMap<String, Vec<(String, String)>> = modules
            .iter()
            .map(|m| {
                (
                    m.manifest.id.clone(),
                    self.secrets.resolve(&m.manifest.id).unwrap_or_default(),
                )
            })
            .collect();
        let host_read_roots = crate::capabilities::env::api::host_read_roots(registry.app());
        let menv =
            crate::capabilities::env::api::member(&crate::capabilities::env::api::MemberFacts {
                sandbox: sb,
                host_read_roots: &host_read_roots,
                net,
                tier,
                base_roots: &self.base_roots,
                plan: host_plan,
                module_vars: &module_vars,
            });
        crate::capabilities::session::api::MemberEnvParts {
            fence: menv.fence,
            module_env: menv.module_env,
            runtime_path: menv.runtime_path,
            runtime_roots: menv.runtime_roots,
        }
    }

    /// 目的：一席的**工具面值**（围栏 / 工具链根 / 发放名单）薄入口：给不建会话、只看围栏的调用方与测试。
    /// 约束：只回值结构，工具面（活对象）不出 `session`。
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn member_face(
        &self,
        registry: &dyn Registry,
        modules: &[Module],
        sb: &Sandbox,
        unavailable: BTreeMap<String, Vec<String>>,
        net: bool,
        mode: ToolMode,
        // 这一席的身份：用户建的单 agent 会话 = solo，协作子会话 = executor（见 systools/roles.yaml）。
        role: &str,
        tier: Tier,
        guest_lease: String,
        host_plan: &PackagePlan,
    ) -> crate::capabilities::session::api::ToolFace {
        let spec = crate::capabilities::session::api::MemberSpec {
            modules,
            sandbox: sb,
            unavailable,
            mode,
            role,
            tier,
            guest_lease,
            shell: self.shell_slot(&sb.session),
            env: self.member_env(registry, modules, sb, net, tier, host_plan),
        };
        crate::capabilities::session::api::member_face(&self.session_ports(), &spec)
    }

    /// 目的：单 agent 会话的整场装配——取本席事实（含 `env` 派生）、解析形态与预算，交给 `session` 装配。
    /// 约束：不插入会话中心；插入与落盘由调用方（`work` 的提交段）统一做。
    // 会话装配把这一场单 agent 会话一次说全；收口成参数对象只是把参数挪个地方。
    // 这是有意的设计取舍（见 docs/testing/quality-isolation.md 的 allow 清单），不是没修。
    #[allow(clippy::too_many_arguments)]
    pub fn build_single(
        &self,
        registry: &dyn Registry,
        a: &AgentMeta,
        modules: &[Module],
        channel: Channel,
        sb: &Sandbox,
        unavailable: BTreeMap<String, Vec<String>>,
        net: bool,
        tier: Tier,
        guest: Option<GuestOs>,
        guest_lease: String,
        host_plan: &PackagePlan,
    ) -> (AgentSession, Vec<SessionEvent>) {
        // 形态按登记处解析；通道已在前置判据里解析成功，这里不再有"没有通道"的分支。
        let spec = SingleSpec {
            agent: a,
            member: MemberSpec {
                modules,
                sandbox: sb,
                unavailable,
                mode: registry.tool_mode(a.model.as_deref()),
                // 单 agent 工作：用户自己开的那场对话（不是任务链的节点）——身份是 solo。
                role: "solo",
                tier,
                guest_lease,
                shell: self.shell_slot(&sb.session),
                env: self.member_env(registry, modules, sb, net, tier, host_plan),
            },
            channel,
            guest,
            context: self.context_budget(registry, a.model.as_deref()),
            core_model: registry.core_model(),
        };
        crate::capabilities::session::api::single_session(&self.session_ports(), &spec)
    }

    /// 目的：本会话的上下文预算：窗口按模型现算（缺省用核心默认），阈值与保留百分比来自全局设置。
    /// 约束：阈值 0% = 关自动压缩。见 docs/session/session-model.md 六。
    pub(crate) fn context_budget(
        &self,
        registry: &dyn Registry,
        model: Option<&str>,
    ) -> (u64, u8, u8) {
        // 窗口是登记处的事实，不在这里另存一份。
        (
            registry.context_of(model),
            registry.app().compact_at_percent,
            registry.app().compact_keep_percent,
        )
    }
}
