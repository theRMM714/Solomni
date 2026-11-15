//! 目的：核心侧的少量事实取用——一次 guest 只读诊断的准备、讨论提醒次数上限。
//! 管：从 history / registry 取事实后转调 `env` 与 `work` 能力。
//! 不管：会话装配要的沙箱 / 权限 / 预算 / 树级 meta（归 `turn` 的会话本体）、会话怎么落盘（history.rs）、
//!   协作链怎么推进（flow.rs）、建工作的两段（work/create.rs）。
//! 联动：诊断与讨论提醒经这里；装配类事实一律走 `self.turn.*`。
use super::*;

impl Conductor {
    /// 目的：一次 guest 只读诊断的准备（毫秒级）：定位这条会话的 guest 租约。
    /// 约束：本机档没有 guest；代理父会话自己没有 guest，要诊断它下面的某个工作。
    pub(crate) fn prepare_guest_diagnose(
        &self,
        sid: &str,
    ) -> Result<crate::capabilities::work::api::GuestDiagnose, String> {
        let meta = self.history.meta(sid)?;
        if meta.exec.tier != crate::kernel::api::Tier::Vm {
            return Err("这条会话是本机档，没有 guest 可诊断".to_string());
        }
        match self.turn.guest_scope(&meta)? {
            Some((lease, _)) => Ok(crate::capabilities::work::api::GuestDiagnose::new(
                Arc::clone(&self.vm),
                lease,
            )),
            None if meta.mode == "proxy" && meta.parent.is_none() => {
                Err("代理父会话本身没有 guest，请对它下面某个工作诊断".to_string())
            }
            None => Err("这条会话没有可诊断的 guest".to_string()),
        }
    }

    /// 目的：一轮内对同一个成员最多提醒几次（用户可设，见 session-model.md 二）。
    pub fn discuss_remind_cap(&self) -> u32 {
        self.registry.app().discuss_remind_cap
    }
}
