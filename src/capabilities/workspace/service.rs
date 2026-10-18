//! 工作区的**用例与端口持有者**：出站端口只在这里（R12）——
//! 目录布局、共享区版本库。别的能力要工作区目录与版本化，走 `api::Workspace`。
//!
//! 装配（new 出适配器）在组合根；这里只收注入的端口。
//!
//! 共享区是**版本化工作区**：主副本 `work/` 只在提交时改写，agent 的沙箱是它的工作副本，
//! pull / commit 的三方比较与 resolve 的选边规则在 `domain::workstore`（纯逻辑），落盘在 `WorkStore` 端口后面。

use crate::capabilities::workspace::api::{
    Change, ChangeKind, CommitReport, CommitRequest, CommitSummary, FileVersion, GcReport,
    PullReport, ResolveAction, ResolveEntry, ResolveReport, ResolveRequest, ResolveTake,
    StatusEntry, StatusReport, VersionView, WorkFiles, WorkRoots, WorkUsage, Workspace,
};
use crate::capabilities::workspace::domain::hash::{content_hash, is_content_hash};
use crate::capabilities::workspace::domain::workstore::{
    clean_rel, expand_paths, plan_commit, pull_verdict, status_of, Commit, CommitAnchor, Conflict,
    ConflictAt, Index, Pending, PendingIndex, PullVerdict, Tree,
};
use crate::capabilities::workspace::ports::{WorkStore, Workdirs};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// 一次 status 里主副本路径清单的上限（超出如实标注截断，不假装列全）。
const MAX_STATUS_TREE: usize = 200;
/// status 里附带的最近提交条数。
const RECENT_COMMITS: usize = 5;
/// status 里列出的“沙箱未跟踪文件”上限（超出如实标注截断）。
const MAX_LOCAL_ONLY: usize = 50;

/// 工作区能力：持四个端口，按用例答话。
pub struct WorkspaceService {
    dirs: Arc<dyn Workdirs + Send + Sync>,
    store: Arc<dyn WorkStore + Send + Sync>,
    /// 版本库写入的串行闸（**每个工作一把**）：不同工作互不相干、可并行；同一个工作的
    /// 提交 / 拉取 / 回档严格串行。它保护的是**版本库状态**（head、基线、对象），不是核心状态。
    version_gates: Mutex<BTreeMap<String, Arc<Mutex<()>>>>,
}

impl WorkspaceService {
    /// 组合根专用。
    pub fn new(
        dirs: Arc<dyn Workdirs + Send + Sync>,
        store: Arc<dyn WorkStore + Send + Sync>,
    ) -> WorkspaceService {
        WorkspaceService {
            dirs,
            store,
            version_gates: Mutex::new(BTreeMap::new()),
        }
    }

    /// 一个工作的（主副本根, 版本库目录）。
    fn work_roots(&self, work: &str) -> Result<(PathBuf, PathBuf), String> {
        let roots = self.dirs.roots(work, &[])?;
        Ok((roots.shared, roots.store))
    }

    /// 一个工作里某 agent 的（主副本根, 版本库目录, 该 agent 沙箱）。
    fn agent_roots(&self, work: &str, agent: &str) -> Result<(PathBuf, PathBuf, PathBuf), String> {
        let roots = self.dirs.roots(work, &[agent.to_string()])?;
        let sandbox = roots
            .agents
            .get(agent)
            .cloned()
            .ok_or_else(|| format!("工作区没有给出 agent {} 的沙箱路径", agent))?;
        Ok((roots.shared, roots.store, sandbox))
    }

    /// 目的：取某个工作的版本库写闸（没有就建一个）。
    /// 约束：返回的 `Arc` 要活到闸用完——调用方先拿它，再 `lock()`，别把它当临时值。
    pub(crate) fn gate(&self, work: &str) -> Arc<Mutex<()>> {
        let mut gates = self.version_gates.lock().unwrap_or_else(|e| e.into_inner());
        gates
            .entry(work.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// 把主副本物化到某个提交点（None = 清空），head 同步。
    /// 机制层只做"按目标树写回、删掉树外的文件"；策略（物化到哪一点）由调用方给。
    fn materialize(
        store_port: &dyn WorkStore,
        shared: &std::path::Path,
        store: &std::path::Path,
        target: Option<u64>,
    ) -> Result<(), String> {
        let tree = match target {
            Some(id) => {
                store_port
                    .read_commit(store, id)?
                    .ok_or_else(|| format!("没有提交点 {}", id))?
                    .tree
            }
            None => Tree::new(),
        };
        for rel in store_port.list(shared)? {
            if !tree.contains_key(&rel) {
                store_port.remove_under(shared, &rel)?;
            }
        }
        for (rel, hash) in &tree {
            let bytes = store_port.read_object(store, hash)?;
            store_port.write_under(shared, rel, &bytes)?;
        }
        match target {
            Some(id) => store_port.set_head(store, id)?,
            None => store_port.clear_head(store)?,
        }
        Ok(())
    }

    /// 目的：预留一个单调递增的提交号；只增不复用，允许空洞。
    fn reserve_id(&self, store: &Path) -> Result<u64, String> {
        let max = self.store.list_commits(store)?.last().copied().unwrap_or(0);
        let next = self
            .store
            .read_next_id(store)?
            .unwrap_or(max + 1)
            .max(max + 1);
        self.store.write_next_id(store, next + 1)?;
        Ok(next)
    }

    /// 目的：把一条待恢复提交幂等地应用到主副本、提交记录、head 与基线。
    /// 约束：调用方已持有工作锁与跨进程锁；不删除 pending（由恢复方或收尾方删）。
    fn apply_commit_pending(
        &self,
        work: &str,
        shared: &Path,
        store: &Path,
        commit: &Commit,
        index: Option<&PendingIndex>,
    ) -> Result<(), String> {
        if let Some(head) = self.store.head(store)? {
            if head > commit.id {
                return Err(format!(
                    "待恢复提交 {} 小于当前 head {}，拒绝倒推共享区",
                    commit.id, head
                ));
            }
        }
        for ch in &commit.changes {
            match ch.kind {
                ChangeKind::Add | ChangeKind::Modify => {
                    let bytes = self.store.read_object(store, &ch.hash)?;
                    if commit.author == "user" {
                        self.dirs.write_work(work, &ch.path, &bytes)?;
                    } else {
                        self.store.write_under(shared, &ch.path, &bytes)?;
                    }
                }
                ChangeKind::Delete => {
                    self.store.remove_under(shared, &ch.path)?;
                }
            }
        }
        self.store.write_commit(store, commit)?;
        self.store.set_head(store, commit.id)?;
        if let Some(index) = index {
            self.store.write_index(store, &index.agent, &index.index)?;
        }
        Ok(())
    }

    /// 目的：删除 keep 祖先链之外的提交记录（keep=None 时全部删）。
    /// 约束：调用方已先物化到 keep，head 不再指向被删的提交。
    fn discard_commits_locked(&self, store: &Path, keep: Option<u64>) -> Result<(), String> {
        let mut keep_set: BTreeSet<u64> = BTreeSet::new();
        let mut cur = keep;
        while let Some(id) = cur {
            if !keep_set.insert(id) {
                break;
            }
            cur = self.store.read_commit(store, id)?.and_then(|c| c.parent);
        }
        for id in self.store.list_commits(store)? {
            if !keep_set.contains(&id) {
                self.store.remove_commit(store, id)?;
            }
        }
        Ok(())
    }

    /// 目的：重放该工作尚未完成的版本库事务；没有 = 无操作。
    /// 约束：调用方已持有工作锁与跨进程锁。
    fn recover_locked(&self, work: &str, shared: &Path, store: &Path) -> Result<(), String> {
        let Some(pending) = self.store.read_pending(store)? else {
            return Ok(());
        };
        match &pending {
            Pending::Commit { commit, index } => {
                self.apply_commit_pending(work, shared, store, commit.as_ref(), index.as_ref())?
            }
            Pending::Restore { target } => {
                WorkspaceService::materialize(self.store.as_ref(), shared, store, *target)?
            }
            Pending::Discard { keep } => {
                WorkspaceService::materialize(self.store.as_ref(), shared, store, *keep)?;
                self.discard_commits_locked(store, *keep)?;
            }
        }
        self.store.clear_pending(store)
    }

    /// 目的：事务收尾；失败时尝试一次重放，重放成功就按成功返回。
    /// 约束：只在 pending 已经写下去之后调用。
    fn settle(
        &self,
        work: &str,
        shared: &Path,
        store: &Path,
        result: Result<(), String>,
    ) -> Result<(), String> {
        match result {
            Ok(()) => match self.store.clear_pending(store) {
                Ok(()) => Ok(()),
                Err(e) => match self.recover_locked(work, shared, store) {
                    Ok(()) => Ok(()),
                    Err(_) => Err(format!("收尾失败：{}；已留下恢复日志", e)),
                },
            },
            Err(e) => match self.recover_locked(work, shared, store) {
                Ok(()) => Ok(()),
                Err(_) => Err(format!(
                    "版本库写未完成：{}；已留下恢复日志，下一次操作会继续",
                    e
                )),
            },
        }
    }

    /// 读 head 的整棵树；空仓库 = 空树。
    fn head_tree(&self, store: &std::path::Path) -> Result<(Option<u64>, Tree), String> {
        let Some(id) = self.store.head(store)? else {
            return Ok((None, Tree::new()));
        };
        let commit = self
            .store
            .read_commit(store, id)?
            .ok_or_else(|| format!("head 指向不存在的提交 {}", id))?;
        Ok((Some(id), commit.tree))
    }
}

/// 冲突清单渲染成模型侧回执（核心自有工具的文案，与代理工具同一口径）。
fn render_conflicts(conflicts: &[ConflictAt]) -> String {
    let mut out = String::from("提交被拒：以下路径有冲突（不做自动合并）：");
    for c in conflicts {
        out.push_str(&format!("\n- {}（{}）", c.path, c.kind.reason()));
    }
    out.push_str(
        "\n先用 work_versions 看基线 / 主副本 / 沙箱三方，人工合并后再用 work_resolve（take=local）声明已解决；\n要放弃本地改动用 take=upstream。二进制冲突只能选边，不做自动合并。",
    );
    out
}

/// 读一个内容寻址对象并收成版本视图（调用方已持有工作锁）。
fn object_version(store: &dyn WorkStore, root: &Path, hash: &str) -> Result<FileVersion, String> {
    let bytes = store.read_object(root, hash)?;
    Ok(FileVersion::from_bytes(hash.to_string(), &bytes))
}

impl Workspace for WorkspaceService {
    fn prepare(&self, session: &str, agents: &[String]) -> Result<(), String> {
        self.dirs.prepare(session, agents)
    }

    fn discard(&self, session: &str, agents: &[String]) -> Result<(), String> {
        self.dirs.discard(session, agents)
    }

    fn work_has(&self, session: &str, name: &str) -> bool {
        self.dirs.work_has(session, name)
    }

    fn files(&self, session: &str, agents: &[String]) -> Result<WorkFiles, String> {
        self.dirs.list(session, agents)
    }

    fn usage(&self, session: &str, agents: &[String]) -> Result<WorkUsage, String> {
        self.dirs.usage(session, agents)
    }

    fn roots(&self, session: &str, agents: &[String]) -> Result<WorkRoots, String> {
        self.dirs.roots(session, agents)
    }

    fn work_pull(&self, work: &str, agent: &str, paths: &[String]) -> Result<PullReport, String> {
        if paths.is_empty() {
            return Err("work_pull 要给出要拉取的路径（文件或文件夹）".to_string());
        }
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (shared, store, sandbox) = self.agent_roots(work, agent)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        let (head, tree) = self.head_tree(&store)?;
        let Some(head) = head else {
            return Err("共享区还没有任何提交：没有可拉取的内容".to_string());
        };
        let available: BTreeSet<String> = tree.keys().cloned().collect();
        let (want, missing) = expand_paths(paths, &available);
        let mut index: Index = self.store.read_index(&store, agent)?;
        let mut report = PullReport {
            commit: head,
            missing,
            ..PullReport::default()
        };
        for rel in want {
            let head_hash = tree.get(&rel).cloned().unwrap_or_default();
            let local = self.store.read_under(&sandbox, &rel)?;
            let local_hash = local.as_ref().map(|b| content_hash(b));
            let base = index.get(&rel).map(String::as_str);
            match pull_verdict(base, Some(&head_hash), local_hash.as_deref()) {
                PullVerdict::Update => {
                    let bytes = self.store.read_object(&store, &head_hash)?;
                    self.store.write_under(&sandbox, &rel, &bytes)?;
                    index.insert(rel.clone(), head_hash);
                    report.updated.push(rel);
                }
                PullVerdict::AlreadySame => {
                    index.insert(rel.clone(), head_hash);
                    report.already.push(rel);
                }
                PullVerdict::NoChange => report.no_change.push(rel),
                PullVerdict::Occupied => report.occupied.push(rel),
                PullVerdict::Conflict => {
                    // pull 侧能落到 Conflict 的形态：本地删了/上游改了，或两边都改了。
                    let kind = if local_hash.is_none() {
                        Conflict::DeleteVsModify
                    } else {
                        Conflict::BothModified
                    };
                    report.conflicts.push(ConflictAt { path: rel, kind });
                }
                PullVerdict::Missing => report.missing.push(rel),
            }
        }
        self.store.write_index(&store, agent, &index)?;
        Ok(report)
    }

    fn work_commit(
        &self,
        work: &str,
        agent: &str,
        session: &str,
        req: &CommitRequest,
    ) -> Result<CommitReport, String> {
        let msg = req.message.trim();
        if msg.is_empty() {
            return Err("提交说明不能为空".to_string());
        }
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (shared, store, sandbox) = self.agent_roots(work, agent)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
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
        let mut locals: BTreeMap<String, Option<Vec<u8>>> = BTreeMap::new();
        for rel in want {
            let bytes = self
                .store
                .read_under(&sandbox, &rel)?
                .ok_or_else(|| format!("沙箱里没有：{}", rel))?;
            locals.insert(rel, Some(bytes));
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
        let changes = match plan_commit(&index, &tree, &locals) {
            Ok(c) => c,
            Err(conflicts) => return Err(render_conflicts(&conflicts)),
        };
        if changes.is_empty() {
            return Err("没有可提交的改动：这些路径与主副本一致".to_string());
        }
        // 先落对象，再写事务日志；主副本与元数据由事务统一应用/恢复。
        let id = self.reserve_id(&store)?;
        let mut new_tree = tree.clone();
        for ch in &changes {
            match ch.kind {
                ChangeKind::Add | ChangeKind::Modify => {
                    let bytes = locals
                        .get(&ch.path)
                        .and_then(|v| v.as_ref())
                        .ok_or_else(|| format!("内部错误：{} 没有本地内容", ch.path))?;
                    self.store.write_object(&store, &ch.hash, bytes)?;
                    new_tree.insert(ch.path.clone(), ch.hash.clone());
                }
                ChangeKind::Delete => {
                    new_tree.remove(&ch.path);
                }
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
        let commit = Commit {
            id,
            parent: head_id,
            author: agent.to_string(),
            session: session.to_string(),
            time: req.time,
            message: msg.to_string(),
            changes: changes.clone(),
            tree: new_tree,
            anchor: Some(CommitAnchor {
                agent: agent.to_string(),
                line: req.line,
            }),
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

    fn work_commit_user(
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
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
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
        let mut new_tree = tree;
        new_tree.insert(rel.clone(), hash.clone());
        let commit = Commit {
            id,
            parent: head_id,
            author: "user".to_string(),
            session: String::new(),
            time,
            message: format!("用户投喂：{}", rel),
            changes: vec![Change {
                path: rel.clone(),
                kind,
                hash,
            }],
            tree: new_tree,
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

    fn work_status(
        &self,
        work: &str,
        agent: &str,
        paths: &[String],
    ) -> Result<StatusReport, String> {
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
        let want: Vec<String> = if paths.is_empty() {
            all.iter().cloned().collect()
        } else {
            let (got, _) = expand_paths(paths, &available);
            got
        };
        let mut entries: Vec<StatusEntry> = Vec::new();
        for rel in want {
            let local = self.store.read_under(&sandbox, &rel)?;
            let local_hash = local.as_ref().map(|b| content_hash(b));
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
        // 最近提交（新→旧）。
        let mut ids = self.store.list_commits(&store)?;
        ids.reverse();
        let mut recent = Vec::new();
        for id in ids.into_iter().take(RECENT_COMMITS) {
            if let Some(c) = self.store.read_commit(&store, id)? {
                recent.push(CommitSummary::from(&c));
            }
        }
        Ok(StatusReport {
            head,
            tree: all_tree,
            tree_truncated: truncated,
            entries,
            local_only,
            local_only_truncated,
            recent,
        })
    }

    fn work_versions(&self, work: &str, agent: &str, path: &str) -> Result<VersionView, String> {
        let rel = clean_rel(path)?;
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (shared, store, sandbox) = self.agent_roots(work, agent)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        let (_, tree) = self.head_tree(&store)?;
        let index: Index = self.store.read_index(&store, agent)?;
        let base = match index.get(&rel) {
            Some(hash) => Some(object_version(self.store.as_ref(), &store, hash)?),
            None => None,
        };
        let head = match tree.get(&rel) {
            Some(hash) => Some(object_version(self.store.as_ref(), &store, hash)?),
            None => None,
        };
        let local = self
            .store
            .read_under(&sandbox, &rel)?
            .map(|bytes| FileVersion::from_bytes(content_hash(&bytes), &bytes));
        if base.is_none() && head.is_none() && local.is_none() {
            return Err(format!("基线、主副本与沙箱都没有这个文件：{}", rel));
        }
        Ok(VersionView {
            path: rel,
            base,
            head,
            local,
        })
    }

    fn work_resolve(
        &self,
        work: &str,
        agent: &str,
        req: &ResolveRequest,
    ) -> Result<ResolveReport, String> {
        if req.paths.is_empty() {
            return Err("work_resolve 要给出要解决的路径（文件或文件夹）".to_string());
        }
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (shared, store, sandbox) = self.agent_roots(work, agent)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        let (_, tree) = self.head_tree(&store)?;
        let mut index: Index = self.store.read_index(&store, agent)?;
        let sandbox_files: BTreeSet<String> = self.store.list(&sandbox)?.into_iter().collect();
        let available: BTreeSet<String> = index
            .keys()
            .chain(tree.keys())
            .chain(sandbox_files.iter())
            .cloned()
            .collect();
        let (want, missing) = expand_paths(&req.paths, &available);
        if want.is_empty() {
            return Err(format!(
                "这些路径在基线、主副本与沙箱里都没有：{}",
                missing.join("、")
            ));
        }
        let mut entries: Vec<ResolveEntry> = Vec::new();
        let mut changed = false;
        for rel in want {
            let head_hash = tree.get(&rel).cloned();
            let base_hash = index.get(&rel).cloned();
            let local = self.store.read_under(&sandbox, &rel)?;
            let action = match req.take {
                ResolveTake::Upstream => match head_hash {
                    Some(hash) => {
                        let head_bytes = self.store.read_object(&store, &hash)?;
                        let same = local.as_deref() == Some(head_bytes.as_slice());
                        if !same {
                            self.store.write_under(&sandbox, &rel, &head_bytes)?;
                        }
                        let base_changed = base_hash.as_deref() != Some(hash.as_str());
                        if base_changed {
                            index.insert(rel.clone(), hash);
                            changed = true;
                        }
                        if !same {
                            ResolveAction::WroteUpstream
                        } else if base_changed {
                            ResolveAction::Marked
                        } else {
                            ResolveAction::NoChange
                        }
                    }
                    None => {
                        let had_local = local.is_some();
                        if had_local {
                            self.store.remove_under(&sandbox, &rel)?;
                        }
                        let had_base = index.remove(&rel).is_some();
                        changed |= had_local || had_base;
                        if had_local {
                            ResolveAction::RemovedLocal
                        } else if had_base {
                            ResolveAction::Cleared
                        } else {
                            ResolveAction::NoChange
                        }
                    }
                },
                ResolveTake::Local => match head_hash {
                    Some(hash) => {
                        let base_changed = base_hash.as_deref() != Some(hash.as_str());
                        if base_changed {
                            index.insert(rel.clone(), hash);
                            changed = true;
                            ResolveAction::Marked
                        } else {
                            ResolveAction::NoChange
                        }
                    }
                    None => {
                        let had_base = index.remove(&rel).is_some();
                        changed |= had_base;
                        if had_base {
                            ResolveAction::Cleared
                        } else {
                            ResolveAction::NoChange
                        }
                    }
                },
            };
            entries.push(ResolveEntry { path: rel, action });
        }
        if changed {
            self.store.write_index(&store, agent, &index)?;
        }
        Ok(ResolveReport {
            take: req.take,
            entries,
            missing,
        })
    }

    fn work_recover(&self, work: &str) -> Result<(), String> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)
    }

    fn work_gc(&self, work: &str, apply: bool) -> Result<GcReport, String> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;

        // 可达 = 所有现存提交树的 hash + 所有 agent 基线的 hash；只被基线引用的对象也必须保留。
        let mut reachable: BTreeSet<String> = BTreeSet::new();
        for id in self.store.list_commits(&store)? {
            if let Some(commit) = self.store.read_commit(&store, id)? {
                reachable.extend(commit.tree.values().cloned());
            }
        }
        for (_, index) in self.store.list_indexes(&store)? {
            reachable.extend(index.values().cloned());
        }

        let mut report = GcReport {
            applied: apply,
            ..GcReport::default()
        };
        for object in self.store.list_objects(&store)? {
            if !is_content_hash(&object.hash) {
                report.ignored_files += 1;
                if apply {
                    self.store
                        .remove_object(&store, &object.hash)
                        .map_err(|e| format!("清临时文件 {} 失败：{}", object.hash, e))?;
                    report.removed_ignored += 1;
                }
                continue;
            }
            report.objects += 1;
            report.bytes += object.size;
            if reachable.contains(&object.hash) {
                report.reachable_objects += 1;
                report.reachable_bytes += object.size;
            } else {
                report.unreachable_objects += 1;
                report.unreachable_bytes += object.size;
                if apply {
                    self.store
                        .remove_object(&store, &object.hash)
                        .map_err(|e| format!("回收对象 {} 失败：{}", object.hash, e))?;
                    report.removed_objects += 1;
                    report.removed_bytes += object.size;
                }
            }
        }
        Ok(report)
    }

    fn work_restore(&self, work: &str, commit: u64) -> Result<(), String> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        if self.store.read_commit(&store, commit)?.is_none() {
            return Err(format!("没有提交点 {}", commit));
        }
        let pending = Pending::Restore {
            target: Some(commit),
        };
        self.store.write_pending(&store, &pending)?;
        let result =
            WorkspaceService::materialize(self.store.as_ref(), &shared, &store, Some(commit));
        self.settle(work, &shared, &store, result)
    }

    fn work_head(&self, work: &str) -> Result<Option<u64>, String> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        self.store.head(&store)
    }

    fn work_rewind_to(
        &self,
        work: &str,
        keep_by_agent: &BTreeMap<String, u64>,
    ) -> Result<Option<u64>, String> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        // 提交按 id 升序：取"各会话都还没越过自己保留行"的最后一个提交点。
        let mut target = None;
        for id in self.store.list_commits(&store)? {
            let Some(c) = self.store.read_commit(&store, id)? else {
                continue;
            };
            let within = match &c.anchor {
                // 旧记录没有锚：保守地算它在回档点之前（不让老数据把共享区清空）。
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
        let pending = Pending::Restore { target };
        self.store.write_pending(&store, &pending)?;
        let result = WorkspaceService::materialize(self.store.as_ref(), &shared, &store, target);
        self.settle(work, &shared, &store, result)?;
        Ok(target)
    }

    fn work_restore_point(&self, work: &str, commit: Option<u64>) -> Result<(), String> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        if let Some(id) = commit {
            if self.store.read_commit(&store, id)?.is_none() {
                return Err(format!("没有提交点 {}", id));
            }
        }
        let pending = Pending::Restore { target: commit };
        self.store.write_pending(&store, &pending)?;
        let result = WorkspaceService::materialize(self.store.as_ref(), &shared, &store, commit);
        self.settle(work, &shared, &store, result)
    }

    fn work_discard_after(&self, work: &str, keep: Option<u64>) -> Result<(), String> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        if let Some(id) = keep {
            if self.store.read_commit(&store, id)?.is_none() {
                return Err(format!("没有提交点 {}", id));
            }
        }
        // 先物化到 keep、写 head，再删非祖先提交；崩溃时 head 不会指向被删提交。
        let pending = Pending::Discard { keep };
        self.store.write_pending(&store, &pending)?;
        let result = (|| {
            WorkspaceService::materialize(self.store.as_ref(), &shared, &store, keep)?;
            self.discard_commits_locked(&store, keep)
        })();
        self.settle(work, &shared, &store, result)
    }
}
