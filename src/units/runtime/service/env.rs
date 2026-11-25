//! 目的：guest 诊断的编排：只做「哪条会话有 guest」的判定与任务交接。
//! 管：prepare_guest_diagnose（诊断任务交给调用方线程跑）。
//! 不管：诊断本身（`vm` 的 `GuestDiagnose`）；会话事实从哪来（history）。
//! 联动：动作分发在 guest_diagnose 分支调它。

use super::*;

impl Core {
    /// 目的：一次 guest 只读诊断的准备（毫秒级）：定位这条会话的 guest 租约。
    /// 约束：本机档没有 guest；代理父会话自己没有 guest，要诊断它下面的某个工作。
    pub(crate) fn prepare_guest_diagnose(
        &self,
        sid: &str,
    ) -> Result<crate::units::work::api::GuestDiagnose, String> {
        let meta = self.history.meta(sid)?;
        if meta.exec.tier != crate::units::pure::exec_model::api::Tier::Vm {
            return Err("这条会话是本机档，没有 guest 可诊断".to_string());
        }
        match crate::units::conversation::session::api::guest_scope(
            self.history.as_ref(),
            &meta,
        )? {
            Some((lease, _)) => Ok(crate::units::work::api::GuestDiagnose::new(
                Arc::clone(&self.vm),
                lease,
            )),
            None if meta.is_proxy() && meta.parent.is_none() => {
                Err("代理父会话本身没有 guest，请对它下面某个工作诊断".to_string())
            }
            None => Err("这条会话没有可诊断的 guest".to_string()),
        }
    }
}
#[cfg(test)]
impl Core {
    /// 目的：一条会话的**沙箱清单**（测试专用）：把注册处与探针的事实取成清单。
    pub(crate) fn sandboxes(
        &self,
        meta: &SessionMeta,
        roster: &crate::units::catalog::modules::api::Roster,
    ) -> Result<crate::units::workspace::api::Sandboxes, String> {
        self.turn
            .sandboxes(self.registry.as_ref(), self.probe.as_ref(), meta, roster)
    }
}
