//! 目的：turn 侧取「一席事实」的地方——沙箱清单、权限链与生效权限、工具环境事实（含 `tools::api::member_env`）、上下文预算，都从这里从端口读齐。
//! 管：`sandboxes` / `permission_chain` / `permissions_for` / `shared_scope_for` / `access_for_agent` 的取数与派生，
//!   `member_env` 的入参收口与逐字段拷贝、`member_face` 的薄入口、`build_single` 的整场装配，`context_budget` 的窗口与阈值。
//! 不管：装配规则（在 `agentloop::api`）、登记处里有什么（`registry`）、会话怎么落盘（`service/admit.rs`）。
//! 联动：`runtime/service/env.rs` 的薄入口转调这里；`service/rebuild.rs` 重建会话时也取这里的事实。

use std::collections::BTreeMap;

use crate::kernel::ports::HostProbe;
use crate::units::business::catalog::modules::api::{Module, Roster};
use crate::units::business::catalog::packages::api::PackagePlan;
use crate::units::business::catalog::registry::api::Registry;

#[cfg(test)]
use crate::units::business::agentloop::api::member_face;
use crate::units::business::agentloop::api::{
    AgentSession, MemberEnvParts, MemberSpec, SingleSpec, single_session,
};
use crate::units::business::conversation::session::api::work_root_of;
use crate::units::business::llm::api::Channel;
use crate::units::business::session_model::api::SessionMeta;
use crate::units::business::transcript::api::SessionEvent;
use crate::units::business::turn_state::api::Turn;
use crate::units::business::vm::api::GuestOs;
use crate::units::business::workspace::api::{
    ModuleDir, Sandbox, Sandboxes, SeatFacts, SeatPlan, SharedScope,
};
use crate::units::pure::exec_model::api::Side;
use crate::units::pure::permission::api::PermissionSet;
use crate::units::pure::registry_model::api::AgentMeta;
#[cfg(test)]
use crate::units::pure::registry_model::api::ToolMode;

/// 目的：一席事实的取用：沙箱、权限、工具面入参与上下文预算。
impl Turn {
    /// 目的：组装本次工作的沙箱清单：工作根来自 Workspace 端口，每席的私有根与模块目录交给工作台单元落值。
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
        let codec =
            crate::units::business::vm::api::path_codec_of(&self.root, probe, &meta.target());
        let modules: Vec<ModuleDir> = roster
            .modules
            .iter()
            .map(|m| ModuleDir {
                id: m.manifest.id.clone(),
                root: m.root.clone(),
                private: m.private.clone(),
            })
            .collect();
        let mut seats: Vec<SeatPlan> = Vec::new();
        for a in &meta.agents {
            let (permissions, params) = self.access_for_agent(registry, meta, &a.name)?;
            seats.push(SeatPlan {
                agent: a.name.clone(),
                modules: a.modules.clone(),
                permissions,
                params,
                private: None,
            });
        }
        let app = registry.app();
        crate::units::business::workspace::api::seat_sandboxes(&SeatFacts {
            work: &work,
            session: &meta.name,
            roots: &roots,
            codec,
            base_scope: &app.sandbox.scope,
            hard_deny: &app.sandbox.policy.deny,
            modules: &modules,
            seats: &seats,
        })
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
        // 合成规则（默认 → 根 ceiling → 每层 narrowing）在会话模型：这里只取链与全局默认。
        let app = registry.app();
        crate::units::business::session_model::api::permissions(&chain, &app.sandbox.defaults)
    }

    /// 目的：一条会话的**生效共享区归属**：全局默认 → 根覆盖 → 每层收窄 → 全局硬黑名单。
    pub(crate) fn shared_scope_for(
        &self,
        registry: &dyn Registry,
        meta: &SessionMeta,
    ) -> Result<SharedScope, String> {
        let params = self.workspace_params_chain(meta)?;
        let app = registry.app();
        // 合成规则（全局默认 → 根覆盖 → 每层收窄 → 硬黑名单）在工作台单元：会话只搬声明。
        crate::units::business::workspace::api::scope_from(
            &params,
            &app.sandbox.scope,
            &app.sandbox.policy.deny,
        )
    }

    /// 目的：某个 agent 的生效授权：协作主会话里的发言席优先取它自己的子会话（收窄层在那里）。
    /// 返回：生效网络档位与根在前的策略参数链（共享区归属由工作台单元按链派生）。
    fn access_for_agent(
        &self,
        registry: &dyn Registry,
        meta: &SessionMeta,
        agent: &str,
    ) -> Result<(PermissionSet, Vec<serde_json::Value>), String> {
        let target = self.seat_meta(meta, agent).unwrap_or_else(|| meta.clone());
        Ok((
            self.permissions_for(registry, &target)?,
            self.workspace_params_chain(&target)?,
        ))
    }

    /// 目的：一条会话的策略参数链——根在前，自己最后（沿 parent 走到顶）。
    /// 约束：只搬声明里那一层不透明参数，不解释字段（解释权在工作台单元）。
    pub(crate) fn workspace_params_chain(
        &self,
        meta: &SessionMeta,
    ) -> Result<Vec<serde_json::Value>, String> {
        Ok(self
            .permission_chain(meta)?
            .into_iter()
            .map(|m| m.workspace.params)
            .collect())
    }

    /// 目的：协作主会话里某一席真正的会话元信息（发言席有自己的子会话就用它，否则就是这条会话）。
    fn seat_meta(&self, meta: &SessionMeta, agent: &str) -> Option<SessionMeta> {
        if meta.agents.len() == 1 && meta.agents[0].name == agent {
            return None;
        }
        let child_name = format!("{}--{}", meta.name, agent);
        self.history
            .meta(&child_name)
            .ok()
            .filter(|child| child.parent.as_deref() == Some(meta.name.as_str()))
    }

    /// 目的：一席的工具环境事实：模块注入项 + 宿主可读根 / 网络 / 档位 / 包计划，交给 `env` 派生后**逐字段拷成值结构**。
    /// 参数：`registry` = 登记处（宿主可读根）；`host_plan` = 本席的包计划。
    /// 约束：`env` 的调用留在这里——装配机在环 2 的 `agentloop`，拿不到 `env`；它只收 `MemberEnvParts` 这个值。
    pub(crate) fn member_env(
        &self,
        registry: &dyn Registry,
        modules: &[Module],
        sb: &Sandbox,
        net: bool,
        side: Side,
        host_plan: &PackagePlan,
    ) -> MemberEnvParts {
        let module_vars: BTreeMap<String, Vec<(String, String)>> = modules
            .iter()
            .map(|m| {
                (
                    m.manifest.id.clone(),
                    self.secrets.resolve(&m.manifest.id).unwrap_or_default(),
                )
            })
            .collect();
        let host_read_roots = crate::units::business::tools::api::host_read_roots(
            &registry.app().sandbox.policy.host_read_roots,
        );
        let menv = crate::units::business::tools::api::member_env(
            &crate::units::business::tools::api::MemberEnvFacts {
                sandbox: sb,
                host_read_roots: &host_read_roots,
                net,
                side,
                base_roots: &self.base_roots,
                package_roots: &host_plan.roots,
                package_path: host_plan.path.clone(),
                module_vars: &module_vars,
            },
        );
        MemberEnvParts {
            fence: menv.fence,
            module_env: menv.module_env,
            runtime_path: menv.runtime_path,
            runtime_roots: menv.runtime_roots,
        }
    }

    /// 目的：一席的**工具面值**（围栏 / 工具链根 / 发放名单）薄入口：给不建会话、只看围栏的调用方与测试。
    /// 约束：只回值结构，工具面（活对象）不出 `agentloop`。
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
        // 这一回合的段与角色：用户建的单 agent 会话 = standard/solo，协作子会话 = 流程的成员角色。
        loop_id: &str,
        role: &str,
        side: Side,
        guest_lease: String,
        host_plan: &PackagePlan,
    ) -> crate::units::business::agentloop::api::ToolFace {
        let source_ids: Vec<String> = ["agent", "module", "workspace"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let abilities: Vec<String> = crate::units::pure::registry_model::api::DEFAULT_ABILITIES
            .iter()
            .map(|s| s.to_string())
            .collect();
        let spec = MemberSpec {
            modules,
            sandbox: sb,
            unavailable,
            mode,
            loop_id,
            role,
            side,
            guest_lease,
            shell: self.shell_slot(&sb.session),
            env: self.member_env(registry, modules, sb, net, side, host_plan),
            source_ids: &source_ids,
            system_tools: &[],
            abilities: &abilities,
            profile: "",
        };
        member_face(&self.session_ports(), &spec)
    }

    /// 目的：单 agent 会话的整场装配——取本席事实（含 `env` 派生）、解析形态与预算，交给 `agentloop` 的装配机。
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
        side: Side,
        guest: Option<GuestOs>,
        guest_lease: String,
        host_plan: &PackagePlan,
        // 会话形状（会话类型展开后的声明）：工具来源与 system 工具都按它发。
        shape: &crate::units::business::session_model::api::SessionSpec,
    ) -> (AgentSession, Vec<SessionEvent>) {
        // 形态按登记处解析；通道已在前置判据里解析成功，这里不再有"没有通道"的分支。
        let abilities = a.abilities();
        let spec = SingleSpec {
            agent: a,
            member: MemberSpec {
                modules,
                sandbox: sb,
                unavailable,
                mode: registry.tool_mode(a.model.as_deref()),
                // 单 agent 工作：用户自己开的那场对话（不是任务链的节点）——坐会话类型声明的那一席。
                loop_id: &shape.loop_id,
                role: &shape.role,
                side,
                guest_lease,
                shell: self.shell_slot(&sb.session),
                env: self.member_env(registry, modules, sb, net, side, host_plan),
                source_ids: &shape.sources,
                system_tools: &shape.system_tools,
                abilities: &abilities,
                profile: &shape.kind,
            },
            channel,
            guest,
            context: self.context_budget(registry, a.model.as_deref()),
            core_model: registry.core_model(),
        };
        single_session(&self.session_ports(), &spec)
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
