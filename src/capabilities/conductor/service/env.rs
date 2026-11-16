//! 目的：conductor 侧的会话装配入口与沙箱 / 权限事实取用——工具环境与单 agent 会话对象只在这里转调 turn。
//! 管：guest 诊断准备、沙箱清单、权限链与生效权限、外部工作区授权粒度、上下文预算，以及 `tools_env` / `build_single` 两个薄入口。
//! 不管：装配本体（在 `turn` 的 `service/assemble.rs`）、会话怎么落盘（history.rs）、协作链怎么推进（flow.rs）、建工作的两段（work/create.rs）。
//! 联动：建工作与 turn 的准备工作经这里取会话对象；`turn` 用本文件取到的一席事实装配工具面。
use super::*;
use crate::capabilities::packages::api as pkgs;
use crate::capabilities::session::api::work_root_of;

impl Conductor {
    /// 目的：一次 guest 只读诊断的准备（毫秒级）：定位这条会话的 guest 租约。
    /// 约束：本机档没有 guest；代理父会话自己没有 guest，要诊断它下面的某个工作。
    pub(crate) fn prepare_guest_diagnose(
        &self,
        sid: &str,
    ) -> Result<super::guest_job::GuestDiagnose, String> {
        let meta = self.history.meta(sid)?;
        if meta.exec.tier != crate::kernel::api::Tier::Vm {
            return Err("这条会话是本机档，没有 guest 可诊断".to_string());
        }
        match crate::capabilities::session::api::guest_scope(self.history.as_ref(), &meta)? {
            Some((lease, _)) => Ok(super::guest_job::GuestDiagnose::new(
                Arc::clone(&self.vm),
                lease,
            )),
            None if meta.mode == "proxy" && meta.parent.is_none() => {
                Err("代理父会话本身没有 guest，请对它下面某个工作诊断".to_string())
            }
            None => Err("这条会话没有可诊断的 guest".to_string()),
        }
    }

    /// 目的：组装本次工作的沙箱清单：工作根来自 Workspace 端口，模块目录来自清单。
    ///  权限策略在此收口：模块目录只对使用它的 agent 可达；同一模块可被多个 agent 共用（各自独立沙箱与工具面）。
    ///  每个 agent 的沙箱按 meta.agents 建；共享区根任何时候都有（代拟还没名单时也有，@ 改写要用）。
    pub(crate) fn sandboxes(
        &self,
        meta: &SessionMeta,
        roster: &crate::capabilities::modules::api::Roster,
    ) -> Result<crate::capabilities::workspace::api::Sandboxes, String> {
        let names: Vec<String> = meta.agents.iter().map(|a| a.name.clone()).collect();
        // 沙箱锚在**顶层工作**上：整棵树不论嵌套多少层只有一个 work/（见 Conductor::work_root）。
        let work = work_root_of(self.history.as_ref(), meta)?;
        let roots = self.workspace.roots(&work, &names)?;
        // 虚拟机档给 guest↔宿主翻译器：system 工具跑在宿主侧，但收模型给的 guest 写法；
        // **同一份**也交给 @ 改写（挂载点、命令装配与用户引用用同一座桥，三处不会各给一种写法）。
        let path_codec = crate::capabilities::vm::api::path_codec_of(
            &self.root,
            self.probe.as_ref(),
            &meta.exec,
        );
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
            &|agent| self.permission_for_agent(meta, agent),
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
        meta: &SessionMeta,
    ) -> Result<crate::capabilities::permission::api::PermissionSet, String> {
        let chain = self.permission_chain(meta)?;
        // 合成规则（默认 → 根 ceiling → 每层 narrowing → 硬黑名单 → 无共享区授权就收紧）在 env。
        let app = self.registry.app();
        crate::capabilities::env::api::permissions(&chain, app)
    }

    /// 目的：这条会话生效的外部工作区授权粒度：沿权限链找最近一份绑定（创建时只写在根），
    ///   没有绑定就是 `session` 缺省。
    pub(crate) fn workspace_grant(
        &self,
        meta: &SessionMeta,
    ) -> Result<crate::capabilities::session::api::WorkspaceGrant, String> {
        let chain = self.permission_chain(meta)?;
        Ok(crate::capabilities::env::api::workspace_grant(&chain))
    }

    /// 目的：某个 agent 的生效权限：协作主会话里的发言席优先取它自己的子会话 narrowing。
    fn permission_for_agent(
        &self,
        meta: &SessionMeta,
        agent: &str,
    ) -> Result<crate::capabilities::permission::api::PermissionSet, String> {
        if meta.agents.len() == 1 && meta.agents[0].name == agent {
            return self.permissions_for(meta);
        }
        let child_name = format!("{}--{}", meta.name, agent);
        if let Ok(child) = self.history.meta(&child_name)
            && child.parent.as_deref() == Some(meta.name.as_str())
        {
            return self.permissions_for(&child);
        }
        self.permissions_for(meta)
    }

    /// 工具环境：内置文件工具永远可用；外部工具按模块分组放行（模块 id → 目录 + 工具表）。
    /// unavailable：本档位下缺运行包、不能执行工具的模块（机制侧据此拒绝执行，并如实报缺哪个能力）。
    /// net：会话是否放行出站网络（exec 段；默认否），随围栏交给机制层。
    // 会话装配把"这一席的工具面"一次说全（模块 / 沙箱 / 缺包 / 网络 / 形态 / 角色 / 档位）。
    // 这是有意的设计取舍（见 docs/testing/quality-isolation.md 的 allow 清单），不是没修。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn tools_env(
        &self,
        modules: &[Module],
        sb: &crate::capabilities::workspace::api::Sandbox,
        unavailable: BTreeMap<String, Vec<String>>,
        net: bool,
        mode: crate::capabilities::llm::api::ToolMode,
        // 这一席的身份：用户建的单 agent 会话 = solo，协作子会话 = executor（见 systools/roles.yaml）。
        role: &str,
        tier: crate::kernel::api::Tier,
        guest_lease: String,
        host_plan: &pkgs::PackagePlan,
    ) -> crate::capabilities::session::api::MemberTools {
        // 事实在这里取齐，装配规则在 turn（层 8 不依赖 conductor，所以入口留在这一侧）。
        let spec = crate::capabilities::turn::api::MemberSpec {
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
            host_read_roots: crate::capabilities::env::api::host_read_roots(self.registry.app()),
            shell: self.turn.shell_slot(&sb.session),
        };
        crate::capabilities::turn::api::member_tools(&self.turn.session_deps(), &spec)
    }

    /// 目的：一轮内对同一个成员最多提醒几次（用户可设，见 session-model.md 二）。
    pub fn discuss_remind_cap(&self) -> u32 {
        self.registry.app().discuss_remind_cap
    }

    /// 目的：本会话的上下文预算：窗口按模型现算（缺省用核心默认），阈值与保留百分比来自全局设置。
    /// 约束：阈值 0% = 关自动压缩。见 docs/session/session-model.md 六。
    pub(crate) fn context_budget(&self, model: Option<&str>) -> (u64, u8, u8) {
        // 窗口是登记处的事实，不在这里另存一份。
        (
            self.registry.context_of(model),
            self.registry.app().compact_at_percent,
            self.registry.app().compact_keep_percent,
        )
    }

    /// 目的：把当前设置与模型窗口刷到一条活会话（设置改了不用重建会话）。
    /// 参数：`s` = 单 agent 会话；窗口按它自己的模型解析。
    pub(crate) fn refresh_context_budget(
        &self,
        s: &mut crate::capabilities::session::api::AgentSession,
    ) {
        let model = s.model_id().map(str::to_string);
        let (window, percent, keep) = self.context_budget(model.as_deref());
        s.set_context_budget(window, percent, keep);
    }

    /// 装配单 agent 会话（不插入会话中心；插入与落盘由 create_work 统一做）。
    // 会话装配把这一场单 agent 会话一次说全；收口成参数对象只是把参数挪个地方。
    // 这是有意的设计取舍（见 docs/testing/quality-isolation.md 的 allow 清单），不是没修。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_single(
        &self,
        a: &AgentMeta,
        modules: &[Module],
        channel: Channel,
        sb: &crate::capabilities::workspace::api::Sandbox,
        unavailable: BTreeMap<String, Vec<String>>,
        net: bool,
        tier: crate::kernel::api::Tier,
        guest: Option<crate::capabilities::vm::api::GuestOs>,
        guest_lease: String,
        host_plan: &pkgs::PackagePlan,
    ) -> (
        crate::capabilities::session::api::AgentSession,
        Vec<SessionEvent>,
    ) {
        // 形态按登记处解析；通道已在前置判据里解析成功，这里不再有"没有通道"的分支。
        let spec = crate::capabilities::turn::api::SingleSpec {
            agent: a,
            member: crate::capabilities::turn::api::MemberSpec {
                modules,
                sandbox: sb,
                unavailable,
                net,
                mode: self.registry.tool_mode(a.model.as_deref()),
                // 单 agent 工作：用户自己开的那场对话（不是任务链的节点）——身份是 solo。
                role: "solo",
                tier,
                guest_lease,
                plan: host_plan,
                base_roots: &self.base_roots,
                host_read_roots: crate::capabilities::env::api::host_read_roots(
                    self.registry.app(),
                ),
                shell: self.turn.shell_slot(&sb.session),
            },
            channel,
            guest,
            context: self.context_budget(a.model.as_deref()),
            core_model: self.registry.core_model(),
        };
        crate::capabilities::turn::api::single_session(&self.turn.session_deps(), &spec)
    }
}
