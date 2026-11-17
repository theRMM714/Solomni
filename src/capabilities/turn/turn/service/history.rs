//! 目的：事件收编与增量落盘手柄——记录事件、派生落盘器，以及子树运行态与派发闸的口径。
//! 管：什么时候写、写什么（落盘机制在 session 的 HistoryStore 端口后面）；停止 / 继续时整棵子树的运行态。
//! 不管：历史列出 / 打开 / 删除与回档（在 lifecycle）；会话对象怎么建（在 assemble 与 rebuild）。
//! 联动：回合推进的所有方法族的事件都经 `record_events` 落盘（见 docs/session/session-model.md）。

use crate::capabilities::conversation::session::api::{
    Persister, RunState, SessionEvent, subtree_of,
};

use super::session::Turn;

impl Turn {
    /// 目的：把整棵子树（含自己）的运行态落成同一个值：**停止**先整棵冻上，**继续**再整棵解开。
    ///  停止时顺序不能反——先冻态再取消生成，被停会话收尾写的那条"这一轮结束"才会被闸门拒掉。
    pub(crate) fn set_subtree_run(&self, name: &str, run: RunState) -> Result<Vec<String>, String> {
        let tree = subtree_of(self.history.as_ref(), name);
        for sid in &tree {
            let mut meta = self.history.meta(sid)?;
            if meta.run != run {
                meta.run = run;
                self.history.save_meta(&meta)?;
            }
        }
        Ok(tree)
    }

    /// 目的：派发目标的形态 + 这一次派发放不放行（放行 = Ok(mode)）。
    ///  代理转达与叫醒共用这一处，免得"看不看运行态"两处口径不一致。
    pub(crate) fn dispatch_target(&self, sid: &str) -> Result<String, String> {
        let meta = self.history.meta(sid)?;
        match meta.dispatch_refusal() {
            Some(e) => Err(e),
            None => Ok(meta.mode),
        }
    }

    /// 目的：事件落盘；失败如实告知（追加一条警告事件），不静默丢历史。
    pub(crate) fn record_events(&self, sid: &str, events: &mut Vec<SessionEvent>) {
        crate::capabilities::conversation::session::api::record_events(
            self.history.as_ref(),
            self.log.as_ref(),
            sid,
            events,
        );
    }

    /// 目的：把一条内容类系统提示记成**转录行**（有 id、可回档/折叠）；返回要外送的事件。
    /// 约束：只在会话对象在核心表里时分配行 id；生成中的会话被取走、没有计数器可分配，
    ///   这时如实退回瞬时提示（界面照样看得见，只是不可回档）。
    pub(crate) fn notice_row(&mut self, sid: &str, text: &str) -> Vec<SessionEvent> {
        let mut out = Vec::new();
        match self.session_mut(sid) {
            Some(crate::capabilities::turn::turn::api::Session::Single(s)) => {
                out.extend(s.note_notice(text))
            }
            Some(crate::capabilities::turn::turn::api::Session::Collab(c)) => {
                c.notice(text, &mut |e| out.push(e))
            }
            None => out.push(SessionEvent::Notice(text.to_string())),
        }
        out
    }

    /// 目的：增量落盘手柄：交给工作线程，按"一次模型调用"的粒度落盘（见 `Persister`）。
    pub(crate) fn persister(&self, sid: &str) -> Persister {
        Persister::new(
            std::sync::Arc::clone(&self.history),
            std::sync::Arc::clone(&self.log),
            sid,
        )
    }
}
