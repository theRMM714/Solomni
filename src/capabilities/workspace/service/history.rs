//! 目的：工作区的读取用例——状态、日志、提交详情与文件版本。
//! 管：work_status、work_log、work_commit_view、work_commit_file、work_versions。
//! 不管：写操作（commit / sync / lifecycle）；回执文案由 api 定义。
//! 联动：service.rs 的底座与 domain::workstore 的 status_of 与树回溯。

use super::*;
use crate::capabilities::workspace::api::{
    ChangeKind, ChangeView, CommitDetail, CommitFileView, CommitSummary, StatusEntry, StatusReport,
    VERSION_TEXT_LIMIT, VersionView, WorkLog, WorkLogEntry, media_hint,
};

impl WorkspaceService {
    pub(super) fn work_status(
        &self,
        work: &str,
        agent: &str,
        paths: &[String],
    ) -> Result<StatusReport, String> {
        if !paths.is_empty() {
            validate_paths(paths)?;
        }
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (shared, store, sandbox) = self.agent_roots(work, agent)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        let (head_id, tree) = self.head_tree(&store)?;
        let head = match head_id {
            Some(id) => self
                .store
                .read_commit(&store, id)?
                .map(|c| CommitSummary::from(&c)),
            None => None,
        };
        let index: Index = self.store.read_index(&store, agent)?;
        let sandbox_files: BTreeSet<String> = self.store.list(&sandbox)?.into_iter().collect();
        let all: BTreeSet<String> = index.keys().chain(tree.keys()).cloned().collect();
        // 显式给路径时，把沙箱里未跟踪的文件也算进候选（能如实报“本地新增”）。
        let available: BTreeSet<String> = all.iter().chain(sandbox_files.iter()).cloned().collect();
        let (want, missing): (Vec<String>, Vec<String>) = if paths.is_empty() {
            (all.iter().cloned().collect(), Vec::new())
        } else {
            expand_paths(paths, &available)
        };
        let external = if self.dirs.is_external(work) {
            let mut out = Vec::new();
            for rel in &want {
                let disk = self.disk_hash(&shared, rel)?;
                if disk.as_deref() != tree.get(rel).map(String::as_str) {
                    out.push(rel.clone());
                }
            }
            out
        } else {
            Vec::new()
        };
        let mut entries: Vec<StatusEntry> = Vec::new();
        for rel in want {
            let local = self.store.scan_under(&sandbox, &rel, None)?;
            let local_hash = local.as_ref().map(|v| v.hash.clone());
            let state = status_of(
                index.get(&rel).map(String::as_str),
                tree.get(&rel).map(String::as_str),
                local_hash.as_deref(),
            );
            entries.push(StatusEntry { path: rel, state });
        }
        // 沙箱里未跟踪的文件：只在不带 paths 时列出，避免把一次筛选变成整盘清单。
        let mut local_only: Vec<String> = if paths.is_empty() {
            sandbox_files
                .iter()
                .filter(|p| !index.contains_key(*p) && !tree.contains_key(*p))
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        let local_only_truncated = local_only.len() > MAX_LOCAL_ONLY;
        local_only.truncate(MAX_LOCAL_ONLY);
        // 主副本路径清单：截断如实标注。
        let mut all_tree: Vec<String> = tree.keys().cloned().collect();
        let truncated = all_tree.len() > MAX_STATUS_TREE;
        all_tree.truncate(MAX_STATUS_TREE);
        // 最近提交（新→旧，只沿当前 head 的祖先链：已回滚离开的提交不算"最近"）。
        let ancestors = self.head_ancestors(&store, head_id)?;
        let mut ids = self.store.list_commits(&store)?;
        ids.reverse();
        let mut recent = Vec::new();
        for id in ids
            .into_iter()
            .filter(|id| ancestors.contains(id))
            .take(RECENT_COMMITS)
        {
            if let Some(c) = self.store.read_commit(&store, id)? {
                recent.push(CommitSummary::from(&c));
            }
        }
        Ok(StatusReport {
            head,
            tree: all_tree,
            tree_truncated: truncated,
            entries,
            missing,
            local_only,
            local_only_truncated,
            external,
            recent,
        })
    }

    pub(super) fn work_log(
        &self,
        work: &str,
        limit: usize,
        before: Option<u64>,
    ) -> Result<WorkLog, String> {
        let limit = limit.clamp(1, MAX_LOG_LIMIT);
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        let head_id = self.store.head(&store)?;
        let head = match head_id {
            Some(id) => Some(CommitSummary::from(
                &self
                    .store
                    .read_commit(&store, id)?
                    .ok_or_else(|| format!("head 指向不存在的提交 {}", id))?,
            )),
            None => None,
        };
        let on_head = self.head_ancestors(&store, head_id)?;
        let all = self.store.list_commits(&store)?;
        let total = all.len();
        let mut commits: Vec<WorkLogEntry> = Vec::new();
        let mut truncated = false;
        for id in all.into_iter().rev() {
            if before.is_some_and(|b| id >= b) {
                continue;
            }
            if commits.len() >= limit {
                truncated = true;
                break;
            }
            let Some(c) = self.store.read_commit(&store, id)? else {
                continue;
            };
            commits.push(WorkLogEntry {
                id: c.id,
                parent: c.parent,
                author: c.author,
                session: c.session,
                time: c.time,
                message: c.message,
                changes: c.changes.len(),
                on_head: on_head.contains(&id),
            });
        }
        Ok(WorkLog {
            head,
            commits,
            total,
            truncated,
        })
    }

    pub(super) fn work_commit_view(&self, work: &str, commit: u64) -> Result<CommitDetail, String> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        let c = self
            .store
            .read_commit(&store, commit)?
            .ok_or_else(|| format!("没有提交点 {}", commit))?;
        let mut changes = Vec::new();
        let mut changes_truncated = false;
        for ch in &c.changes {
            if changes.len() >= MAX_COMMIT_CHANGES {
                changes_truncated = true;
                break;
            }
            let size = match ch.kind {
                ChangeKind::Delete => 0,
                ChangeKind::Add | ChangeKind::Modify => self.store.stat_object(&store, &ch.hash)?,
            };
            changes.push(ChangeView {
                path: ch.path.clone(),
                kind: ch.kind,
                hash: ch.hash.clone(),
                size,
                media_type: media_hint(&ch.path).map(|h| h.mime.to_string()),
            });
        }
        Ok(CommitDetail {
            id: c.id,
            parent: c.parent,
            author: c.author,
            session: c.session,
            time: c.time,
            message: c.message,
            changes,
            changes_truncated,
        })
    }

    pub(super) fn work_commit_file(
        &self,
        work: &str,
        commit: u64,
        path: &str,
    ) -> Result<CommitFileView, String> {
        let rel = clean_rel(path)?;
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        let tree = Self::tree_of(self.store.as_ref(), &store, commit)?;
        let hash = tree
            .get(&rel)
            .ok_or_else(|| format!("提交 {} 里没有路径：{}", commit, rel))?;
        let binary = media_hint(&rel).map(|h| h.binary).unwrap_or(false);
        let scanned = self.store.scan_object(
            &store,
            hash,
            if binary {
                None
            } else {
                Some(VERSION_TEXT_LIMIT)
            },
        )?;
        Ok(CommitFileView {
            path: rel.clone(),
            hash: hash.clone(),
            size: scanned.size,
            media_type: media_hint(&rel).map(|h| h.mime.to_string()),
            text: scanned.text,
            truncated: scanned.truncated,
            too_large: scanned.size > self.max_asset_bytes,
        })
    }

    pub(super) fn work_versions(
        &self,
        work: &str,
        agent: &str,
        path: &str,
    ) -> Result<VersionView, String> {
        let rel = clean_rel(path)?;
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (shared, store, sandbox) = self.agent_roots(work, agent)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        let (head_commit, tree) = self.head_tree(&store)?;
        let index: Index = self.store.read_index(&store, agent)?;
        let media_type = media_hint(&rel).map(|h| h.mime.to_string());
        let base = match index.get(&rel) {
            Some(hash) => Some(self.version_of_object(&store, hash, &rel)?),
            None => None,
        };
        let head = match tree.get(&rel) {
            Some(hash) => Some(self.version_of_object(&store, hash, &rel)?),
            None => None,
        };
        let local = self.version_of_local(&sandbox, &rel)?;
        if base.is_none() && head.is_none() && local.is_none() {
            return Err(format!("基线、主副本与沙箱都没有这个文件：{}", rel));
        }
        Ok(VersionView {
            path: rel,
            head_commit,
            media_type,
            base,
            head,
            local,
        })
    }
}
