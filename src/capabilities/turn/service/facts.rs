//! 目的：turn 侧取「一席事实」的地方——沙箱清单、权限链与生效权限、外部工作区授权粒度、工具面事实、上下文预算，都从这里从端口读齐。
//! 管：`sandboxes` / `permission_chain` / `permissions_for` / `workspace_grant` / `permission_for_agent` 的取数与派生，
//!   `member_tools` 的入参收口，`context_budget` 的窗口与阈值。
//! 不管：装配规则（`service/assemble.rs`）、登记处里有什么（`registry`）、会话怎么落盘（`service/admit.rs`）。
//! 联动：`conductor/service/env.rs` 的薄入口转调这里；`service/rebuild.rs` 重建会话时也取这里的事实。

use std::collections::BTreeMap;

use crate::capabilities::llm::api::ToolMode;
use crate::capabilities::modules::api::{Module, Roster};
use crate::capabilities::packages::api::PackagePlan;
use crate::capabilities::permission::api::PermissionSet;
use crate::capabilities::registry::api::Registry;
use crate::capabilities::session::api::{MemberTools, SessionMeta, WorkspaceGrant, work_root_of};
use crate::capabilities::turn::service::assemble::MemberSpec;
use crate::capabilities::turn::service::session::Turn;
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

    /// 目的：一席工具面的原件：模块 / 沙箱 / 缺包 / 网络 / 形态 / 角色 / 档位与工具链根一并交给装配。
    /// 参数：`registry` = 登记处（工具模式与宿主可读根）；`host_plan` = 本席的包计划。
    // 会话装配把"这一席的工具面"一次说全（模块 / 沙箱 / 缺包 / 网络 / 形态 / 角色 / 档位）。
    // 这是有意的设计取舍（见 docs/testing/quality-isolation.md 的 allow 清单），不是没修。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn member_tools(
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
    ) -> MemberTools {
        // 事实在这里取齐，装配规则在 turn（同一能力内，装配入口在 assemble.rs）。
        let spec = MemberSpec {
            modules,
            sandbox: sb,
            unavailable,
            net,
            mode,
            role,
            tier,
            guest_lease,
            plan: host_plan,
            base_roots: &self.base_roots,
            host_read_roots: crate::capabilities::env::api::host_read_roots(registry.app()),
            shell: self.shell_slot(&sb.session),
        };
        crate::capabilities::turn::api::member_tools(&self.session_deps(), &spec)
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
