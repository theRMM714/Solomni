//! 目的：`work_diff` 用例——两侧取树与对象、逐路径判定变更类型，纯算法在 `domain/workstore/diff.rs`。
//! 管：从工作线 / 沙箱 / 任意提交取树，读内容对象，产出 `DiffReport`。
//! 不管：算法（`diff_text`）与文案（`domain/work_tools.rs`）；权限（`service/work_tools.rs`）。
//! 联动：`work_diff` 工具执行者与 `Workspace` trait。

use super::*;
use crate::units::business::versions::domain::workstore::{
    DIFF_MAX_BYTES, DiffEntry, DiffReport, DiffRequest, diff_text,
};

impl VersionService {
    /// 目的：比较两个版本（缺省：工作线头 ↔ 沙箱）的逐路径差异。
    /// 约束：二进制与超过 `DIFF_MAX_BYTES` 的文件不展开 hunk，只报类型与大小。
    pub(super) fn work_diff(
        &self,
        work: &str,
        agent: &str,
        req: &DiffRequest,
    ) -> Result<DiffReport, String> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let _shared = self.shared_guard(work)?;
        let (shared, store, sandbox) = self.agent_roots(work, agent)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;

        let (head, head_tree) = self.work_tree(&store, agent)?;
        let (from_tree, from_label) = match req.from {
            Some(id) => (
                Self::tree_of(self.store.as_ref(), &store, id)?,
                format!("提交 {}", id),
            ),
            None => (
                head_tree,
                match head {
                    Some(id) => format!("工作线 {}", id),
                    None => "工作线（还没有提交）".to_string(),
                },
            ),
        };
        let (to_tree, to_label) = match req.to {
            Some(id) => (
                Self::tree_of(self.store.as_ref(), &store, id)?,
                format!("提交 {}", id),
            ),
            None => (self.scan_tree(&sandbox)?, "沙箱".to_string()),
        };

        let filter: BTreeSet<String> = req.paths.iter().cloned().collect();
        let mut entries = Vec::new();
        let mut keys: BTreeSet<&String> = from_tree.keys().collect();
        keys.extend(to_tree.keys());
        for path in keys {
            if !filter.is_empty() && !filter.contains(path) {
                continue;
            }
            let a = from_tree.get(path);
            let b = to_tree.get(path);
            if a == b {
                continue;
            }
            let kind = match (a, b) {
                (None, Some(_)) => ChangeKind::Add,
                (Some(_), None) => ChangeKind::Delete,
                _ => ChangeKind::Modify,
            };
            let mut entry = DiffEntry {
                path: path.clone(),
                kind,
                binary: false,
                too_large: false,
                hunks: Vec::new(),
            };
            let old = match a {
                Some(h) => self.object_text(&store, h, &mut entry)?,
                None => Some(String::new()),
            };
            let new = if req.to.is_some() {
                match b {
                    Some(h) => self.object_text(&store, h, &mut entry)?,
                    None => Some(String::new()),
                }
            } else {
                // 目标侧是沙箱：内容可能还没被任何提交收进对象库，直接读文件。
                match b {
                    Some(_) => self.sandbox_text(&sandbox, path, &mut entry)?,
                    None => Some(String::new()),
                }
            };
            if !entry.binary
                && !entry.too_large
                && let (Some(old), Some(new)) = (old, new)
            {
                entry.hunks = diff_text(&old, &new);
            }
            entries.push(entry);
        }
        Ok(DiffReport {
            from: from_label,
            to: to_label,
            entries,
            truncated: false,
        })
    }

    /// 目的：沙箱当前内容的树（路径 → 指纹）。
    fn scan_tree(&self, root: &Path) -> Result<Tree, String> {
        let mut tree = Tree::new();
        for rel in self.store.list(root)? {
            let Some(f) = self.store.scan_under(root, &rel, None)? else {
                continue;
            };
            tree.insert(rel, f.hash);
        }
        Ok(tree)
    }

    /// 目的：读一个内容对象并给出文本（`None` = 二进制或过大，`entry` 上记原因）。
    fn object_text(
        &self,
        store: &Path,
        hash: &str,
        entry: &mut DiffEntry,
    ) -> Result<Option<String>, String> {
        let size = self.store.stat_object(store, hash)?;
        if size > DIFF_MAX_BYTES {
            entry.too_large = true;
            return Ok(None);
        }
        let bytes = self.store.read_object(store, hash)?;
        Ok(decode(bytes, entry))
    }

    /// 目的：读沙箱里的一份文件并给出文本（沙箱内容可能还没进对象库）。
    fn sandbox_text(
        &self,
        sandbox: &Path,
        rel: &str,
        entry: &mut DiffEntry,
    ) -> Result<Option<String>, String> {
        let Some(f) = self
            .store
            .scan_under(sandbox, rel, Some(DIFF_MAX_BYTES as usize))?
        else {
            return Ok(Some(String::new()));
        };
        if f.truncated || f.too_large {
            entry.too_large = true;
            return Ok(None);
        }
        match f.text {
            Some(t) => Ok(Some(t)),
            None => {
                entry.binary = true;
                Ok(None)
            }
        }
    }
}

/// 目的：把字节解成文本；二进制或超过单文件上限时在 `entry` 上记原因并返回 `None`。
fn decode(bytes: Vec<u8>, entry: &mut DiffEntry) -> Option<String> {
    if bytes.len() as u64 > DIFF_MAX_BYTES {
        entry.too_large = true;
        return None;
    }
    if bytes.contains(&0) {
        entry.binary = true;
        return None;
    }
    match String::from_utf8(bytes) {
        Ok(t) => Some(t),
        Err(_) => {
            entry.binary = true;
            None
        }
    }
}
