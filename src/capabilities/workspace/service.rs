//! 工作区的**用例与端口持有者**：出站端口只在这里（R12）——
//! 清单来源、运行包库来源、目录布局、共享区版本库。别的能力要清单事实或工作区目录，走 `api::Workspace`。
//!
//! 装配（new 出适配器）在组合根；这里只收注入的端口。
//!
//! 共享区是**版本化工作区**：主副本 `work/` 只在提交时改写，agent 的沙箱是它的工作副本，
//! pull / commit 的三方比较规则在 `domain::workstore`（纯逻辑），落盘在 `WorkStore` 端口后面。

use crate::capabilities::workspace::api::{
    Change, ChangeKind, CommitReport, CommitRequest, CommitSummary, Library, PullReport, Roster,
    StatusEntry, StatusReport, WorkFiles, WorkRoots, WorkUsage, Workspace,
};
use crate::capabilities::workspace::domain::hash::content_hash;
use crate::capabilities::workspace::domain::workstore::{
    clean_rel, expand_paths, plan_commit, pull_verdict, status_of, Commit, ConflictAt, Index,
    PullVerdict, Tree,
};
use crate::capabilities::workspace::ports::{ModuleSource, PackageSource, WorkStore, Workdirs};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// 一次 status 里主副本路径清单的上限（超出如实标注截断，不假装列全）。
const MAX_STATUS_TREE: usize = 200;
/// status 里附带的最近提交条数。
const RECENT_COMMITS: usize = 5;

/// 工作区能力：持四个端口，按用例答话。
pub struct WorkspaceService {
    source: Arc<dyn ModuleSource + Send + Sync>,
    packages: Arc<dyn PackageSource + Send + Sync>,
    dirs: Arc<dyn Workdirs + Send + Sync>,
    store: Arc<dyn WorkStore + Send + Sync>,
    /// 版本库写入的串行闸：多个 agent 会话可能在各自的工作线程上同时提交 / 拉取。
    /// 它保护的是**版本库状态**（head、基线、对象），不是核心状态。
    version_lock: Mutex<()>,
}

impl WorkspaceService {
    /// 组合根专用。
    pub fn new(
        source: Arc<dyn ModuleSource + Send + Sync>,
        packages: Arc<dyn PackageSource + Send + Sync>,
        dirs: Arc<dyn Workdirs + Send + Sync>,
        store: Arc<dyn WorkStore + Send + Sync>,
    ) -> WorkspaceService {
        WorkspaceService {
            source,
            packages,
            dirs,
            store,
            version_lock: Mutex::new(()),
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

    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.version_lock.lock().unwrap_or_else(|e| e.into_inner())
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
    out.push_str("\n先 work_pull（拉不到的就 read 主副本对应路径）取上游版本，人工合进你的沙箱文件，再提交一次。");
    out
}

impl Workspace for WorkspaceService {
    fn roster(&self) -> Roster {
        self.source.scan()
    }

    fn library(&self) -> Library {
        self.packages.scan()
    }

    fn runtimes_dir(&self) -> std::path::PathBuf {
        self.packages.dir()
    }

    fn prepare(&self, session: &str, agents: &[String]) -> Result<(), String> {
        self.dirs.prepare(session, agents)
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
        let _g = self.lock();
        let (_, store, sandbox) = self.agent_roots(work, agent)?;
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
                PullVerdict::Conflict => report.conflicts.push(rel),
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
        let _g = self.lock();
        let (shared, store, sandbox) = self.agent_roots(work, agent)?;
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
        // 全部就绪 → 落对象、改主副本、追加提交、更新基线与 head（任一步失败即如实报错）。
        let id = self
            .store
            .list_commits(&store)?
            .last()
            .copied()
            .unwrap_or(0)
            + 1;
        let mut new_tree = tree.clone();
        for ch in &changes {
            match ch.kind {
                ChangeKind::Add | ChangeKind::Modify => {
                    let bytes = locals
                        .get(&ch.path)
                        .and_then(|v| v.as_ref())
                        .ok_or_else(|| format!("内部错误：{} 没有本地内容", ch.path))?;
                    self.store.write_object(&store, &ch.hash, bytes)?;
                    self.store.write_under(&shared, &ch.path, bytes)?;
                    new_tree.insert(ch.path.clone(), ch.hash.clone());
                }
                ChangeKind::Delete => {
                    self.store.remove_under(&shared, &ch.path)?;
                    new_tree.remove(&ch.path);
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
        };
        self.store.write_commit(&store, &commit)?;
        self.store.set_head(&store, id)?;
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
        self.store.write_index(&store, agent, &new_index)?;
        Ok(CommitReport { id, changes })
    }

    fn work_commit_user(
        &self,
        work: &str,
        path: &str,
        bytes: &[u8],
        time: i64,
    ) -> Result<u64, String> {
        let rel = clean_rel(path)?;
        if rel.contains('/') {
            return Err(format!("用户投喂只收顶层文件名：{}", rel));
        }
        let _g = self.lock();
        let (_, store) = self.work_roots(work)?;
        let (head_id, tree) = self.head_tree(&store)?;
        let hash = content_hash(bytes);
        self.store.write_object(&store, &hash, bytes)?;
        // 主副本的可见文件走目录布局端口写（与既有投喂落点一致）。
        self.dirs.write_work(work, &rel, bytes)?;
        let kind = if tree.contains_key(&rel) {
            ChangeKind::Modify
        } else {
            ChangeKind::Add
        };
        let id = self
            .store
            .list_commits(&store)?
            .last()
            .copied()
            .unwrap_or(0)
            + 1;
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
                path: rel,
                kind,
                hash,
            }],
            tree: new_tree,
        };
        self.store.write_commit(&store, &commit)?;
        self.store.set_head(&store, id)?;
        Ok(id)
    }

    fn work_status(
        &self,
        work: &str,
        agent: &str,
        paths: &[String],
    ) -> Result<StatusReport, String> {
        let (_, store, sandbox) = self.agent_roots(work, agent)?;
        let (head_id, tree) = self.head_tree(&store)?;
        let head = match head_id {
            Some(id) => self
                .store
                .read_commit(&store, id)?
                .map(|c| CommitSummary::from(&c)),
            None => None,
        };
        let index: Index = self.store.read_index(&store, agent)?;
        let all: BTreeSet<String> = index.keys().chain(tree.keys()).cloned().collect();
        let want: Vec<String> = if paths.is_empty() {
            all.iter().cloned().collect()
        } else {
            let (got, _) = expand_paths(paths, &all);
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
            recent,
        })
    }

    fn work_restore(&self, work: &str, commit: u64) -> Result<(), String> {
        let _g = self.lock();
        let (shared, store) = self.work_roots(work)?;
        let target = self
            .store
            .read_commit(&store, commit)?
            .ok_or_else(|| format!("没有提交点 {}", commit))?;
        // 先删主副本里不在目标树里的文件，再逐个写回。
        for rel in self.store.list(&shared)? {
            if !target.tree.contains_key(&rel) {
                self.store.remove_under(&shared, &rel)?;
            }
        }
        for (rel, hash) in &target.tree {
            let bytes = self.store.read_object(&store, hash)?;
            self.store.write_under(&shared, rel, &bytes)?;
        }
        self.store.set_head(&store, commit)?;
        Ok(())
    }
}
