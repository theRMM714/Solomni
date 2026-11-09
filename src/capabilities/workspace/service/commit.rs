//! 目的：工作区的提交用例——把沙箱改写落成提交、落整棵树或增量、刷新 base 锚点与归档标记。
//! 管：commit_locked 与 commit / commit_auto / commit_user 三个用例、agent base 记录、归档标记的写入与修剪。
//! 不管：三方比较的纯规则（domain::workstore）、目录布局与共享区落盘（ports::WorkStore）。
//! 联动：service.rs 提供锁、工作根与树物化；sync 与 lifecycle 会调用本文件的提交与锚点方法。

use super::*;
use crate::capabilities::workspace::api::{Change, ChangeKind, CommitReport, CommitRequest};

impl WorkspaceService {
    /// 目的：提交的唯一实现（调用方已持 gate、shared、store 锁并已恢复事务）。
    /// 约束：不做基线准入——自动提交的准入由 `work_commit_auto` 在调用前做。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn commit_locked(
        &self,
        work: &str,
        shared: &Path,
        store: &Path,
        sandbox: &Path,
        agent: &str,
        session: &str,
        req: &CommitRequest,
        source: Source,
        anchor: Option<CommitAnchor>,
    ) -> Result<CommitReport, String> {
        let msg = req.message.trim();
        if msg.is_empty() {
            return Err("提交说明不能为空".to_string());
        }
        validate_paths(&req.paths)?;
        let shared = shared.to_path_buf();
        let store = store.to_path_buf();
        let sandbox = sandbox.to_path_buf();
        let (head_id, tree) = self.head_tree(&store)?;
        let index: Index = self.store.read_index(&store, agent)?;
        // 沙箱里的文件全集（paths 里的文件夹据此展开）。
        let sandbox_files: BTreeSet<String> = self.store.list(&sandbox)?.into_iter().collect();
        let (want, missing) = expand_paths(&req.paths, &sandbox_files);
        if !missing.is_empty() {
            return Err(format!(
                "沙箱里没有这些路径（要删除共享区的文件请放进 deletes）：{}",
                missing.join("、")
            ));
        }
        let mut locals: BTreeMap<String, Option<String>> = BTreeMap::new();
        let mut oversize: Vec<(String, u64)> = Vec::new();
        for rel in want {
            let size = self
                .store
                .stat_under(&sandbox, &rel)?
                .ok_or_else(|| format!("沙箱里没有：{}", rel))?;
            if size > self.max_asset_bytes {
                oversize.push((rel, size));
                continue;
            }
            let scanned = self
                .store
                .scan_under(&sandbox, &rel, None)?
                .ok_or_else(|| format!("沙箱里没有：{}", rel))?;
            locals.insert(rel, Some(scanned.hash));
        }
        if !oversize.is_empty() {
            return Err(render_oversize(&oversize, self.max_asset_bytes));
        }
        for raw in &req.deletes {
            let rel = clean_rel(raw)?;
            if locals.contains_key(&rel) {
                return Err(format!("{} 同时出现在 paths 与 deletes 里", rel));
            }
            locals.insert(rel, None);
        }
        if locals.is_empty() {
            return Err("没有可提交的改动：paths 与 deletes 都是空的".to_string());
        }
        if self.dirs.is_external(work) {
            self.reject_external_changes(&shared, &tree, &locals)?;
        }
        let plan = match plan_commit(&index, &tree, &locals) {
            Ok(p) => p,
            Err(conflicts) => return Err(render_conflicts(&conflicts)),
        };
        if plan.changes.is_empty() {
            return Err(render_unchanged(&plan.unchanged));
        }
        let changes = plan.changes;
        // 先落对象，再写事务日志；主副本与元数据由事务统一应用/恢复。
        let id = self.reserve_id(&store)?;
        for ch in &changes {
            if matches!(ch.kind, ChangeKind::Add | ChangeKind::Modify) {
                self.store.ingest_under(
                    &store,
                    &sandbox,
                    &ch.path,
                    &ch.hash,
                    self.max_asset_bytes,
                )?;
            }
        }
        let mut new_index = index;
        for ch in &changes {
            match ch.kind {
                ChangeKind::Delete => {
                    new_index.remove(&ch.path);
                }
                _ => {
                    new_index.insert(ch.path.clone(), ch.hash.clone());
                }
            }
        }
        let depth = self.next_depth(&store, head_id)?;
        let commit = Commit {
            id,
            parent: head_id,
            base: head_id.unwrap_or(0),
            author: agent.to_string(),
            session: session.to_string(),
            time: req.time,
            message: msg.to_string(),
            changes: changes.clone(),
            tree: deltas_tree(&tree, &changes, depth),
            depth,
            source,
            anchor,
        };
        let pending_index = PendingIndex {
            agent: agent.to_string(),
            index: new_index,
        };
        let pending = Pending::Commit {
            commit: Box::new(commit.clone()),
            index: Some(pending_index.clone()),
        };
        self.store.write_pending(&store, &pending)?;
        let result =
            self.apply_commit_pending(work, &shared, &store, &commit, Some(&pending_index));
        self.settle(work, &shared, &store, result)?;
        Ok(CommitReport { id, changes })
    }

    /// 目的：读该 agent 的逐路径基线（`refs/agents/<agent>`）：路径 → 拉取时的提交号。
    /// 约束：没有记录 = 空表；不伪造历史，第一次 pull / commit 自然写实。
    pub(super) fn agent_bases(
        &self,
        store: &Path,
        agent: &str,
    ) -> Result<BTreeMap<String, u64>, String> {
        let name = format!("agents/{}", agent);
        let Some(reference) = self.store.read_ref(store, &name)? else {
            return Ok(BTreeMap::new());
        };
        Ok(reference
            .heads
            .into_iter()
            .filter_map(|(path, oid)| oid.map(|oid| (path, oid)))
            .collect())
    }

    /// 目的：写回该 agent 的逐路径基线；`main` 记成这次写入时的主线头。
    pub(super) fn write_agent_bases(
        &self,
        store: &Path,
        agent: &str,
        main: u64,
        bases: &BTreeMap<String, u64>,
    ) -> Result<(), String> {
        self.store.write_ref(
            store,
            &format!("agents/{}", agent),
            &Ref {
                kind: RefKind::Agent,
                target: Some(main),
                heads: bases
                    .iter()
                    .map(|(path, oid)| (path.clone(), Some(*oid)))
                    .collect(),
            },
        )
    }

    /// 目的：提交成功后把这些路径的基线推进到新提交号：下一次自动提交才是快进。
    pub(super) fn record_committed_bases(
        &self,
        store: &Path,
        agent: &str,
        report: &CommitReport,
    ) -> Result<(), String> {
        let mut bases = self.agent_bases(store, agent)?;
        for change in &report.changes {
            match change.kind {
                ChangeKind::Delete => {
                    bases.remove(&change.path);
                }
                _ => {
                    bases.insert(change.path.clone(), report.id);
                }
            }
        }
        self.write_agent_bases(store, agent, report.id, &bases)
    }

    /// 目的：这次提交会碰到的**全部**路径（`paths` 里的文件夹展开 + `deletes`）——基线准入的对象。
    pub(super) fn commit_paths(
        &self,
        sandbox: &Path,
        req: &CommitRequest,
    ) -> Result<Vec<String>, String> {
        let files: BTreeSet<String> = self.store.list(sandbox)?.into_iter().collect();
        let (mut paths, _missing) = expand_paths(&req.paths, &files);
        for raw in &req.deletes {
            paths.push(clean_rel(raw)?);
        }
        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    /// 目的：留档点：head 祖先链上「各会话都还没越过自己保留行」的最后一个提交。
    /// 约束：只沿当前 head 的祖先链选点；没有锚的提交保守地算在回档点之前。
    pub(super) fn archive_point(
        &self,
        store: &Path,
        head: Option<u64>,
        keep_by_agent: &BTreeMap<String, u64>,
    ) -> Result<Option<u64>, String> {
        let ancestors = self.head_ancestors(store, head)?;
        let mut target = None;
        for id in self.store.list_commits(store)? {
            if !ancestors.contains(&id) {
                continue;
            }
            let Some(c) = self.store.read_commit(store, id)? else {
                continue;
            };
            let within = match &c.anchor {
                None => true,
                Some(a) => keep_by_agent
                    .get(&a.agent)
                    .map(|k| a.line <= *k)
                    .unwrap_or(false),
            };
            if within {
                target = Some(id);
            }
        }
        Ok(target)
    }

    /// 目的：把一棵树铺进一个工作树：不一致的文件先备份进该 agent 的 lost/，再删掉、写上目标版本。
    /// 返回：备份的文件数。
    /// 约束：只动列出目录里的普通文件；备份不消费（work_lost / work_restore_lost 照常读回）。
    pub(super) fn lay_down_tree(
        &self,
        store: &Path,
        root: &Path,
        tree: &Tree,
        agent: &str,
        backup: Option<&str>,
    ) -> Result<usize, String> {
        let mut backed = 0usize;
        for rel in self.store.list(root)? {
            let Some(current) = self.store.scan_under(root, &rel, None)? else {
                continue;
            };
            if tree.get(&rel) == Some(&current.hash) {
                continue;
            }
            if let Some(prefix) = backup {
                let lost = lost_root(store, agent)?;
                let dest = format!("{}/{}", prefix, rel);
                self.store
                    .copy_under_to_under(root, &rel, &lost, &dest, &current.hash)?;
                backed += 1;
            }
            self.store.remove_under(root, &rel)?;
        }
        for (rel, hash) in tree {
            let same = self
                .store
                .scan_under(root, rel, None)?
                .map(|v| v.hash)
                .as_deref()
                == Some(hash.as_str());
            if !same {
                self.store.copy_object_to_under(store, hash, root, rel)?;
            }
        }
        Ok(backed)
    }

    /// 目的：把某个 agent 的基线、内容索引与逐路径基线一起对齐到一个提交点（None = 清空）。
    pub(super) fn reset_agent_point(
        &self,
        store: &Path,
        agent: &str,
        point: Option<u64>,
        tree: &Tree,
    ) -> Result<(), String> {
        self.store.write_index(store, agent, tree)?;
        match point {
            Some(id) => {
                let bases: BTreeMap<String, u64> =
                    tree.keys().map(|path| (path.clone(), id)).collect();
                self.write_agent_bases(store, agent, id, &bases)
            }
            None => self.store.remove_ref(store, &format!("agents/{}", agent)),
        }
    }

    /// 目的：删除硬回退时备份进 lost/ 的目录前缀（按落点区分，便于用户认领）。
    pub(super) fn delete_prefix(point: Option<u64>) -> String {
        match point {
            Some(id) => format!("delete-{}", id),
            None => "delete-empty".to_string(),
        }
    }

    /// 目的：作废不在 `keep_marks` 里的留档引用，返回作废号。
    /// 约束：只删引用，不删提交与 lost 备份；调用方已持有版本库锁。
    pub(super) fn prune_archives_locked(
        &self,
        store: &Path,
        keep_marks: &[u64],
    ) -> Result<Vec<u64>, String> {
        let keep: BTreeSet<u64> = keep_marks.iter().copied().collect();
        let mut pruned = Vec::new();
        for (name, r) in self.store.list_refs(store)? {
            if r.kind != RefKind::Archive {
                continue;
            }
            let Some(id) = name
                .strip_prefix("archive/")
                .and_then(|s| s.parse::<u64>().ok())
            else {
                continue;
            };
            if keep.contains(&id) {
                continue;
            }
            self.store.remove_ref(store, &name)?;
            pruned.push(id);
        }
        pruned.sort_unstable();
        Ok(pruned)
    }
}

impl WorkspaceService {
    pub(super) fn work_commit(
        &self,
        work: &str,
        agent: &str,
        session: &str,
        req: &CommitRequest,
    ) -> Result<CommitReport, String> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let _shared = self.dirs.lock_shared(work)?;
        let (shared, store, sandbox) = self.agent_roots(work, agent)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        let anchor = CommitAnchor {
            agent: agent.to_string(),
            line: req.line,
        };
        let report = self.commit_locked(
            work,
            &shared,
            &store,
            &sandbox,
            agent,
            session,
            req,
            Source::Tool,
            Some(anchor),
        )?;
        self.record_committed_bases(&store, agent, &report)?;
        Ok(report)
    }

    pub(super) fn work_commit_auto(
        &self,
        work: &str,
        agent: &str,
        session: &str,
        req: &CommitRequest,
    ) -> Result<Option<CommitReport>, CommitRefusal> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let _shared = self.dirs.lock_shared(work).map_err(CommitRefusal::Broken)?;
        let (shared, store, sandbox) = self
            .agent_roots(work, agent)
            .map_err(CommitRefusal::Broken)?;
        let _lock = self.store.lock(&store).map_err(CommitRefusal::Broken)?;
        self.recover_locked(work, &shared, &store)
            .map_err(CommitRefusal::Broken)?;
        // 基线准入在写共享区之前：过期就一条 pending 都不留。
        if let Some(main) = self.store.head(&store).map_err(CommitRefusal::Broken)? {
            let tree =
                Self::tree_of(self.store.as_ref(), &store, main).map_err(CommitRefusal::Broken)?;
            let paths = self
                .commit_paths(&sandbox, req)
                .map_err(CommitRefusal::Broken)?;
            let bases = self
                .agent_bases(&store, agent)
                .map_err(CommitRefusal::Broken)?;
            let stale = stale_paths(BaseRef { oid: main }, &bases, &paths, |path| {
                tree.contains_key(path)
            });
            if !stale.is_empty() {
                return Err(CommitRefusal::Stale {
                    main: BaseRef { oid: main },
                    paths: stale,
                });
            }
        }
        let anchor = CommitAnchor {
            agent: agent.to_string(),
            line: req.line,
        };
        let report = match self.commit_locked(
            work,
            &shared,
            &store,
            &sandbox,
            agent,
            session,
            req,
            Source::Tool,
            Some(anchor),
        ) {
            Ok(r) => r,
            // 内容与主线一致（如把同样内容再写一遍）：没有可发布的改动，不算失败。
            Err(e) if e.starts_with("没有可提交的改动") => return Ok(None),
            Err(e) => return Err(CommitRefusal::Broken(e)),
        };
        self.record_committed_bases(&store, agent, &report)
            .map_err(CommitRefusal::Broken)?;
        Ok(Some(report))
    }

    pub(super) fn work_commit_user(
        &self,
        work: &str,
        path: &str,
        bytes: &[u8],
        time: i64,
        line: u64,
    ) -> Result<u64, String> {
        let rel = clean_rel(path)?;
        if rel.contains('/') {
            return Err(format!("用户投喂只收顶层文件名：{}", rel));
        }
        if bytes.len() as u64 > self.max_asset_bytes {
            return Err(render_oversize(
                &[(rel.clone(), bytes.len() as u64)],
                self.max_asset_bytes,
            ));
        }
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let _shared = self.dirs.lock_shared(work)?;
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        let (head_id, tree) = self.head_tree(&store)?;
        let hash = content_hash(bytes);
        self.store.write_object(&store, &hash, bytes)?;
        let kind = if tree.contains_key(&rel) {
            ChangeKind::Modify
        } else {
            ChangeKind::Add
        };
        let id = self.reserve_id(&store)?;
        let changes = vec![Change {
            path: rel.clone(),
            kind,
            hash,
        }];
        let depth = self.next_depth(&store, head_id)?;
        let commit = Commit {
            id,
            parent: head_id,
            base: head_id.unwrap_or(0),
            author: "user".to_string(),
            session: String::new(),
            time,
            message: format!("用户投喂：{}", rel),
            changes: changes.clone(),
            tree: deltas_tree(&tree, &changes, depth),
            depth,
            source: Source::User,
            anchor: Some(CommitAnchor {
                agent: String::new(),
                line,
            }),
        };
        let pending = Pending::Commit {
            commit: Box::new(commit.clone()),
            index: None,
        };
        self.store.write_pending(&store, &pending)?;
        let result = self.apply_commit_pending(work, &shared, &store, &commit, None);
        self.settle(work, &shared, &store, result)?;
        Ok(id)
    }
}
