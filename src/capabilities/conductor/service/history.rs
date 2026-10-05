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
            let work = self.work_root(sid).unwrap_or_else(|_| m.name.clone());
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
    pub(crate) fn subtree_of(&self, name: &str) -> Vec<String> {
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

    /// 这个会话的**使用类型**（机制册的第一把钥匙）：按落盘形态派生，重建后是同一份。
    /// 协作派生的成员 / 节点会话算 **collab**（它属于那场协作），其余单 agent 会话算 single，代理算 proxy。
    pub(crate) fn session_kind_of(
        &self,
        meta: &crate::capabilities::session::api::SessionMeta,
    ) -> Result<String, String> {
        match meta.mode.as_str() {
            "proxy" => Ok("proxy".to_string()),
            "collab" => Ok("collab".to_string()),
            _ => match meta.parent.as_deref() {
                Some(p) => match self.history.meta(p) {
                    Ok(pm) if pm.mode == "collab" => Ok("collab".to_string()),
                    _ => Ok("single".to_string()),
                },
                None => Ok("single".to_string()),
            },
        }
    }

    /// 这一席的**角色**（机制册的第二把钥匙，也是工具面与提示词的角色名）。
    /// 与实时建立同一把尺子：协作的成员 / 节点 = executor；代理 = core_proxy；其余单 agent 工作 = solo。
    pub(crate) fn role_of(
        &self,
        meta: &crate::capabilities::session::api::SessionMeta,
    ) -> Result<String, String> {
        if meta.mode == "proxy" {
            return Ok("core_proxy".to_string());
        }
        match meta.parent.as_deref() {
            Some(p) => match self.history.meta(p) {
                Ok(pm) if pm.mode == "collab" => Ok("executor".to_string()),
                _ => Ok("solo".to_string()),
            },
            None => Ok("solo".to_string()),
        }
    }

    /// 一个会话的**工作根**，从它自己的 meta 起算：`meta` 可能**还没落盘**
    /// （建工作区发生在落盘之前），所以有父就上溯、没有父就是它自己。
    pub(crate) fn work_root_of(&self, meta: &SessionMeta) -> Result<String, String> {
        match &meta.parent {
            None => Ok(meta.name.clone()),
            Some(p) => self.work_root(p),
        }
    }

    /// 把整棵子树（含自己）的运行态落成同一个值：**停止**先整棵冻上，**继续**再整棵解开。
    /// 停止时顺序不能反——先冻态再取消生成，被停会话收尾写的那条"这一轮结束"才会被闸门拒掉。
    pub(crate) fn set_subtree_run(
        &self,
        name: &str,
        run: crate::capabilities::session::api::RunState,
    ) -> Result<Vec<String>, String> {
        let tree = self.subtree_of(name);
        for sid in &tree {
            let mut meta = self.history.meta(sid)?;
            if meta.run != run {
                meta.run = run;
                self.history.save_meta(&meta)?;
            }
        }
        Ok(tree)
    }

    /// 解除整棵子树的「已停止」（停止的逆操作）；返回是否确实解除了至少一个。
    /// 子树里只要有**已关闭**的就整条拒绝——关闭是终态，不能被「继续」拉回来。
    pub(crate) fn resume_subtree(&self, name: &str) -> Result<bool, String> {
        use crate::capabilities::session::api::RunState;
        let mut any = false;
        for sid in self.subtree_of(name) {
            let mut meta = self.history.meta(&sid)?;
            match meta.run {
                RunState::Closed => {
                    return Err(format!("会话 {} 已关闭：终态，不能再继续", sid));
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

    /// 一个会话的**工作根**（顶层会话名）：沿 `meta.parent` 一路走到顶。
    /// 整棵树不论嵌套多少层只有一个 `work/`——共享区与 agent 沙箱都锚在它上面。
    pub(crate) fn work_root(&self, name: &str) -> Result<String, String> {
        let mut cur = self.history.meta(name)?;
        let mut guard = 0usize;
        while let Some(p) = cur.parent.clone() {
            guard += 1;
            if guard > 64 {
                return Err(format!("会话父子关系成环：{}", name));
            }
            cur = self.history.meta(&p)?;
        }
        Ok(cur.name)
    }

    /// **运行态闸门**：派发 / 唤醒前过它，停止与关闭都拒绝。
    /// 拿不到 meta（系统会话等未落盘）就当正常运行——运行态是落盘事实，
    /// 不存在的会话自有别的检查兜底（这里不替它报"无此会话"）。
    pub(crate) fn dispatch_gate(&self, sid: &str) -> Result<(), String> {
        match self.history.meta(sid) {
            Ok(meta) => match meta.dispatch_refusal() {
                Some(e) => Err(e),
                None => Ok(()),
            },
            Err(_) => Ok(()),
        }
    }

    /// 派发目标的形态 + 这一次派发放不放行（放行 = Ok(mode)）。
    /// 代理转达与叫醒共用这一处，免得"看不看运行态"两处口径不一致。
    pub(crate) fn dispatch_target(&self, sid: &str) -> Result<String, String> {
        let meta = self.history.meta(sid)?;
        match meta.dispatch_refusal() {
            Some(e) => Err(e),
            None => Ok(meta.mode),
        }
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
