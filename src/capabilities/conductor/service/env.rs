//! 目的：conductor 侧的会话装配入口与沙箱 / 权限事实取用——工具环境与单 agent 会话对象只在这里转调 turn。
//! 管：guest 诊断准备，以及沙箱 / 权限 / 工作区授权 / 工具面 / 上下文预算五个薄入口（事实都在 `turn::service::facts`）。
//! 不管：装配本体（在 `turn` 的 `service/assemble.rs`）、会话怎么落盘（history.rs）、协作链怎么推进（flow.rs）、建工作的两段（work/create.rs）。
//! 联动：建工作、协作推进与回放经这里取沙箱与权限；`build_single` 与 `refresh_context_budget` 也在这里。
use super::*;
use crate::capabilities::packages::api as pkgs;

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
    /// 约束：取数与逐席规则都在 `turn::service::facts`；这里只把端口拆成实参。
    pub(crate) fn sandboxes(
        &self,
        meta: &SessionMeta,
        roster: &crate::capabilities::modules::api::Roster,
    ) -> Result<crate::capabilities::workspace::api::Sandboxes, String> {
        self.turn
            .sandboxes(self.registry.as_ref(), self.probe.as_ref(), meta, roster)
    }

    /// 目的：一条会话的**生效权限**：全局默认 → 根 ceiling → 每层 narrowing → 全局硬黑名单。
    pub(crate) fn permissions_for(
        &self,
        meta: &SessionMeta,
    ) -> Result<crate::capabilities::permission::api::PermissionSet, String> {
        self.turn.permissions_for(self.registry.as_ref(), meta)
    }

    /// 目的：一轮内对同一个成员最多提醒几次（用户可设，见 session-model.md 二）。
    pub fn discuss_remind_cap(&self) -> u32 {
        self.registry.app().discuss_remind_cap
    }

    /// 目的：本会话的上下文预算：窗口按模型现算（缺省用核心默认），阈值与保留百分比来自全局设置。
    /// 约束：阈值 0% = 关自动压缩。见 docs/session/session-model.md 六。
    pub(crate) fn context_budget(&self, model: Option<&str>) -> (u64, u8, u8) {
        self.turn.context_budget(self.registry.as_ref(), model)
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
