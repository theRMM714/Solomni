//! 目的：工作区能力的 service 门面——`WorkspaceService` 的字段与构造、端口持有、以及各用例族共用的底座原语。
//! 管：`Workdirs` / `WorkStore` 两个端口只在这里（R12）；gate、tree_of、pin_head、restore_impl、materialize、settle、提交号预留与待恢复事务，以及 `work_*` 工具执行者的装配。
//! 不管：具体用例（commit / sync / history / resolve / lifecycle 各子模块）、`Workspace` 的 trait 转发（facade）、落盘实现（detail）。
//! 联动：`api::Workspace` 的唯一实现在 facade.rs，逐条转发到同级子模块；组合根注入两个端口适配器。

use crate::capabilities::workspace::api::{
    Change, ChangeKind, FileVersion, MAX_ASSET_BYTES, VERSION_TEXT_LIMIT, media_hint,
};
use crate::capabilities::workspace::domain::freshness::lines_changed;
use crate::capabilities::workspace::domain::hash::{content_hash, is_content_hash};
use crate::capabilities::workspace::domain::path_filter;
use crate::capabilities::workspace::domain::workstore::{
    Commit, CommitAnchor, CommitLine, Conflict, ConflictAt, Index, LocalTakeVerdict, Pending,
    PendingIndex, PullVerdict, Ref, RefKind, Source, StatusState, Tree, Unchanged, backup_rel,
    clean_rel, expand_paths, local_take_verdict, needs_backup, plan_commit, plan_merge,
    pull_verdict, split_backup, status_of, validate_paths,
};
use crate::capabilities::workspace::ports::{WorkStore, Workdirs};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

mod commit;
mod diff;
mod facade;
mod history;
mod ignore;
mod lifecycle;
mod reset;
mod resolve;
pub(crate) mod roots;
mod sync;
pub(crate) mod work_tools;

/// 首次导入外部工作区的文件数 / 字节上限（超出只导入可导入的部分，不拖垮建会话）。
const MAX_IMPORT_FILES: usize = 2000;
const MAX_IMPORT_BYTES: u64 = 256 * 1024 * 1024;
/// 每多少条增量提交落一次整棵树；越大越省空间，读树回溯越久。
const ROOT_INTERVAL: u32 = 256;

/// 目的：这条提交要不要落整棵树；要落就从基线树正放 changes 得到它，不落 = `None`。
/// 约束：只在 `depth % ROOT_INTERVAL == 0` 时落根，其余提交只记 changes。
fn deltas_tree(base: &Tree, changes: &[Change], depth: u32) -> Option<Tree> {
    if !depth.is_multiple_of(ROOT_INTERVAL) {
        return None;
    }
    let mut tree = base.clone();
    for ch in changes {
        match ch.kind {
            ChangeKind::Add | ChangeKind::Modify => {
                tree.insert(ch.path.clone(), ch.hash.clone());
            }
            ChangeKind::Delete => {
                tree.remove(&ch.path);
            }
        }
    }
    Some(tree)
}

/// 一次 status 里主副本路径清单的上限（超出如实标注截断，不假装列全）。
const MAX_STATUS_TREE: usize = 200;
/// status 里附带的最近提交条数。
const RECENT_COMMITS: usize = 5;
/// work_lost 一次最多列出的备份数（超出如实标注截断）。
const MAX_LOST: usize = 50;
/// 提交历史一页的上限（界面分页；不让一次读把整段历史塞进内存）。
const MAX_LOG_LIMIT: usize = 200;
/// 一条提交详情里最多列出的改动条数（超出如实标注截断）。
const MAX_COMMIT_CHANGES: usize = 500;

/// 目的：工作区能力——持四个端口，按用例答话。
pub struct WorkspaceService {
    dirs: Arc<dyn Workdirs + Send + Sync>,
    store: Arc<dyn WorkStore + Send + Sync>,
    /// 单文件进入共享区的上限（生产用 `MAX_ASSET_BYTES`；测试可注入小值）。
    max_asset_bytes: u64,
    /// 版本库写入的串行闸（**每个工作一把**）：不同工作互不相干、可并行；同一个工作的
    /// 提交 / 拉取 / 回档严格串行。它保护的是**版本库状态**（head、基线、对象），不是核心状态。
    version_gates: Mutex<BTreeMap<String, Arc<Mutex<()>>>>,
}

impl WorkspaceService {
    /// 目的：组合根专用构造——单文件上限取生产默认值。
    pub fn new(
        dirs: Arc<dyn Workdirs + Send + Sync>,
        store: Arc<dyn WorkStore + Send + Sync>,
    ) -> WorkspaceService {
        WorkspaceService::with_max_asset_bytes(dirs, store, MAX_ASSET_BYTES)
    }

    /// 目的：用指定的单文件上限装配；生产走 `new`，测试用小值钉超限分支。
    pub fn with_max_asset_bytes(
        dirs: Arc<dyn Workdirs + Send + Sync>,
        store: Arc<dyn WorkStore + Send + Sync>,
        max_asset_bytes: u64,
    ) -> WorkspaceService {
        WorkspaceService {
            dirs,
            store,
            max_asset_bytes,
            version_gates: Mutex::new(BTreeMap::new()),
        }
    }

    /// 一个工作的（主副本根, 版本库目录）。
    fn work_roots(&self, work: &str) -> Result<(PathBuf, PathBuf), String> {
        let roots = self.dirs.roots(work, &[])?;
        Ok((roots.shared, roots.store))
    }

    /// 目的：一个工作的内容面过滤规则原文（版本库目录里的 `.workignore`）。
    /// 约束：文件缺失时退回种子——未准备 / 旧会话与新建会话看到同一份名单，不因缺文件而放开过滤。
    fn work_rules(&self, store: &Path) -> Result<String, String> {
        Ok(self
            .store
            .read_text(&store.join(path_filter::WORKIGNORE))?
            .unwrap_or_else(|| path_filter::WORKIGNORE_SEED.to_string()))
    }

    /// 目的：一个沙箱里「算工作区内容」的文件清单（与导入用同一份 `.workignore` 剪枝）。
    /// 约束：规则只管发现；调用方点名的路径仍可单独读写。
    fn sandbox_files(&self, store: &Path, sandbox: &Path) -> Result<Vec<String>, String> {
        let rules = self.work_rules(store)?;
        self.store.list_visible(sandbox, Some(&rules))
    }

    /// 目的：把这次要落的路径展开成文件集合：可见清单说了算，但**显式点名的单个文件**照样算。
    /// 约束：规则只管发现——被 `.workignore` 盖住的文件，点名了仍然生效（同 git add -f）；
    ///   点名一个被盖住的文件夹不展开（与 git add 目录一致，文件夹属于发现）。
    fn expand_forced(
        &self,
        sandbox: &Path,
        visible: &BTreeSet<String>,
        paths: &[String],
    ) -> Result<(Vec<String>, Vec<String>), String> {
        let mut files = visible.clone();
        let named_but_hidden = paths.iter().any(|raw| {
            let rel = raw.trim_matches('/');
            !files.contains(rel) && !files.iter().any(|p| p.starts_with(&format!("{}/", rel)))
        });
        if named_but_hidden {
            let all = self.store.list(sandbox)?;
            for raw in paths {
                let rel = raw.trim_matches('/');
                if !files.contains(rel) && all.iter().any(|p| p == rel) {
                    files.insert(rel.to_string());
                }
            }
        }
        Ok(expand_paths(paths, &files))
    }

    /// 目的：工作区磁盘上一条相对路径的实际内容指纹（不存在 = None）。
    /// 约束：只给外部工作区用；符号链接逃逸 / 目录之类读不了就如实报错。
    fn disk_hash(&self, shared: &Path, rel: &str) -> Result<Option<String>, String> {
        Ok(self.store.scan_under(shared, rel, None)?.map(|v| v.hash))
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

    /// 目的：把一条提交还原成整棵树：根提交自带树，增量提交沿 parent 回溯后按 changes 正放。
    /// 约束：旧格式提交（带整棵树）天然是根；回溯遇到断链或过深一律报错，不猜。
    fn tree_of(store_port: &dyn WorkStore, store: &Path, id: u64) -> Result<Tree, String> {
        let mut deltas: Vec<Commit> = Vec::new();
        let mut cur = id;
        let base = loop {
            let c = store_port
                .read_commit(store, cur)?
                .ok_or_else(|| format!("没有提交点 {}", cur))?;
            if let Some(t) = c.tree.clone() {
                break t;
            }
            let parent = c
                .parent
                .ok_or_else(|| format!("提交 {} 既没有树也没有父提交", cur))?;
            deltas.push(c);
            cur = parent;
            if deltas.len() > 100_000 {
                return Err(format!("提交 {} 的回溯链过深", id));
            }
        };
        let mut tree = base;
        for c in deltas.iter().rev() {
            for ch in &c.changes {
                match ch.kind {
                    ChangeKind::Add | ChangeKind::Modify => {
                        tree.insert(ch.path.clone(), ch.hash.clone());
                    }
                    ChangeKind::Delete => {
                        tree.remove(&ch.path);
                    }
                }
            }
        }
        Ok(tree)
    }

    /// 目的：新提交在树重建链上的深度（父提交 depth + 1；根 = 0）。
    fn next_depth(&self, store: &Path, parent: Option<u64>) -> Result<u32, String> {
        match parent {
            Some(p) => Ok(self
                .store
                .read_commit(store, p)?
                .ok_or_else(|| format!("没有提交点 {}", p))?
                .depth
                .saturating_add(1)),
            None => Ok(0),
        }
    }

    /// 目的：把当前 head 钉成 `refs/live/<id>`——回档 / 切枝之后旧线仍可达，GC 不回收。
    /// 约束：只在移动 head 之前调用；已钉过 = 无操作。
    fn pin_head(&self, store: &Path) -> Result<(), String> {
        let Some(id) = self.store.head(store)? else {
            return Ok(());
        };
        let name = format!("live/{}", id);
        if self.store.read_ref(store, &name)?.is_some() {
            return Ok(());
        }
        self.store.write_ref(
            store,
            &name,
            &Ref {
                kind: RefKind::Live,
                target: Some(id),
                heads: BTreeMap::new(),
                saves: BTreeMap::new(),
                since: 0,
            },
        )
    }

    /// 目的：回滚到某个提交点的唯一实现；`force` = 用户已做危险确认，允许覆盖外部改动。
    fn restore_impl(&self, work: &str, commit: u64, force: bool) -> Result<(), String> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let _shared = self.dirs.lock_shared(work)?;
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        if self.store.read_commit(&store, commit)?.is_none() {
            return Err(format!("没有提交点 {}", commit));
        }
        let conflicts = self.external_materialize_conflicts(work, &shared, &store, Some(commit))?;
        if !force && !conflicts.is_empty() {
            return Err(format!(
                "回档会覆盖工作区里被外部改动的内容：{}——先 work_sync 采纳外部版本；确认覆盖后重试。",
                conflicts.join("、")
            ));
        }
        self.pin_head(&store)?;
        let pending = Pending::Restore {
            target: Some(commit),
        };
        self.store.write_pending(&store, &pending)?;
        let result =
            WorkspaceService::materialize(self.store.as_ref(), &shared, &store, Some(commit));
        self.settle(work, &shared, &store, result)
    }

    /// 目的：把共享区物化到某个提交点（None = 清空），head 同步。
    /// 约束：删除域只取**当前 head 树**里目标树没有的路径——工作区里从未受管的文件永不自动删。
    fn materialize(
        store_port: &dyn WorkStore,
        shared: &std::path::Path,
        store: &std::path::Path,
        target: Option<u64>,
    ) -> Result<(), String> {
        let tree = match target {
            Some(id) => WorkspaceService::tree_of(store_port, store, id)?,
            None => Tree::new(),
        };
        let current = match store_port.head(store)? {
            Some(id) => WorkspaceService::tree_of(store_port, store, id)?,
            None => Tree::new(),
        };
        for rel in current.keys() {
            if !tree.contains_key(rel) {
                store_port.remove_under(shared, rel)?;
            }
        }
        for (rel, hash) in &tree {
            store_port.copy_object_to_under(store, hash, shared, rel)?;
        }
        match target {
            Some(id) => store_port.set_head(store, id)?,
            None => store_port.clear_head(store)?,
        }
        Ok(())
    }

    /// 目的：回档物化前核对工作区磁盘：会覆盖 / 删除被外部改过内容的路径整次拒绝。
    /// 约束：只在外部工作区生效；强制覆盖需要用户危险确认（界面另行接入）。
    fn external_materialize_conflicts(
        &self,
        work: &str,
        shared: &Path,
        store: &Path,
        target: Option<u64>,
    ) -> Result<Vec<String>, String> {
        if !self.dirs.is_external(work) {
            return Ok(Vec::new());
        }
        let current = match self.store.head(store)? {
            Some(id) => Self::tree_of(self.store.as_ref(), store, id)?,
            None => Tree::new(),
        };
        let target_tree = match target {
            Some(id) => Self::tree_of(self.store.as_ref(), store, id)?,
            None => Tree::new(),
        };
        let mut conflicts: BTreeSet<String> = BTreeSet::new();
        for (rel, head) in &current {
            if target_tree.contains_key(rel) {
                continue;
            }
            let disk = self.disk_hash(shared, rel)?;
            if disk.is_some() && disk.as_deref() != Some(head.as_str()) {
                conflicts.insert(rel.clone());
            }
        }
        for rel in target_tree.keys() {
            let expected = current.get(rel).map(String::as_str);
            let disk = self.disk_hash(shared, rel)?;
            if disk.as_deref() != expected {
                conflicts.insert(rel.clone());
            }
        }
        Ok(conflicts.into_iter().collect())
    }

    /// 目的：提交前核对工作区磁盘与 head：外部改过的路径整次提交拒绝，绝不静默覆盖。
    /// 约束：只对绑定了外部工作区的会话调用；删除型改动在磁盘已不存在时视为已达成。
    fn reject_external_changes(
        &self,
        shared: &Path,
        tree: &Tree,
        locals: &BTreeMap<String, Option<String>>,
    ) -> Result<(), String> {
        let mut changed: Vec<String> = Vec::new();
        for (rel, local) in locals {
            let disk = self.disk_hash(shared, rel)?;
            let head = tree.get(rel).map(String::as_str);
            let ok = match (local, head) {
                (None, Some(h)) => disk.is_none() || disk.as_deref() == Some(h),
                (None, None) => disk.is_none(),
                (Some(_), Some(h)) => disk.as_deref() == Some(h),
                (Some(_), None) => disk.is_none(),
            };
            if !ok {
                changed.push(rel.clone());
            }
        }
        if changed.is_empty() {
            return Ok(());
        }
        Err(format!(
            "工作区被外部改动，提交已拒绝：{}——先用 work_sync 采纳工作区当前版本，再重试。",
            changed.join("、")
        ))
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

    /// 目的：把一条待恢复提交幂等地应用到它该去的地方——主线写共享区并推进 head，存档线只写提交与存档线头。
    /// 约束：调用方已持有工作锁与跨进程锁；不删除 pending（由恢复方或收尾方删）。
    ///   存档线是沙箱快照，**绝不碰共享区、`refs/main` 与「从主副本拉过什么」的基线**。
    fn apply_commit_pending(
        &self,
        work: &str,
        shared: &Path,
        store: &Path,
        commit: &Commit,
        index: Option<&PendingIndex>,
        line: CommitLine,
    ) -> Result<(), String> {
        match line {
            CommitLine::Main => {
                if let Some(head) = self.store.head(store)?
                    && head > commit.id
                {
                    return Err(format!(
                        "待恢复提交 {} 小于当前 head {}，拒绝倒推共享区",
                        commit.id, head
                    ));
                }
                for ch in &commit.changes {
                    match ch.kind {
                        ChangeKind::Add | ChangeKind::Modify => {
                            if commit.author == "user" {
                                let bytes = self.store.read_object(store, &ch.hash)?;
                                self.dirs.write_work(work, &ch.path, &bytes)?;
                            } else {
                                self.store
                                    .copy_object_to_under(store, &ch.hash, shared, &ch.path)?;
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
                // 发布成功：发布者的工作线快进到这条提交，它从这里开始与主线一致。
                // 约束：只有工具侧发布（`work_push`）才有发布者；外部同步与用户投喂不是 agent，不给它们建工作线。
                if commit.source == Source::Tool {
                    self.fast_forward_work(store, &commit.author, Some(commit.id))?;
                }
            }
            // 工作线：只写提交并推进自己的头；共享区、`refs/main` 与拉取基线都不动。
            CommitLine::Work => {
                self.store.write_commit(store, commit)?;
                self.fast_forward_work(store, &commit.author, Some(commit.id))?;
            }
            CommitLine::Save => {
                self.store.write_commit(store, commit)?;
                self.advance_save_line(store, &commit.author, commit.id)?;
            }
        }
        Ok(())
    }

    /// 目的：删除主线 keep 祖先链与各 agent 存档点祖先链之外的提交记录（都为空时全部删）。
    /// 约束：调用方已先物化到 keep，head 不再指向被删的提交；存档点是沙箱快照，必须一起留住。
    fn discard_commits_locked(
        &self,
        store: &Path,
        keep: Option<u64>,
        saves: &BTreeMap<String, Option<u64>>,
    ) -> Result<(), String> {
        let mut keep_set: BTreeSet<u64> = BTreeSet::new();
        let mut roots: Vec<Option<u64>> = vec![keep];
        roots.extend(saves.values().copied());
        for root in roots {
            let mut cur = root;
            while let Some(id) = cur {
                if !keep_set.insert(id) {
                    break;
                }
                cur = self.store.read_commit(store, id)?.and_then(|c| c.parent);
            }
        }
        for id in self.store.list_commits(store)? {
            if !keep_set.contains(&id) {
                self.store.remove_commit(store, id)?;
            }
        }
        // 指向已删提交的引用一并清掉，免得引用的活集合里留下悬空目标。
        for (name, r) in self.store.list_refs(store)? {
            if let Some(t) = r.target
                && self.store.read_commit(store, t)?.is_none()
            {
                self.store.remove_ref(store, &name)?;
            }
        }
        Ok(())
    }

    /// 目的：丢掉 since 及之后产生的全部提交（恢复留档时用：留档之后长出的那条分叉一并作废）。
    /// 约束：调用方已物化到留档目标并写回 head/工作线头；指向被删提交的引用（含更晚的档案）一起清掉。
    fn discard_since_locked(&self, store: &Path, since: u64) -> Result<(), String> {
        for id in self.store.list_commits(store)? {
            if id >= since {
                self.store.remove_commit(store, id)?;
            }
        }
        for (name, r) in self.store.list_refs(store)? {
            if self.ref_mentions_missing(store, &r)? {
                self.store.remove_ref(store, &name)?;
            }
        }
        Ok(())
    }

    /// 目的：这条引用的目标（含各 agent 的工作线头与存档点）是否指向已不存在的提交。
    fn ref_mentions_missing(&self, store: &Path, r: &Ref) -> Result<bool, String> {
        let mut ids: Vec<Option<u64>> = vec![r.target];
        ids.extend(r.heads.values().copied());
        ids.extend(r.saves.values().copied());
        for id in ids.into_iter().flatten() {
            if self.store.read_commit(store, id)?.is_none() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// 目的：重放该工作尚未完成的版本库事务；没有 = 无操作。
    /// 约束：调用方已持有工作锁与跨进程锁。
    fn recover_locked(&self, work: &str, shared: &Path, store: &Path) -> Result<(), String> {
        let Some(pending) = self.store.read_pending(store)? else {
            return Ok(());
        };
        match &pending {
            Pending::Commit {
                commit,
                index,
                line,
            } => self.apply_commit_pending(
                work,
                shared,
                store,
                commit.as_ref(),
                index.as_ref(),
                *line,
            )?,
            Pending::Restore { target } => {
                WorkspaceService::materialize(self.store.as_ref(), shared, store, *target)?
            }
            Pending::Discard {
                keep,
                agents,
                saves,
            } => {
                if !agents.is_empty() {
                    let prefix = Self::delete_prefix(*keep);
                    let base = match keep {
                        Some(id) => Self::tree_of(self.store.as_ref(), store, *id)?,
                        None => Tree::new(),
                    };
                    for agent in agents {
                        let save = saves.get(agent).copied().flatten();
                        let sandbox_tree = match save {
                            Some(id) => Self::tree_of(self.store.as_ref(), store, id)?,
                            None => base.clone(),
                        };
                        let (_, _, sandbox) = self.agent_roots(work, agent)?;
                        self.lay_down_tree(store, &sandbox, &sandbox_tree, agent, Some(&prefix))?;
                        self.reset_agent_point(store, agent, *keep, save, &base)?;
                    }
                }
                WorkspaceService::materialize(self.store.as_ref(), shared, store, *keep)?;
                self.discard_commits_locked(store, *keep, saves)?;
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

    /// 目的：给"先读后改"门禁取一份快照：哪些路径受管、沙箱当前内容、以及该 agent 的基线版本。
    /// 约束：只读版本库与沙箱，不写任何东西；受管 = 主线 head 树 ∪ 该 agent 工作线 head 树。
    pub(super) fn work_gate_info(
        &self,
        work: &str,
        agent: &str,
        paths: &[String],
    ) -> Result<Vec<crate::capabilities::workspace::api::PathGateInfo>, String> {
        let (_, store, sandbox) = self.agent_roots(work, agent)?;
        let (_, main) = self.head_tree(&store)?;
        let (_, line) = self.work_tree(&store, agent)?;
        let mut out = Vec::with_capacity(paths.len());
        for raw in paths {
            let rel = clean_rel(raw)?;
            let base = line.get(&rel).or_else(|| main.get(&rel)).map(|hash| {
                crate::capabilities::workspace::api::GateFile {
                    hash: hash.clone(),
                    size: None,
                }
            });
            let current = self.store.scan_under(&sandbox, &rel, None)?.map(|v| {
                crate::capabilities::workspace::api::GateFile {
                    hash: v.hash,
                    size: Some(v.size),
                }
            });
            out.push(crate::capabilities::workspace::api::PathGateInfo {
                path: rel.clone(),
                tracked: line.contains_key(&rel) || main.contains_key(&rel),
                current,
                base,
            });
        }
        Ok(out)
    }

    /// 读 head 的整棵树；空仓库 = 空树。
    fn head_tree(&self, store: &std::path::Path) -> Result<(Option<u64>, Tree), String> {
        let Some(id) = self.store.head(store)? else {
            return Ok((None, Tree::new()));
        };
        Ok((Some(id), Self::tree_of(self.store.as_ref(), store, id)?))
    }

    /// 当前 head 的祖先链（含 head）。用户回滚离开过的提交仍留在库里，但不在这一条线上。
    fn head_ancestors(
        &self,
        store: &std::path::Path,
        head: Option<u64>,
    ) -> Result<BTreeSet<u64>, String> {
        let mut set = BTreeSet::new();
        let mut cur = head;
        while let Some(id) = cur {
            if !set.insert(id) {
                break;
            }
            cur = self.store.read_commit(store, id)?.and_then(|c| c.parent);
        }
        Ok(set)
    }

    /// 目的：把一条对象的版本视图收成 `FileVersion`；已知媒体类型或超限时不读内容。
    fn version_of_object(
        &self,
        store: &Path,
        hash: &str,
        rel: &str,
    ) -> Result<FileVersion, String> {
        let size = self.store.stat_object(store, hash)?;
        let too_large = size > self.max_asset_bytes;
        let binary_hint = media_hint(rel).map(|h| h.binary).unwrap_or(false);
        if too_large || binary_hint {
            return Ok(FileVersion {
                hash: hash.to_string(),
                size,
                text: None,
                truncated: false,
                too_large,
            });
        }
        let mut version = self
            .store
            .scan_object(store, hash, Some(VERSION_TEXT_LIMIT))?;
        if version.hash != hash {
            return Err(format!("对象内容与指纹不符：{}", hash));
        }
        version.too_large = false;
        Ok(version)
    }

    /// 目的：把沙箱本地文件收成 `FileVersion`；不存在 = None。
    fn version_of_local(&self, sandbox: &Path, rel: &str) -> Result<Option<FileVersion>, String> {
        let Some(size) = self.store.stat_under(sandbox, rel)? else {
            return Ok(None);
        };
        let too_large = size > self.max_asset_bytes;
        let binary_hint = media_hint(rel).map(|h| h.binary).unwrap_or(false);
        let mut version = if too_large || binary_hint {
            self.store
                .scan_under(sandbox, rel, None)?
                .ok_or_else(|| format!("沙箱里没有：{}", rel))?
        } else {
            self.store
                .scan_under(sandbox, rel, Some(VERSION_TEXT_LIMIT))?
                .ok_or_else(|| format!("沙箱里没有：{}", rel))?
        };
        version.too_large = too_large;
        Ok(Some(version))
    }
}

/// 冲突清单渲染成模型侧回执（核心自有工具的文案，与代理工具同一口径）。
fn render_conflicts(conflicts: &[ConflictAt]) -> String {
    let mut out = String::from("提交被拒：以下路径有冲突（不做自动合并）：");
    for c in conflicts {
        out.push_str(&format!("\n- {}（{}）", c.path, c.kind.reason()));
    }
    out.push_str(
        "\n先用 work_versions 看基线 / 主副本 / 沙箱三方：文本可人工合并后用 work_resolve（take=local）声明已解决，要放弃本地改动用 take=upstream；\n图片 / 视频 / 二进制不能合并，只能选边（take=upstream 取主副本，take=local 保留本地），不做自动合并。",
    );
    out
}

/// 推送冲突渲染成模型侧回执：冲突是「我提交之后主线也动过」，不同类型的补救路径不同——
/// 删除型冲突（一边删、另一边改）不能靠"合进沙箱再提交"收敛，必须选边或撤回那条提交。
fn render_push_conflicts(conflicts: &[ConflictAt]) -> String {
    let mut out = String::from("推送被拒：以下路径有冲突——你提交之后主线也动过（不做自动合并）：");
    for c in conflicts {
        out.push_str(&format!("\n- {}（{}）", c.path, c.kind.reason()));
    }
    let deleted_by_me: Vec<&ConflictAt> = conflicts
        .iter()
        .filter(|c| c.kind == Conflict::DeleteVsModify)
        .collect();
    let deleted_upstream: Vec<&ConflictAt> = conflicts
        .iter()
        .filter(|c| c.kind == Conflict::ModifyVsDelete)
        .collect();
    let both: Vec<&ConflictAt> = conflicts
        .iter()
        .filter(|c| !matches!(c.kind, Conflict::DeleteVsModify | Conflict::ModifyVsDelete))
        .collect();
    if !deleted_by_me.is_empty() {
        let names = push_conflict_paths(&deleted_by_me);
        out.push_str(&format!(
            "\n\n本地删了、主线改了（{}）：删除不会覆盖主线已经改过的内容，work_pull 遇到这类冲突也不写文件，重新提交只会再被拒一次。二选一：\n1. 放弃删除、接受主线版本：work_resolve take=upstream paths=[{}] → work_commit paths=[{}] → work_push（内容回到主线版本，净改动为零）；\n2. 撤回那条删除提交：work_log 找到它的父提交，work_reset to=<父提交>，再 work_pull 取回主线版本，然后 work_push。",
            names, names, names
        ));
    }
    if !deleted_upstream.is_empty() {
        let names = push_conflict_paths(&deleted_upstream);
        out.push_str(&format!(
            "\n\n主线删了、你改了（{}）：修改不会让主线已经删掉的文件复活。二选一：\n1. 接受删除：work_resolve take=upstream paths=[{}]（沙箱里的本地版本会先备份进 lost/）→ work_commit deletes=[{}] → work_push（净改动为零）；\n2. 撤回那条修改提交：work_log 找到它的父提交，work_reset to=<父提交>，再 work_push。",
            names, names, names
        ));
    }
    if !both.is_empty() {
        out.push_str(&format!(
            "\n\n两边都改了、内容不同（{}）：先用 work_versions 看基线 / 主副本 / 沙箱三方，把主线版本合进沙箱（共享区同一相对路径就是主副本全文），再用 work_resolve take=local head=<主线提交> 登记已解决，最后 work_commit 与 work_push。",
            push_conflict_paths(&both)
        ));
    }
    out.push_str("\n\n你的工作线提交不会因为这次拒绝丢失。");
    out
}

/// 目的：把一组冲突路径写成 `paths=[…]` 里能直接用的形式（最多 20 个，其余标总数）。
fn push_conflict_paths(conflicts: &[&ConflictAt]) -> String {
    let names: Vec<&str> = conflicts.iter().take(20).map(|c| c.path.as_str()).collect();
    let mut joined = names.join("、");
    if conflicts.len() > 20 {
        joined.push_str(&format!("…（共 {} 条）", conflicts.len()));
    }
    joined
}

/// 空提交的原因渲染成模型侧回执：逐条说清是“一致”“上游已更新”还是“上游已删除”，
/// 并给下一步（不再笼统说“与主副本一致”）。
fn render_unchanged(items: &[(String, Unchanged)]) -> String {
    let mut out = String::from("没有可提交的改动：");
    for (path, why) in items.iter().take(50) {
        out.push_str(&format!("\n- {}：{}", path, why.as_str()));
    }
    if items.len() > 50 {
        out.push_str(&format!("\n…（共 {} 条）", items.len()));
    }
    if items
        .iter()
        .any(|(_, why)| matches!(why, Unchanged::UpstreamDeleted))
    {
        out.push_str(
            "\n上游已删除的路径：要保留本地内容为新增，先 work_resolve take=local（带当前 head），再 work_commit；要放弃，用 take=upstream。",
        );
    }
    if items
        .iter()
        .any(|(_, why)| matches!(why, Unchanged::UpstreamMoved))
    {
        out.push_str("\n上游已更新的路径：先 work_pull 把新版拉进沙箱，再提交。");
    }
    out.push_str("\n先用 work_status / work_versions 看逐条状态，再决定下一步。");
    out
}

/// 把超限清单渲染成模型侧回执（点名路径、大小与上限，并给替代路径）。
fn render_oversize(items: &[(String, u64)], limit: u64) -> String {
    let mut out = format!(
        "提交被拒（文件过大）：共享区单文件上限 {} 字节（{} MiB）。",
        limit,
        limit / 1024 / 1024
    );
    for (path, size) in items {
        out.push_str(&format!("\n- {}：{} 字节", path, size));
    }
    out.push_str("\n共享区不做分块存储；请压缩或拆分后重试。");
    out
}

/// 目的：该 agent 的备份根 `<版本库>/lost/<agent>`。
/// 约束：agent 必须是单个、干净的路径组件；不允许用它把备份根挪到别处。
fn lost_root(store: &Path, agent: &str) -> Result<PathBuf, String> {
    if agent.is_empty()
        || agent == "."
        || agent == ".."
        || agent.contains('/')
        || agent.contains('\\')
    {
        return Err(format!("agent 名不能用作备份目录：{}", agent));
    }
    Ok(store.join("lost").join(agent))
}

/// 目的：备份目录里的临时文件（复制中断留下的）不算一份备份。
fn is_lost_temp(rel: &str) -> bool {
    rel.split('/').any(|s| s.starts_with(".tmp-"))
}
