//! 目的：`work_reset` 用例——把沙箱路径恢复成基线版本（`paths`），或把工作线移回某个提交（`to`）。
//! 管：祖先链校验、铺盘、覆盖前备份进 `lost/<agent>/@reset-<目标>/`。
//! 不管：参数解析与权限（`service/work_tools.rs`）、文案（`domain/work_tools.rs`）。
//! 联动：`copy_object_to_under` / `remove_under`（铺盘与备份）、`Workspace` trait。

use super::*;
use crate::capabilities::workspace::domain::workstore::{ResetReport, ResetRequest};

impl WorkspaceService {
    /// 目的：按请求恢复沙箱（`paths`）或移动工作线（`to`）。
    /// 约束：`to` 必须在这条工作线的祖先链上；移动只换头，提交对象一个都不删；
    ///   hard 形态只动工作线跟踪的路径（目标树 ∪ 当前头树），拉过但没提交的文件原样保留。
    pub(super) fn work_reset(
        &self,
        work: &str,
        agent: &str,
        req: &ResetRequest,
    ) -> Result<ResetReport, String> {
        if !req.paths.is_empty() && req.to.is_some() {
            return Err(
                "work_reset 一次只做一件事：给 paths = 把沙箱文件恢复成基线版本；给 to = 把工作线移回那条提交。"
                    .to_string(),
            );
        }
        if req.paths.is_empty() && req.to.is_none() {
            return Err(
                "work_reset 需要 to（把工作线移回某条提交）或 paths（把沙箱文件恢复成基线版本）。"
                    .to_string(),
            );
        }
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let _shared = self.dirs.lock_shared(work)?;
        let (shared, store, sandbox) = self.agent_roots(work, agent)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;

        let (head, head_tree) = self.work_tree(&store, agent)?;
        if let Some(target) = req.to {
            self.ensure_on_line(&store, head, target)?;
            self.set_line_head(&store, &Self::work_ref(agent), RefKind::Agent, Some(target))?;
            if req.keep_changes {
                return Ok(ResetReport {
                    before: head,
                    after: Some(target),
                    restored: Vec::new(),
                    removed: Vec::new(),
                    backed_up: 0,
                });
            }
            let tree = Self::tree_of(self.store.as_ref(), &store, target)?;
            let dir = format!("reset-{}", target);
            let mut restored = Vec::new();
            let mut removed = Vec::new();
            let mut backed = 0usize;
            for (rel, hash) in &tree {
                let current = self.store.scan_under(&sandbox, rel, None)?;
                if current.as_ref().map(|f| f.hash.as_str()) == Some(hash.as_str()) {
                    continue;
                }
                if current.is_some() {
                    backed += self.backup_one(&store, &sandbox, agent, &dir, rel)?;
                }
                self.store
                    .copy_object_to_under(&store, hash, &sandbox, rel)?;
                restored.push(rel.clone());
            }
            for rel in head_tree.keys() {
                if tree.contains_key(rel) {
                    continue;
                }
                if self.store.scan_under(&sandbox, rel, None)?.is_some() {
                    backed += self.backup_one(&store, &sandbox, agent, &dir, rel)?;
                    self.store.remove_under(&sandbox, rel)?;
                    removed.push(rel.clone());
                }
            }
            return Ok(ResetReport {
                before: head,
                after: Some(target),
                restored,
                removed,
                backed_up: backed,
            });
        }

        let baseline = self.store.read_index(&store, agent)?;
        let mut restored = Vec::new();
        let mut removed = Vec::new();
        let mut backed = 0usize;
        for raw in &req.paths {
            let rel = clean_rel(raw)?;
            let current = self.store.scan_under(&sandbox, &rel, None)?;
            let want = head_tree.get(&rel).or_else(|| baseline.get(&rel)).cloned();
            match want {
                Some(hash) => {
                    if current.as_ref().map(|f| f.hash.as_str()) == Some(hash.as_str()) {
                        continue;
                    }
                    if current.is_some() {
                        backed += self.backup_one(&store, &sandbox, agent, "reset-paths", &rel)?;
                    }
                    self.store
                        .copy_object_to_under(&store, &hash, &sandbox, &rel)?;
                    restored.push(rel);
                }
                None => {
                    if current.is_none() {
                        continue;
                    }
                    backed += self.backup_one(&store, &sandbox, agent, "reset-paths", &rel)?;
                    self.store.remove_under(&sandbox, &rel)?;
                    removed.push(rel);
                }
            }
        }
        Ok(ResetReport {
            before: head,
            after: head,
            restored,
            removed,
            backed_up: backed,
        })
    }

    /// 目的：确认 `target` 在 `head` 的祖先链上（否则移动工作线会让后来的提交丢掉来路）。
    fn ensure_on_line(&self, store: &Path, head: Option<u64>, target: u64) -> Result<(), String> {
        let mut cur = head;
        let mut seen: BTreeSet<u64> = BTreeSet::new();
        while let Some(id) = cur {
            if id == target {
                return Ok(());
            }
            if !seen.insert(id) {
                break;
            }
            cur = self.store.read_commit(store, id)?.and_then(|c| c.parent);
        }
        Err(format!(
            "只能退到自己的工作线提交上：{} 不在 {} 的祖先链里（先 work_log 看自己的线）。",
            target,
            match head {
                Some(id) => format!("工作线 {}", id),
                None => "空的工作线".to_string(),
            }
        ))
    }

    /// 目的：覆盖或删除前把一个文件复制进该 agent 的 lost/，返回备份份数（恒 1）。
    fn backup_one(
        &self,
        store: &Path,
        root: &Path,
        agent: &str,
        dir: &str,
        rel: &str,
    ) -> Result<usize, String> {
        let Some(f) = self.store.scan_under(root, rel, None)? else {
            return Ok(0);
        };
        let lost = lost_root(store, agent)?;
        let dest = backup_rel(dir, rel);
        self.store
            .copy_under_to_under(root, rel, &lost, &dest, &f.hash)?;
        Ok(1)
    }
}
