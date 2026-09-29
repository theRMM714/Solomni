//! **历史与落盘**：历史列出 / 打开 / 删除、事件收编（record_events）与增量落盘手柄。
//!
//! 落盘机制在 session 的 HistoryStore 端口后面，本文件只说"什么时候写、写什么"。
//!
//! 它是 Conductor 的一个方法族：与 mod.rs 同在 service 模块下（子模块看得见父模块的私有字段），
//! 方法取 pub(crate)（或 pub）供兄弟族与 conductor/api.rs 调用。

use super::*;

impl Conductor {
    /// 历史列表：**树序**（每个父会话紧跟它的子会话）——侧栏据此缩进，
    /// 顺序与缩进同源，不会再出现"子会话排到父会话上面"的错位（见 session-model.md 一）。
    /// 读盘失败如实记日志，返回空表。
    pub fn history_list(&self) -> Vec<HistoryView> {
        match self.history.list() {
            Ok(v) => tree_order(v),
            Err(e) => {
                self.log
                    .error("conductor::history_list", &format!("会话列表失败：{}", e));
                Vec::new()
            }
        }
    }

    /// 打开历史会话：返回元信息与事件流（只读回放；是否续跑由用户点「继续」授权）。
    /// 回放同样应用 rewind 截断——流水保留审计，会话内容以截断后为准。
    pub fn history_open(
        &self,
        name: &str,
    ) -> Result<(SessionMeta, Vec<serde_json::Value>), String> {
        let (meta, events) = self.history.load(name)?;
        Ok((
            meta,
            crate::capabilities::session::api::truncate_events(&events),
        ))
    }

    /// 删除会话（= 删目录）。内存中的同名会话一并移除，避免内存与磁盘不一致。
    /// 删之前先请适配层撤销该会话各 agent 的围栏授权：痕迹与会话同生共死，不随会话数量堆积。
    pub fn history_delete(&mut self, name: &str) -> Result<bool, String> {
        if self.running.contains(name) {
            return Err(Self::running_refusal(name));
        }
        if let Ok((meta, _)) = self.history.load(name) {
            let roster = self.workspace.roster();
            match self.sandboxes(&meta, &roster) {
                Ok(sandboxes) => {
                    for sb in &sandboxes.list {
                        // 撤销要覆盖同一次授权写下的全部条目：读写根 + 用户授权的只读根。
                        let spec = crate::capabilities::tools::api::FenceSpec::from_sandbox(
                            sb,
                            meta.exec.net,
                        )
                        .with_read_only(self.fence_read_roots());
                        if let Err(e) = self.tools.release_fence(&spec) {
                            self.log.warn(
                                "conductor::history_delete",
                                &format!("撤销围栏授权未完成：{}", e),
                            );
                        }
                    }
                }
                Err(e) => self.log.warn(
                    "conductor::history_delete",
                    &format!("取沙箱失败，未撤销授权：{}", e),
                ),
            }
        }
        self.sessions.remove(name);
        self.history.delete(name)
    }

    /// 事件落盘；失败如实告知（追加一条警告事件），不静默丢历史。
    pub(crate) fn record_events(&self, sid: &str, events: &mut Vec<SessionEvent>) {
        if events.is_empty() {
            return;
        }
        if let Some(warn) = persist_events(self.history.as_ref(), self.log.as_ref(), sid, events) {
            events.push(SessionEvent::Notice(warn));
        }
    }

    /// 增量落盘手柄：交给工作线程，按"一次模型调用"的粒度落盘（见 `Persister`）。
    pub(crate) fn persister(&self, sid: &str) -> Persister {
        Persister {
            history: Arc::clone(&self.history),
            log: Arc::clone(&self.log),
            sid: sid.to_string(),
        }
    }
}
