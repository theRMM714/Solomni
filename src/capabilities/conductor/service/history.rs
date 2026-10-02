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

    /// 删除会话（含它的整棵子树）。
    ///
    /// 口径（见 docs/session/session-model.md 七）：
    /// - **子会话不允许单独删**：它由核心按节点派生，删父会话会一起删掉；单独删会让父子关系断掉。
    /// - **删父带子**：按 `meta.parent` 收集整棵子树，**先子后父**逐个撤围栏授权 → 移出内存 → 删目录。
    /// - **子树里任一节点在生成中就整体拒绝**：不能删到一半留下半个状态。
    ///
    /// 内部的 `session::History::delete`（纯存储）不受影响——系统路径还需要它。
    pub fn history_delete(&mut self, name: &str) -> Result<bool, String> {
        // 不是落盘会话（或已不存在）：只清内存，交存储层如实回 false。
        let Ok((meta, _)) = self.history.load(name) else {
            self.sessions.remove(name);
            return self.history.delete(name);
        };
        // 子会话由核心管理，不允许用户单独删。
        if let Some(parent) = meta.parent.clone() {
            return Err(format!(
                "会话 {} 是核心管理的子会话（父会话 {}）：删父会话会一起删掉它，不能单独删",
                name, parent
            ));
        }
        let subtree = self.subtree_of(name);
        for sid in &subtree {
            if self.running.contains(sid) {
                return Err(Self::running_refusal(sid));
            }
        }
        // 撤围栏授权：子会话与父会话共用工作区，按工作根去重，避免对同一套根重复撤销。
        let roster = self.workspace.roster();
        let mut released: Vec<String> = Vec::new();
        for sid in &subtree {
            let Ok((m, _)) = self.history.load(sid) else {
                continue;
            };
            let work = m.work().to_string();
            if released.iter().any(|w| w == &work) {
                continue;
            }
            released.push(work);
            match self.sandboxes(&m, &roster) {
                Ok(sandboxes) => {
                    for sb in &sandboxes.list {
                        // 撤销要覆盖同一次授权写下的全部条目：读写根 + 用户授权的只读根。
                        let spec = crate::capabilities::tools::api::FenceSpec::from_sandbox(
                            sb, m.exec.net,
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
        for sid in &subtree {
            self.sessions.remove(sid);
        }
        // 后序删除：先子后父。父会话目录被删时，落在它内部的子会话目录一起消失。
        let mut target_deleted = false;
        for sid in subtree.iter().rev() {
            let ok = self.history.delete(sid)?;
            if sid == name {
                target_deleted = ok;
            }
        }
        Ok(target_deleted)
    }

    /// 一个会话的整棵子树（含它自己，父在前、子在后）：父子关系以 `meta.parent` 为唯一判据。
    fn subtree_of(&self, name: &str) -> Vec<String> {
        let list = self.history_list();
        let mut out = vec![name.to_string()];
        let mut i = 0;
        while i < out.len() {
            let cur = out[i].clone();
            for h in &list {
                if h.parent.as_deref() == Some(cur.as_str()) && !out.iter().any(|x| x == &h.name) {
                    out.push(h.name.clone());
                }
            }
            i += 1;
        }
        out
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
