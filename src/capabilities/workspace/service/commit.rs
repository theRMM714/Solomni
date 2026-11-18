//! 目的：工作区的提交用例——把沙箱改写落成提交、落整棵树或增量、维护三条线各自的头、推送与归档标记。
//! 管：commit_locked 与 work_commit / work_commit_auto / work_commit_user / work_push 四个用例、`refs/agents/<agent>`（工作线）与 `refs/agents/<agent>/saves`（存档线）的读写、归档标记的写入与修剪。
//! 不管：三方比较的纯规则（domain::workstore）、目录布局与共享区落盘（ports::WorkStore）。
//! 联动：service.rs 提供锁、工作根与树物化；sync 与 lifecycle 会调用本文件的提交与锚点方法。

use super::*;
use crate::capabilities::workspace::api::{
    Change, ChangeKind, CommitReport, CommitRequest, PushReport, PushRequest,
};

impl WorkspaceService {
    /// 目的：提交的唯一实现（调用方已持 gate、shared、store 锁并已恢复事务）。
    /// 约束：`line` 决定提交落到哪条线——主线会改共享区与 `refs/main`，存档线只写提交与 `refs/agents/<agent>/saves`。
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
        line: CommitLine,
    ) -> Result<CommitReport, String> {
        let msg = req.message.trim();
        if msg.is_empty() {
            return Err("提交说明不能为空".to_string());
        }
        validate_paths(&req.paths)?;
        let shared = shared.to_path_buf();
        let store = store.to_path_buf();
        let sandbox = sandbox.to_path_buf();
        let (parent, tree) = match line {
            CommitLine::Main => self.head_tree(&store)?,
            CommitLine::Work => self.work_tree(&store, agent)?,
            CommitLine::Save => self.save_tree(&store, agent)?,
        };
        let index: Index = match line {
            CommitLine::Main => self.store.read_index(&store, agent)?,
            // 工作线与存档线的基线都是它自己头上那一版：本地提交不做准入比较，只按点名路径落改动。
            CommitLine::Work | CommitLine::Save => tree.clone(),
        };
        // 沙箱里的文件全集（paths 里的文件夹据此展开）。
        let sandbox_files: BTreeSet<String> = self.store.list(&sandbox)?.into_iter().collect();
        // 存档线记的是**整棵沙箱快照**：回档要把沙箱铺回它当时的样子，只记调用方点名的路径会漏。
        let (want, missing) = match line {
            CommitLine::Main | CommitLine::Work => expand_paths(&req.paths, &sandbox_files),
            CommitLine::Save => (sandbox_files.iter().cloned().collect(), Vec::new()),
        };
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
        if line != CommitLine::Save {
            for raw in &req.deletes {
                let rel = clean_rel(raw)?;
                if locals.contains_key(&rel) {
                    return Err(format!("{} 同时出现在 paths 与 deletes 里", rel));
                }
                locals.insert(rel, None);
            }
        }
        // 工作线上「删掉一个拉过、却没在自己的线上提交过」的路径：我的树里本来就没有它，plan_commit 只会说
        // 「已经没有了」。补一条按拉取基线指纹的删除记录，推送时的三方比较才认得出这条删除。
        let mut carried: Vec<Change> = Vec::new();
        if line == CommitLine::Work {
            let pulled = self.store.read_index(&store, agent)?;
            for raw in &req.deletes {
                let rel = clean_rel(raw)?;
                if tree.contains_key(&rel) || locals.get(&rel).is_some_and(Option::is_some) {
                    continue;
                }
                if let Some(hash) = pulled.get(&rel) {
                    locals.remove(&rel);
                    carried.push(Change {
                        path: rel,
                        kind: ChangeKind::Delete,
                        hash: hash.clone(),
                    });
                }
            }
        }
        if locals.is_empty() && carried.is_empty() {
            return Err("没有可提交的改动：paths 与 deletes 都是空的".to_string());
        }
        // 外部工作区的磁盘是真相：写共享区前要核对外部改动（存档线不碰共享区）。
        if line == CommitLine::Main && self.dirs.is_external(work) {
            self.reject_external_changes(&shared, &tree, &locals)?;
        }
        let plan = match plan_commit(&index, &tree, &locals) {
            Ok(p) => p,
            Err(conflicts) => return Err(render_conflicts(&conflicts)),
        };
        let mut changes = plan.changes;
        if changes.is_empty() && carried.is_empty() {
            return Err(render_unchanged(&plan.unchanged));
        }
        changes.extend(carried);
        changes.sort_by(|a, b| a.path.cmp(&b.path));
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
        let pending_index = match line {
            CommitLine::Main => {
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
                Some(PendingIndex {
                    agent: agent.to_string(),
                    index: new_index,
                })
            }
            // 工作线与存档线都不改「从主副本拉过什么」的基线：本地提交与快照不等于拉取。
            CommitLine::Work | CommitLine::Save => None,
        };
        let depth = self.next_depth(&store, parent)?;
        let commit = Commit {
            id,
            parent,
            base: parent.unwrap_or(0),
            author: agent.to_string(),
            session: session.to_string(),
            time: req.time,
            message: msg.to_string(),
            changes: changes.clone(),
            tree: deltas_tree(&tree, &changes, depth),
            depth,
            source,
            line,
            // 主线提交才 squash 工作线；本地提交与快照没有来源集合。
            sources: Vec::new(),
            anchor,
        };
        let pending = Pending::Commit {
            commit: Box::new(commit.clone()),
            index: pending_index.clone(),
            line,
        };
        self.store.write_pending(&store, &pending)?;
        let result =
            self.apply_commit_pending(work, &shared, &store, &commit, pending_index.as_ref(), line);
        self.settle(work, &shared, &store, result)?;
        Ok(CommitReport { id, changes })
    }

    /// 目的：该 agent 发布线的引用名——它只跟着主线走（发布成功 = 快进到这里）。
    pub(super) fn work_ref(agent: &str) -> String {
        format!("agents/{}", agent)
    }

    /// 目的：该 agent 存档线的引用名——沙箱快照只落在它上面，永不进共享区。
    pub(super) fn save_ref(agent: &str) -> String {
        format!("agents/{}/saves", agent)
    }

    /// 目的：把一条线的头挪到 `target`；引用不存在就建。
    /// 约束：只改 `target`，不动 `heads`（归档引用用该字段记各 agent 的回退点）。
    pub(super) fn set_line_head(
        &self,
        store: &Path,
        name: &str,
        kind: RefKind,
        target: Option<u64>,
    ) -> Result<(), String> {
        let mut r = self.store.read_ref(store, name)?.unwrap_or(Ref {
            kind,
            target: None,
            heads: BTreeMap::new(),
            saves: BTreeMap::new(),
        });
        r.kind = kind;
        r.target = target;
        self.store.write_ref(store, name, &r)
    }

    /// 目的：该 agent 存档线的头；旧布局（`agents/<agent>` 同时记逐路径基线）读到就升级成两条线。
    /// 约束：升级只在「存档线还不存在、旧引用里有基线」时发生一次；旧目标当存档起点，基线本身不再参与任何判定。
    pub(super) fn save_head(&self, store: &Path, agent: &str) -> Result<Option<u64>, String> {
        let name = Self::save_ref(agent);
        if let Some(r) = self.store.read_ref(store, &name)? {
            return Ok(r.target);
        }
        let work = Self::work_ref(agent);
        let Some(old) = self.store.read_ref(store, &work)? else {
            return Ok(None);
        };
        if old.kind != RefKind::Agent || old.heads.is_empty() {
            return Ok(None);
        }
        self.set_line_head(store, &name, RefKind::Agent, old.target)?;
        self.set_line_head(store, &work, RefKind::Agent, old.target)?;
        Ok(old.target)
    }

    /// 目的：该 agent 存档线的头与整棵树（空线 = `(None, 空树)`）。
    fn save_tree(&self, store: &Path, agent: &str) -> Result<(Option<u64>, Tree), String> {
        let Some(id) = self.save_head(store, agent)? else {
            return Ok((None, Tree::new()));
        };
        Ok((Some(id), Self::tree_of(self.store.as_ref(), store, id)?))
    }

    /// 目的：该 agent 工作线的头与整棵树（空线 = `(None, 空树)`）。
    pub(super) fn work_tree(
        &self,
        store: &Path,
        agent: &str,
    ) -> Result<(Option<u64>, Tree), String> {
        let Some(r) = self.store.read_ref(store, &Self::work_ref(agent))? else {
            return Ok((None, Tree::new()));
        };
        match r.target {
            Some(id) => Ok((Some(id), Self::tree_of(self.store.as_ref(), store, id)?)),
            None => Ok((None, Tree::new())),
        }
    }

    /// 目的：推送成功后把沙箱里**没被本地改过**的路径快进到合并结果（与 git 的 checkout 同一语义）。
    /// 约束：沙箱与我的工作线头不一致的路径有未提交改动，一律不动，如实留给调用方。
    fn fast_forward_sandbox(
        &self,
        store: &Path,
        sandbox: &Path,
        mine: &Tree,
        merged: &Tree,
    ) -> Result<(), String> {
        for (rel, hash) in merged {
            let local = self.store.scan_under(sandbox, rel, None)?.map(|v| v.hash);
            if local.as_deref() == Some(hash.as_str()) {
                continue;
            }
            if local.as_deref() != mine.get(rel).map(String::as_str) {
                continue;
            }
            self.store.copy_object_to_under(store, hash, sandbox, rel)?;
        }
        for (rel, hash) in mine {
            if merged.contains_key(rel) {
                continue;
            }
            let local = self.store.scan_under(sandbox, rel, None)?.map(|v| v.hash);
            if local.as_deref() != Some(hash.as_str()) {
                continue;
            }
            self.store.remove_under(sandbox, rel)?;
        }
        Ok(())
    }

    /// 目的：存档线推进到一条新的存档提交（只前进，不倒推——倒推会把沙箱快照接回旧内容）。
    pub(super) fn advance_save_line(
        &self,
        store: &Path,
        agent: &str,
        id: u64,
    ) -> Result<(), String> {
        if let Some(head) = self.save_head(store, agent)?
            && head > id
        {
            return Err(format!(
                "待恢复存档 {} 小于当前存档头 {}，拒绝倒推存档线",
                id, head
            ));
        }
        self.set_line_head(store, &Self::save_ref(agent), RefKind::Agent, Some(id))
    }

    /// 目的：把该 agent 的发布线快进到一条主线提交（发布成功 / 回退之后）。
    pub(super) fn fast_forward_work(
        &self,
        store: &Path,
        agent: &str,
        commit: Option<u64>,
    ) -> Result<(), String> {
        self.set_line_head(store, &Self::work_ref(agent), RefKind::Agent, commit)
    }

    /// 目的：清掉该 agent 的存档线头（沙箱被整体重铺后，旧快照不再描述沙箱内容）。
    pub(super) fn drop_save_line(&self, store: &Path, agent: &str) -> Result<(), String> {
        self.store.remove_ref(store, &Self::save_ref(agent))
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

    /// 目的：把一个 agent 的发布线与存档线一起对齐到一个提交点（None = 清空两条线）。
    /// 约束：沙箱被整体重铺后，旧存档快照不再描述沙箱内容，所以存档线跟着落点重建：
    ///   `work` 是主线落点，`save` 是沙箱铺的那份快照（None = 铺主线树），
    ///   `tree` 是索引/基线（三方比较的 base），始终取主线落点树。
    pub(super) fn reset_agent_point(
        &self,
        store: &Path,
        agent: &str,
        work: Option<u64>,
        save: Option<u64>,
        tree: &Tree,
    ) -> Result<(), String> {
        self.store.write_index(store, agent, tree)?;
        self.fast_forward_work(store, agent, work)?;
        match save {
            Some(id) => self.set_line_head(store, &Self::save_ref(agent), RefKind::Agent, Some(id)),
            None => self.drop_save_line(store, agent),
        }
    }

    /// 目的：该 agent 存档线上「转录行还没越过 keep」的最后一个快照（None = 沙箱那时还是空的）。
    /// 约束：只沿存档线自己的祖先链找；没有锚的存档保守地算在回档点之前。
    pub(super) fn save_point_at(
        &self,
        store: &Path,
        agent: &str,
        keep: u64,
    ) -> Result<Option<u64>, String> {
        let mut cur = self.save_head(store, agent)?;
        let mut seen: BTreeSet<u64> = BTreeSet::new();
        while let Some(id) = cur {
            if !seen.insert(id) {
                break;
            }
            let Some(c) = self.store.read_commit(store, id)? else {
                break;
            };
            let within = match &c.anchor {
                None => true,
                Some(a) => a.agent == agent && a.line <= keep,
            };
            if within {
                return Ok(Some(id));
            }
            cur = c.parent;
        }
        Ok(None)
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
            session: session.to_string(),
            line: req.line,
        };
        self.commit_locked(
            work,
            &shared,
            &store,
            &sandbox,
            agent,
            session,
            req,
            Source::Tool,
            Some(anchor),
            CommitLine::Work,
        )
    }

    /// 目的：把工作线上已提交的改动推送进主线：三方合并 → 一条 squash 提交 → 物化共享区 → 两条线快进。
    /// 约束：冲突整次拒绝（不自动合并）；成功后沙箱里没被本地改过的路径一起快进到合并结果。
    pub(super) fn work_push(
        &self,
        work: &str,
        agent: &str,
        session: &str,
        req: &PushRequest,
    ) -> Result<PushReport, String> {
        let msg = req.message.trim();
        if msg.is_empty() {
            return Err("提交说明不能为空（推送要一条主线提交的说明）".to_string());
        }
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let _shared = self.dirs.lock_shared(work)?;
        let (shared, store, sandbox) = self.agent_roots(work, agent)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        let (head, theirs) = self.head_tree(&store)?;
        let (work_id, mine) = self.work_tree(&store, agent)?;
        // 基线 = 我拉取过的主线内容：没拉过的路径基线为空，合并时按「我新增」处理。
        let base: Tree = self.store.read_index(&store, agent)?;
        let plan = match plan_merge(&base, &mine, &theirs) {
            Ok(p) => p,
            Err(conflicts) => return Err(render_push_conflicts(&conflicts)),
        };
        if plan.changes.is_empty() {
            return Ok(PushReport {
                commit: None,
                changes: Vec::new(),
            });
        }
        let mut locals: BTreeMap<String, Option<String>> = BTreeMap::new();
        for ch in &plan.changes {
            let value = match ch.kind {
                ChangeKind::Delete => None,
                ChangeKind::Add | ChangeKind::Modify => Some(ch.hash.clone()),
            };
            locals.insert(ch.path.clone(), value);
        }
        // 外部工作区的磁盘是真相：合并要写共享区，先核对外部改动。
        if self.dirs.is_external(work) {
            self.reject_external_changes(&shared, &theirs, &locals)?;
        }
        // 对象：工作线提交时已把内容灌进版本库，这里只核对对象还在（缺了就是版本库损坏）。
        for ch in &plan.changes {
            if matches!(ch.kind, ChangeKind::Add | ChangeKind::Modify) {
                self.store.stat_object(&store, &ch.hash).map_err(|e| {
                    format!("推送被拒：{} 的内容对象不在版本库里（{}）", ch.path, e)
                })?;
            }
        }
        // 这次 squash 吸收了哪些本地提交：work_log 据此把已发布的提交标成「已推送」。
        let main_ancestors = self.head_ancestors(&store, head)?;
        let sources: Vec<u64> = self
            .head_ancestors(&store, work_id)?
            .into_iter()
            .filter(|id| !main_ancestors.contains(id))
            .collect();
        let id = self.reserve_id(&store)?;
        let commit = Commit {
            id,
            parent: head,
            base: head.unwrap_or(0),
            author: agent.to_string(),
            session: session.to_string(),
            time: req.time,
            message: msg.to_string(),
            changes: plan.changes.clone(),
            // 主线提交直接带整棵树：推送次数少、回档要按它铺共享区，不必回溯。
            tree: Some(plan.tree.clone()),
            depth: 0,
            source: Source::Tool,
            line: CommitLine::Main,
            sources,
            anchor: Some(CommitAnchor {
                agent: agent.to_string(),
                session: session.to_string(),
                line: req.line,
            }),
        };
        let index = PendingIndex {
            agent: agent.to_string(),
            index: plan.tree.clone(),
        };
        let pending = Pending::Commit {
            commit: Box::new(commit.clone()),
            index: Some(index.clone()),
            line: CommitLine::Main,
        };
        self.store.write_pending(&store, &pending)?;
        let result = self.apply_commit_pending(
            work,
            &shared,
            &store,
            &commit,
            Some(&index),
            CommitLine::Main,
        );
        self.settle(work, &shared, &store, result)?;
        self.fast_forward_sandbox(&store, &sandbox, &mine, &plan.tree)?;
        Ok(PushReport {
            commit: Some(id),
            changes: plan.changes,
        })
    }

    /// 目的：把沙箱这次的改写记进该 agent 的存档线（`refs/agents/<agent>/saves`），共享区一个字都不动。
    /// 约束：存档不是发布——它只给回档/删除/恢复提供「沙箱在这次调用之后长什么样」的还原点，进不了主线；
    ///   要进主线得显式提交并发布（work_commit + work_push）。
    pub(super) fn work_commit_auto(
        &self,
        work: &str,
        agent: &str,
        session: &str,
        req: &CommitRequest,
    ) -> Result<Option<CommitReport>, String> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let _shared = self.dirs.lock_shared(work)?;
        let (shared, store, sandbox) = self.agent_roots(work, agent)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        let anchor = CommitAnchor {
            agent: agent.to_string(),
            session: session.to_string(),
            line: req.line,
        };
        match self.commit_locked(
            work,
            &shared,
            &store,
            &sandbox,
            agent,
            session,
            req,
            Source::Tool,
            Some(anchor),
            CommitLine::Save,
        ) {
            Ok(r) => Ok(Some(r)),
            // 与上次存档完全一致（如只读了一次文件）：没有可记的改动，不算失败。
            Err(e) if e.starts_with("没有可提交的改动") => Ok(None),
            Err(e) => Err(e),
        }
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
            line: CommitLine::Main,
            sources: Vec::new(),
            anchor: Some(CommitAnchor {
                agent: String::new(),
                session: String::new(),
                line,
            }),
        };
        let pending = Pending::Commit {
            commit: Box::new(commit.clone()),
            index: None,
            line: CommitLine::Main,
        };
        self.store.write_pending(&store, &pending)?;
        let result =
            self.apply_commit_pending(work, &shared, &store, &commit, None, CommitLine::Main);
        self.settle(work, &shared, &store, result)?;
        Ok(id)
    }
}
