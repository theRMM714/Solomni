//! 目的：工作区的读取用例——状态、日志、提交详情与文件版本。
//! 管：work_status、work_log、work_commit_view、work_commit_file、work_versions。
//! 不管：写操作（commit / sync / lifecycle）；回执文案由 api 定义。
//! 联动：service.rs 的底座与 domain::workstore 的 status_of 与树回溯。

use super::*;
use crate::capabilities::workspace::api::{
    ChangeKind, ChangeView, CommitDetail, CommitFileView, CommitSummary, StatusEntry, StatusReport,
    VERSION_TEXT_LIMIT, VersionView, WorkLog, WorkLogEntry, media_hint,
};
use crate::capabilities::workspace::domain::workstore::work_state_of;

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
        let (work_id, work_tree) = self.work_tree(&store, agent)?;
        let work_head = match work_id {
            Some(id) => self
                .store
                .read_commit(&store, id)?
                .map(|c| CommitSummary::from(&c)),
            None => None,
        };
        let index: Index = self.store.read_index(&store, agent)?;
        let sandbox_files: BTreeSet<String> =
            self.sandbox_files(&store, &sandbox)?.into_iter().collect();
        // 候选路径：拉取基线、我的工作线、主线与沙箱的并集；状态只看「工作线 / 主线 / 沙箱」三方。
        let all: BTreeSet<String> = index
            .keys()
            .chain(work_tree.keys())
            .chain(tree.keys())
            .chain(sandbox_files.iter())
            .cloned()
            .collect();
        // 显式给路径时，把沙箱里未跟踪的文件也算进候选（能如实报“本地新增”）。
        let available: BTreeSet<String> = all.iter().chain(sandbox_files.iter()).cloned().collect();
        let (want, missing): (Vec<String>, Vec<String>) = if paths.is_empty() {
            (all.iter().cloned().collect(), Vec::new())
        } else {
            self.expand_forced(&sandbox, &available, paths)?
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
            let state = work_state_of(
                work_tree.get(&rel).map(String::as_str),
                tree.get(&rel).map(String::as_str),
                local_hash.as_deref(),
            );
            entries.push(StatusEntry { path: rel, state });
        }
        // 主副本路径清单：截断如实标注。
        let mut all_tree: Vec<String> = tree.keys().cloned().collect();
        let truncated = all_tree.len() > MAX_STATUS_TREE;
        all_tree.truncate(MAX_STATUS_TREE);
        // 待推送判定与 work_push 共用 plan_push：净改动 / 冲突 / 领先提交数只有一个口径。
        let push = self.plan_push(&store, agent)?;
        let unpushed = push.unpushed;
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
            work_head,
            unpushed,
            pending: push.pending,
            conflicts: push.conflicts.len(),
            tree: all_tree,
            tree_truncated: truncated,
            entries,
            missing,
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
        // 只列主线提交：工作线提交与存档快照是各自的本地历史，进不了共享区历史。
        let mut main_ids: Vec<u64> = Vec::new();
        // 已被推送的工作线提交：squash 之后原提交不再是主线祖先，只能靠主线提交记下的来源判。
        let mut published: BTreeSet<u64> = BTreeSet::new();
        for id in self.store.list_commits(&store)? {
            if let Some(c) = self.store.read_commit(&store, id)?
                && c.line == CommitLine::Main
            {
                published.extend(c.sources.iter().copied());
                main_ids.push(id);
            }
        }
        let total = main_ids.len();
        // 各 agent 工作线的祖先集：工作线提交散布在各自的线上，on_head 只能按自己那条线判。
        let mut lines: BTreeMap<String, BTreeSet<u64>> = BTreeMap::new();
        for (name, r) in self.store.list_refs(&store)? {
            let Some(agent) = name.strip_prefix("agents/") else {
                continue;
            };
            if agent.contains('/') {
                continue;
            }
            lines.insert(agent.to_string(), self.head_ancestors(&store, r.target)?);
        }
        // 工作线本地提交：各 agent 各一条线，放在同一页里按 id 新→旧，pushed 标是否已进主线。
        let mut work: Vec<WorkLogEntry> = Vec::new();
        for id in self.store.list_commits(&store)?.into_iter().rev() {
            let Some(c) = self.store.read_commit(&store, id)? else {
                continue;
            };
            if c.line != CommitLine::Work {
                continue;
            }
            let on_line = lines.get(&c.author).is_some_and(|s| s.contains(&id));
            work.push(WorkLogEntry {
                id: c.id,
                parent: c.parent,
                author: c.author,
                session: c.session,
                time: c.time,
                message: c.message,
                changes: c.changes.len(),
                on_head: on_line,
                pushed: published.contains(&id) || on_head.contains(&id),
            });
        }
        let mut commits: Vec<WorkLogEntry> = Vec::new();
        let mut truncated = false;
        for id in main_ids.into_iter().rev() {
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
                pushed: true,
            });
        }
        Ok(WorkLog {
            head,
            commits,
            work,
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
