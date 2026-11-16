//! 目的：conductor 侧的会话装配入口与沙箱 / 权限事实取用——工具环境与会话预算只在这里转调 turn。
//! 管：guest 诊断准备，以及沙箱 / 权限 / 工作区授权 / 工具面 / 上下文预算五个薄入口（事实都在 `turn::service::facts`）。
//! 不管：装配本体（在 `turn` 的 `service/assemble.rs`）、会话怎么落盘（history.rs）、协作链怎么推进（flow.rs）、建工作的两段（work/create.rs）。
//! 联动：建工作与回放经这里取沙箱与权限；回合推进的预算刷新在 `turn`。
use super::*;
#[cfg(test)]
use crate::capabilities::session::api::SessionMeta;

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
        match crate::capabilities::session::api::guest_scope(self.history.as_ref(), &meta)? {
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

    /// 目的：组装本次工作的沙箱清单：工作根来自 Workspace 端口，模块目录来自清单。
    /// 约束：取数与逐席规则都在 `turn::service::facts`；这里只把端口拆成实参（测试访问器）。
    #[cfg(test)]
    pub(crate) fn sandboxes(
        &self,
        meta: &SessionMeta,
        roster: &crate::capabilities::modules::api::Roster,
    ) -> Result<crate::capabilities::workspace::api::Sandboxes, String> {
        self.turn
            .sandboxes(self.registry.as_ref(), self.probe.as_ref(), meta, roster)
    }

    /// 目的：一条会话的**生效权限**：全局默认 → 根 ceiling → 每层 narrowing → 全局硬黑名单。
    #[cfg(test)]
    pub(crate) fn permissions_for(
        &self,
        meta: &SessionMeta,
    ) -> Result<crate::capabilities::permission::api::PermissionSet, String> {
        self.turn.permissions_for(self.registry.as_ref(), meta)
    }
}
