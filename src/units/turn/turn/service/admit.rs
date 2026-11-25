//! 目的：turn 侧的会话准入——会话进在世表之前的落盘、按需重建、派发闸与子树续跑。
//! 管：`admit`（落盘并进在世表）、`ensure_session`（不在世就重建再放入）、`dispatch_gate`（终态会话拒绝派发）、`resume_subtree`（把停掉的子树拉回运行态）。
//! 不管：重建本体（见 `service/rebuild.rs`）、生成驱动与工作编排（见 `runtime`）。
//! 联动：`runtime` 的启用、派发与协作推进都经这里；`ensure_session` 重建时按 `runtime` 传来的登记处与主机探针取事实。

use crate::kernel::ports::HostProbe;
use crate::units::catalog::registry::api::Registry;

use crate::units::conversation::session::api::{subtree_of, truncate_events};
use crate::units::session_model::api::{RunState, SessionMeta};
use crate::units::turn::turn::api::Session;
use crate::units::turn::turn::service::session::Turn;

impl Turn {
    /// 目的：把一条已建好的会话说进在世表并落盘（建会话的尾巴只走这一条，免得两处各写一遍顺序）。
    /// 参数：`meta` = 会话身份；`session` = 造好的会话本体。
    /// 错误：落盘失败时原样报错（会话已在世，由调用方决定是否收摊）。
    pub fn admit(&mut self, meta: &SessionMeta, session: Session) -> Result<(), String> {
        self.unmark_starting(&meta.name);
        self.history.create(meta)?;
        self.state.put(meta.name.clone(), session);
        Ok(())
    }

    /// 目的：要一条会话的活体时按需重建——已在世直接过，生成中就地拒绝，否则从落盘重建后进在世表。
    /// 参数：`sid` = 会话名；`registry` / `probe` = 重建时要的登记处与主机探针。
    /// 错误：读落盘失败或重建失败时原样报错。
    pub fn ensure_session(
        &mut self,
        sid: &str,
        registry: &dyn Registry,
        probe: &dyn HostProbe,
    ) -> Result<(), String> {
        if self.holds(sid) {
            return Ok(());
        }
        if self.is_running(sid) {
            return Err(Self::running_refusal(sid));
        }
        let (meta, events) = self.history.load(sid)?;
        let rebuilt = self.rebuild_session(&meta, &truncate_events(&events), registry, probe)?;
        self.put(sid.to_string(), rebuilt);
        Ok(())
    }

    /// 目的：派发前的一道闸——终态会话不许再派活（拒绝理由直接告诉用户该做什么）。
    /// 参数：`sid` = 会话名。
    /// 错误：会话处于终态时返回拒绝文案；读不到身份则不拦。
    pub fn dispatch_gate(&self, sid: &str) -> Result<(), String> {
        match self.history.meta(sid) {
            Ok(meta) => match meta.dispatch_refusal() {
                Some(e) => Err(e),
                None => Ok(()),
            },
            Err(_) => Ok(()),
        }
    }

    /// 目的：把一棵子树里「用户停过」的会话拉回运行态（继续 = 重新授权派发）。
    /// 参数：`name` = 子树根的会话名。
    /// 返回：是否真的改过谁的状态（没有停掉的 = `false`）。
    /// 错误：子树里有已归档的会话时原样报错（要先「恢复归档」）。
    pub fn resume_subtree(&self, name: &str) -> Result<bool, String> {
        let mut any = false;
        for sid in subtree_of(self.history.as_ref(), name) {
            let mut meta = self.history.meta(&sid)?;
            match meta.run {
                RunState::Archived => {
                    return Err(format!("会话 {} 已归档：先「恢复归档」再继续", sid));
                }
                RunState::Stopped => {
                    meta.run = RunState::Active;
                    self.history.save_meta(&meta)?;
                    any = true;
                }
                RunState::Active => {}
            }
        }
        Ok(any)
    }
}
