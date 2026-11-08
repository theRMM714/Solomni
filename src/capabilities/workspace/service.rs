//! 工作区的**用例与端口持有者**：出站端口只在这里（R12）——
//! 目录布局、共享区版本库。别的能力要工作区目录与版本化，走 `api::Workspace`。
//!
//! 装配（new 出适配器）在组合根；这里只收注入的端口。
//!
//! 共享区是**版本化工作区**：主副本 `work/` 只在提交时改写，agent 的沙箱是它的工作副本，
//! pull / commit 的三方比较与 resolve 的选边规则在 `domain::workstore`（纯逻辑），落盘在 `WorkStore` 端口后面。

use crate::capabilities::workspace::api::{
    media_hint, AdoptReport, ArchiveReport, Change, ChangeKind, ChangeView, CommitDetail,
    CommitFileView, CommitReport, CommitRequest, CommitSummary, FileVersion, GcReport, HeldChange,
    LostEntry, LostReport, PullReport, ReadQuery, ResolveAction, ResolveEntry, ResolveReport,
    ResolveRequest, ResolveTake, ResolveWarning, ResolveWarningKind, RestoreLostReport,
    StatusEntry, StatusReport, SyncReport, VersionView, WorkFiles, WorkLog, WorkLogEntry,
    WorkRoots, WorkUsage, Workspace, MAX_ASSET_BYTES, VERSION_TEXT_LIMIT,
};
use crate::capabilities::workspace::domain::base::{stale_paths, BaseRef, CommitRefusal};
use crate::capabilities::workspace::domain::freshness::lines_changed;
use crate::capabilities::workspace::domain::hash::{content_hash, is_content_hash};
use crate::capabilities::workspace::domain::workstore::{
    clean_rel, expand_paths, local_take_verdict, lost_rel, needs_backup, plan_commit, pull_verdict,
    status_of, validate_paths, Commit, CommitAnchor, Conflict, ConflictAt, Index, LocalTakeVerdict,
    Pending, PendingIndex, PullVerdict, Ref, RefKind, Source, StatusState, Tree, Unchanged,
};
use crate::capabilities::workspace::ports::{WorkStore, Workdirs};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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

/// 首次导入时忽略的目录（VCS 元数据与依赖缓存；用户仍可之后显式 work_sync）。
fn import_ignored(rel: &str) -> bool {
    rel.split('/').any(|seg| {
        matches!(
            seg,
            ".git" | ".svn" | ".hg" | "node_modules" | "target" | "__pycache__" | ".venv" | "venv"
        )
    })
}

/// 一次 status 里主副本路径清单的上限（超出如实标注截断，不假装列全）。
const MAX_STATUS_TREE: usize = 200;
/// status 里附带的最近提交条数。
const RECENT_COMMITS: usize = 5;
/// status 里列出的“沙箱未跟踪文件”上限（超出如实标注截断）。
const MAX_LOCAL_ONLY: usize = 50;
/// work_lost 一次最多列出的备份数（超出如实标注截断）。
const MAX_LOST: usize = 50;
/// 提交历史一页的上限（界面分页；不让一次读把整段历史塞进内存）。
const MAX_LOG_LIMIT: usize = 200;
/// 一条提交详情里最多列出的改动条数（超出如实标注截断）。
const MAX_COMMIT_CHANGES: usize = 500;

/// 工作区能力：持四个端口，按用例答话。
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
    /// 组合根专用：单文件上限取生产默认值。
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
        // 指向已删提交的引用一并清掉，免得引用的活集合里留下悬空目标。
        for (name, r) in self.store.list_refs(store)? {
            if let Some(t) = r.target {
                if self.store.read_commit(store, t)?.is_none() {
                    self.store.remove_ref(store, &name)?;
                }
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

/// 把 take=local 的拒绝清单渲染成模型侧回执：整次不做，逐条点名。
fn render_resolve_denied(paths: &[String]) -> String {
    let mut out = String::from(
        "解决被拒：以下路径的本地内容与基线一致（你没有改过它），take=local 不会包含主副本的新改动：",
    );
    for path in paths.iter().take(50) {
        out.push_str(&format!("\n- {}", path));
    }
    if paths.len() > 50 {
        out.push_str(&format!("\n…（共 {} 条）", paths.len()));
    }
    out.push_str(
        "\n先用 work_versions 看基线 / 主副本 / 沙箱三方：要接受主副本用 take=upstream；要保留本地，请先把主副本内容合进沙箱，再带 head=<主副本提交 id> 调 take=local。\n本次没有改动文件或基线。",
    );
    out
}

/// 一条待执行的 resolve 计划：先把三方看全、把要备份的备份完，再统一改文件/基线。
struct ResolvePlan {
    path: String,
    head_hash: Option<String>,
    base_hash: Option<String>,
    local_hash: Option<String>,
    /// 覆盖/删除前要写入的备份相对路径（版本库内）；不需要备份 = None。
    backup: Option<String>,
}

/// 目的：提交与基线落盘的共用实现——已持 gate / shared / store 锁的调用方复用，避免二次加锁。
impl WorkspaceService {
    /// 目的：提交的唯一实现（调用方已持 gate、shared、store 锁并已恢复事务）。
    /// 约束：不做基线准入——自动提交的准入由 `work_commit_auto` 在调用前做。
    #[allow(clippy::too_many_arguments)]
    fn commit_locked(
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
    fn agent_bases(&self, store: &Path, agent: &str) -> Result<BTreeMap<String, u64>, String> {
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
    fn write_agent_bases(
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
    fn record_committed_bases(
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
    fn commit_paths(&self, sandbox: &Path, req: &CommitRequest) -> Result<Vec<String>, String> {
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
    fn archive_point(
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
    fn lay_down_tree(
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
    fn reset_agent_point(
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
}

impl Workspace for WorkspaceService {
    fn bind(&self, session: &str, root: Option<&Path>) -> Result<(), String> {
        self.dirs.bind(session, root)
    }

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
        validate_paths(paths)?;
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let _shared = self.dirs.lock_shared(work)?;
        let (shared, store, sandbox) = self.agent_roots(work, agent)?;
        let external = self.dirs.is_external(work);
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
        let mut index_dirty = false;
        for rel in want {
            // 逐条收敛：中途失败时，已成功的路径保留在 index 里并落盘，
            // 避免“沙箱已是新版、基线还没推进”在下次上游提交后变成伪冲突。
            let step = (|| -> Result<(), String> {
                let head_hash = tree.get(&rel).cloned().unwrap_or_default();
                if external {
                    // 磁盘是外部工作区的真相：与 head 不一致时绝不拿旧对象覆盖沙箱。
                    if self.disk_hash(&shared, &rel)?.as_deref() != Some(head_hash.as_str()) {
                        report.external.push(rel.clone());
                        return Ok(());
                    }
                }
                let local = self.store.scan_under(&sandbox, &rel, None)?;
                let local_hash = local.as_ref().map(|v| v.hash.clone());
                let base = index.get(&rel).map(String::as_str);
                match pull_verdict(base, Some(&head_hash), local_hash.as_deref()) {
                    PullVerdict::Update => {
                        self.store
                            .copy_object_to_under(&store, &head_hash, &sandbox, &rel)?;
                        index.insert(rel.clone(), head_hash);
                        report.updated.push(rel.clone());
                        index_dirty = true;
                    }
                    PullVerdict::AlreadySame => {
                        index.insert(rel.clone(), head_hash);
                        report.already.push(rel.clone());
                        index_dirty = true;
                    }
                    PullVerdict::NoChange => report.no_change.push(rel.clone()),
                    PullVerdict::Occupied => report.occupied.push(rel.clone()),
                    PullVerdict::Conflict => {
                        // pull 侧能落到 Conflict 的形态：本地删了/上游改了，或两边都改了。
                        let kind = if local_hash.is_none() {
                            Conflict::DeleteVsModify
                        } else {
                            Conflict::BothModified
                        };
                        report.conflicts.push(ConflictAt {
                            path: rel.clone(),
                            kind,
                        });
                    }
                    PullVerdict::Missing => report.missing.push(rel.clone()),
                }
                Ok(())
            })();
            if let Err(e) = step {
                let written = report.updated.len() + report.already.len();
                let mut msg = format!(
                    "拉取中断：已写入/登记 {} 个路径；失败于 {}：{}",
                    written, rel, e
                );
                if index_dirty {
                    match self.store.write_index(&store, agent, &index) {
                        Ok(()) => msg.push_str("；已完成部分的基线已记录，重试同一路径即可"),
                        Err(se) => msg.push_str(&format!(
                            "；已完成部分的基线写入也失败：{}。重试 work_pull 会收敛",
                            se
                        )),
                    }
                } else {
                    msg.push_str("；还没有写入任何文件，重试同一路径即可");
                }
                return Err(msg);
            }
        }
        if let Err(e) = self.store.write_index(&store, agent, &index) {
            return Err(format!(
                "拉取已写入沙箱，但基线写入失败：{}；重试 work_pull 会收敛（已是最新的文件只更新基线）",
                e
            ));
        }
        // 逐路径基线 = 这次拉到的提交号；之后自动提交据此判快进。
        let mut bases = self.agent_bases(&store, agent)?;
        for rel in report.updated.iter().chain(report.already.iter()) {
            bases.insert(rel.clone(), head);
        }
        self.write_agent_bases(&store, agent, head, &bases)?;
        Ok(report)
    }

    fn work_commit(
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

    fn work_commit_auto(
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

    fn work_adopt_sandbox(
        &self,
        work: &str,
        agent: &str,
        session: &str,
        time: i64,
        line: u64,
    ) -> Result<AdoptReport, String> {
        // 外部工作区的真相是磁盘（work_sync 采纳）；沙箱副本不是它的权威版本。
        if self.dirs.is_external(work) {
            return Ok(AdoptReport::default());
        }
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let _shared = self.dirs.lock_shared(work)?;
        let (shared, store, sandbox) = self.agent_roots(work, agent)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        let (_, tree) = self.head_tree(&store)?;
        let index: Index = self.store.read_index(&store, agent)?;
        // 候选 = 受管路径 + 沙箱里的全部文件：新增 / 修改 / 删除都在这一遍里认出来。
        let mut all: BTreeSet<String> = index.keys().cloned().collect();
        all.extend(tree.keys().cloned());
        all.extend(self.store.list(&sandbox)?);
        let mut paths: Vec<String> = Vec::new();
        let mut deletes: Vec<String> = Vec::new();
        let mut held: Vec<HeldChange> = Vec::new();
        for rel in all {
            let local = self.store.scan_under(&sandbox, &rel, None)?;
            let local_hash = local.as_ref().map(|v| v.hash.as_str());
            let state = status_of(
                index.get(&rel).map(String::as_str),
                tree.get(&rel).map(String::as_str),
                local_hash,
            );
            match state {
                StatusState::LocalOnly | StatusState::LocalModified => {
                    // 超大文件会让整次提交被拒：单列出来，让剩下的照常进共享区。
                    let size = self.store.stat_under(&sandbox, &rel)?.unwrap_or(0);
                    if size > self.max_asset_bytes {
                        held.push(HeldChange {
                            path: rel,
                            why: format!("超过单文件上限（{} 字节）", self.max_asset_bytes),
                        });
                        continue;
                    }
                    paths.push(rel);
                }
                StatusState::LocalDeleted => deletes.push(rel),
                StatusState::BothPresent => held.push(HeldChange {
                    path: rel,
                    why: "共享区已有这个路径、你的沙箱还没有基线：先 work_pull 再改".to_string(),
                }),
                StatusState::Conflict => held.push(HeldChange {
                    path: rel,
                    why: "共享区先改过、你的沙箱也改了：先 work_pull（冲突用 work_resolve）"
                        .to_string(),
                }),
                _ => {}
            }
        }
        if paths.is_empty() && deletes.is_empty() {
            return Ok(AdoptReport {
                commit: None,
                changed: Vec::new(),
                held,
            });
        }
        let req = CommitRequest {
            paths,
            deletes,
            message: "沙箱改动（shell / 模块工具）".to_string(),
            time,
            line,
        };
        let anchor = CommitAnchor {
            agent: agent.to_string(),
            line,
        };
        let report = self.commit_locked(
            work,
            &shared,
            &store,
            &sandbox,
            agent,
            session,
            &req,
            Source::External,
            Some(anchor),
        )?;
        self.record_committed_bases(&store, agent, &report)?;
        Ok(AdoptReport {
            commit: Some(report.id),
            changed: report.changes.iter().map(|c| c.path.clone()).collect(),
            held,
        })
    }

    fn work_stale_reads(&self, work: &str, queries: &[ReadQuery]) -> Result<Vec<bool>, String> {
        if queries.is_empty() {
            return Ok(Vec::new());
        }
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (shared, store) = self.work_roots(work)?;
        let _ = shared;
        let _lock = self.store.lock(&store)?;
        let (_head, tree) = self.head_tree(&store)?;
        let mut out = Vec::with_capacity(queries.len());
        for q in queries {
            // 路径在主副本里没了 = 读过的东西已经不在了。
            let Some(now) = tree.get(&q.path) else {
                out.push(true);
                continue;
            };
            // 指纹相同 = 读的就是现在的版本（文件头插一行不会走到这里，插入会改指纹，
            // 但下面的行区间比较会把"读过的行没变"判成不过期）。
            if *now == q.blob {
                out.push(false);
                continue;
            }
            let old = self.store.read_object(&store, &q.blob).ok();
            let new = self.store.read_object(&store, now).ok();
            match (old, new) {
                (Some(old), Some(new)) => match (String::from_utf8(old), String::from_utf8(new)) {
                    (Ok(old), Ok(new)) => {
                        out.push(lines_changed(&old, &new, q.from, q.to));
                    }
                    // 非 UTF-8（二进制 / 编码坏掉）：没法按行比，按过期处理。
                    _ => out.push(true),
                },
                // 旧对象缺失（被 GC 或从未入库）：宁可供模型重读，不供奉可能错的旧内容。
                _ => out.push(true),
            }
        }
        Ok(out)
    }

    fn work_sync(&self, work: &str, paths: &[String], time: i64) -> Result<SyncReport, String> {
        if paths.is_empty() {
            return Err("work_sync 要给出要同步的路径（文件或文件夹）".to_string());
        }
        validate_paths(paths)?;
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let _shared = self.dirs.lock_shared(work)?;
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        let (head_id, tree) = self.head_tree(&store)?;
        // 文件集合 = 工作区磁盘全部文件 + 当前 head；这样“磁盘新增”和“外部删除”都能被展开。
        let disk_files: BTreeSet<String> = self.store.list(&shared)?.into_iter().collect();
        let available: BTreeSet<String> = tree
            .keys()
            .cloned()
            .chain(disk_files.iter().cloned())
            .collect();
        let (want, missing) = expand_paths(paths, &available);
        let mut changes: Vec<Change> = Vec::new();
        let mut unchanged: Vec<String> = Vec::new();
        for rel in want {
            let disk = self.disk_hash(&shared, &rel)?;
            let head = tree.get(&rel).map(String::as_str);
            match (disk.as_deref(), head) {
                (None, None) => {}
                (None, Some(_)) => {
                    changes.push(Change {
                        path: rel,
                        kind: ChangeKind::Delete,
                        hash: String::new(),
                    });
                }
                (Some(d), None) => {
                    changes.push(Change {
                        path: rel,
                        kind: ChangeKind::Add,
                        hash: d.to_string(),
                    });
                }
                (Some(d), Some(h)) if d == h => unchanged.push(rel),
                (Some(d), Some(_)) => {
                    changes.push(Change {
                        path: rel,
                        kind: ChangeKind::Modify,
                        hash: d.to_string(),
                    });
                }
            }
        }
        if changes.is_empty() {
            return Ok(SyncReport {
                commit: None,
                changes,
                unchanged,
                missing,
            });
        }
        for ch in &changes {
            if matches!(ch.kind, ChangeKind::Add | ChangeKind::Modify) {
                self.store.ingest_under(
                    &store,
                    &shared,
                    &ch.path,
                    &ch.hash,
                    self.max_asset_bytes,
                )?;
            }
        }
        let id = self.reserve_id(&store)?;
        let depth = self.next_depth(&store, head_id)?;
        let commit = Commit {
            id,
            parent: head_id,
            base: head_id.unwrap_or(0),
            author: "external".to_string(),
            session: String::new(),
            time,
            message: format!("外部工作区同步：{} 条", changes.len()),
            changes: changes.clone(),
            tree: deltas_tree(&tree, &changes, depth),
            depth,
            source: Source::External,
            anchor: None,
        };
        let pending = Pending::Commit {
            commit: Box::new(commit.clone()),
            index: None,
        };
        self.store.write_pending(&store, &pending)?;
        let result = self.apply_commit_pending(work, &shared, &store, &commit, None);
        self.settle(work, &shared, &store, result)?;
        Ok(SyncReport {
            commit: Some(id),
            changes,
            unchanged,
            missing,
        })
    }

    fn work_import_root(&self, work: &str, time: i64) -> Result<SyncReport, String> {
        if !self.dirs.is_external(work) {
            return Ok(SyncReport::default());
        }
        let (shared, store) = self.work_roots(work)?;
        if self.store.head(&store)?.is_some() {
            return Ok(SyncReport::default());
        }
        let mut paths: Vec<String> = Vec::new();
        let mut total: u64 = 0;
        for rel in self.store.list(&shared)? {
            if import_ignored(&rel) {
                continue;
            }
            let size = self.store.stat_under(&shared, &rel)?.unwrap_or(0);
            if size > self.max_asset_bytes {
                continue;
            }
            total = total.saturating_add(size);
            paths.push(rel);
            if paths.len() >= MAX_IMPORT_FILES || total >= MAX_IMPORT_BYTES {
                break;
            }
        }
        if paths.is_empty() {
            return Ok(SyncReport::default());
        }
        self.work_sync(work, &paths, time)
    }

    fn work_sync_tracked(&self, work: &str, time: i64) -> Result<SyncReport, String> {
        let (_, store) = self.work_roots(work)?;
        let (_, tree) = self.head_tree(&store)?;
        let paths: Vec<String> = tree.keys().cloned().collect();
        if paths.is_empty() {
            return Ok(SyncReport::default());
        }
        self.work_sync(work, &paths, time)
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

    fn work_status(
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

    fn work_log(&self, work: &str, limit: usize, before: Option<u64>) -> Result<WorkLog, String> {
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

    fn work_commit_view(&self, work: &str, commit: u64) -> Result<CommitDetail, String> {
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

    fn work_commit_file(
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

    fn work_versions(&self, work: &str, agent: &str, path: &str) -> Result<VersionView, String> {
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

    fn work_resolve(
        &self,
        work: &str,
        agent: &str,
        req: &ResolveRequest,
    ) -> Result<ResolveReport, String> {
        if req.paths.is_empty() {
            return Err("work_resolve 要给出要解决的路径（文件或文件夹）".to_string());
        }
        validate_paths(&req.paths)?;
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (shared, store, sandbox) = self.agent_roots(work, agent)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        let (head_id, tree) = self.head_tree(&store)?;
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
        if matches!(req.take, ResolveTake::Local) {
            if let Some(current) = head_id {
                match req.expect_head {
                    Some(expected) if expected == current => {}
                    Some(expected) => {
                        return Err(format!(
                            "主副本已从提交 {} 变为 {}：你查看后主副本又前进了，不能把旧合并结果登记到新版本。先重新 work_versions 看三方、把新版合进沙箱，再带 head={} 调 work_resolve。",
                            expected, current, current
                        ))
                    }
                    None => {
                        return Err(format!(
                            "take=local 要带 head=<主副本提交 id>（先 work_status 或 work_versions 取，当前是 {}）；主副本在你查看之后可能已变化，resolve 不能替你猜。",
                            current
                        ))
                    }
                }
            }
        }
        let mut plans: Vec<ResolvePlan> = Vec::new();
        let mut denied: Vec<String> = Vec::new();
        let mut warnings: Vec<ResolveWarning> = Vec::new();
        for rel in want {
            let head_hash = tree.get(&rel).cloned();
            let base_hash = index.get(&rel).cloned();
            let local = self.store.scan_under(&sandbox, &rel, None)?;
            let local_hash = local.as_ref().map(|v| v.hash.clone());
            let backup = if matches!(req.take, ResolveTake::Upstream)
                && needs_backup(head_hash.as_deref(), local_hash.as_deref())
            {
                Some(lost_rel(agent, &rel)?)
            } else {
                None
            };
            if matches!(req.take, ResolveTake::Local) {
                match local_take_verdict(
                    base_hash.as_deref(),
                    head_hash.as_deref(),
                    local_hash.as_deref(),
                ) {
                    LocalTakeVerdict::DenyUnchanged => denied.push(rel.clone()),
                    LocalTakeVerdict::WarnLocalMissing => warnings.push(ResolveWarning {
                        path: rel.clone(),
                        kind: ResolveWarningKind::LocalMissing,
                    }),
                    LocalTakeVerdict::WarnNoBase => warnings.push(ResolveWarning {
                        path: rel.clone(),
                        kind: ResolveWarningKind::NoBase,
                    }),
                    LocalTakeVerdict::Ok => {}
                }
            }
            plans.push(ResolvePlan {
                path: rel,
                head_hash,
                base_hash,
                local_hash,
                backup,
            });
        }
        if !denied.is_empty() {
            return Err(render_resolve_denied(&denied));
        }
        // 先把所有要覆盖/删除的本地版本备份完，再做任何改动；备份失败就整次拒绝。
        let backup_root = lost_root(&store, agent)?;
        for plan in &plans {
            if plan.backup.is_some() {
                let hash = plan
                    .local_hash
                    .as_deref()
                    .ok_or_else(|| format!("内部错误：{} 需要备份但没有本地指纹", plan.path))?;
                self.store.copy_under_to_under(
                    &sandbox,
                    &plan.path,
                    &backup_root,
                    &plan.path,
                    hash,
                )?;
            }
        }
        let mut entries: Vec<ResolveEntry> = Vec::new();
        let mut changed = false;
        let mut done = 0usize;
        let apply = (|| -> Result<(), (String, String)> {
            for plan in plans {
                let ResolvePlan {
                    path: rel,
                    head_hash,
                    base_hash,
                    local_hash,
                    backup,
                } = plan;
                let action = match req.take {
                    ResolveTake::Upstream => match head_hash.as_deref() {
                        Some(hash) => {
                            let same = local_hash.as_deref() == Some(hash);
                            if !same {
                                self.store
                                    .copy_object_to_under(&store, hash, &sandbox, &rel)
                                    .map_err(|e| (rel.clone(), e))?;
                            }
                            let base_changed = base_hash.as_deref() != Some(hash);
                            if base_changed {
                                index.insert(rel.clone(), hash.to_string());
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
                            let had_local = local_hash.is_some();
                            if had_local {
                                self.store
                                    .remove_under(&sandbox, &rel)
                                    .map_err(|e| (rel.clone(), e))?;
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
                    ResolveTake::Local => match head_hash.as_deref() {
                        Some(hash) => {
                            let base_changed = base_hash.as_deref() != Some(hash);
                            if base_changed {
                                index.insert(rel.clone(), hash.to_string());
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
                entries.push(ResolveEntry {
                    path: rel,
                    action,
                    backup,
                });
                done += 1;
            }
            Ok(())
        })();
        if let Err((rel, e)) = apply {
            return Err(format!(
                "解决中断：已处理 {} 条；失败于 {}：{}；已覆盖的文件可用 work_pull 对齐，已备份的本地版本在版本库 lost/ 下（可用 work_lost 找回）。",
                done, rel, e
            ));
        }
        if changed {
            if let Err(e) = self.store.write_index(&store, agent, &index) {
                return Err(format!(
                    "解决已完成，但基线写入失败：{}；重试 work_pull 或 work_resolve 会补上基线。",
                    e
                ));
            }
        }
        Ok(ResolveReport {
            take: req.take,
            head: head_id,
            entries,
            missing,
            warnings,
        })
    }

    fn work_lost(&self, work: &str, agent: &str, path: Option<&str>) -> Result<LostReport, String> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (_shared, store, sandbox) = self.agent_roots(work, agent)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &_shared, &store)?;
        let lost = lost_root(&store, agent)?;
        let filter = match path {
            Some(p) => Some(clean_rel(p)?),
            None => None,
        };
        let mut rels = self.store.list(&lost)?;
        rels.retain(|r| !is_lost_temp(r));
        if let Some(f) = &filter {
            rels.retain(|r| r == f || r.starts_with(&format!("{}/", f)));
        }
        let total = rels.len();
        let truncated = total > MAX_LOST;
        rels.truncate(MAX_LOST);
        let mut entries = Vec::new();
        for rel in rels {
            let Some(v) = self.store.scan_under(&lost, &rel, None)? else {
                continue;
            };
            let current = self.store.scan_under(&sandbox, &rel, None)?;
            entries.push(LostEntry {
                path: rel,
                hash: v.hash,
                size: v.size,
                current_hash: current.map(|c| c.hash),
            });
        }
        Ok(LostReport {
            agent: agent.to_string(),
            total,
            entries,
            truncated,
        })
    }

    fn work_restore_lost(
        &self,
        work: &str,
        agent: &str,
        path: &str,
        expect_current: &str,
    ) -> Result<RestoreLostReport, String> {
        let rel = clean_rel(path)?;
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (_shared, store, sandbox) = self.agent_roots(work, agent)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &_shared, &store)?;
        let lost = lost_root(&store, agent)?;
        let backups = self.store.list(&lost)?;
        if !backups.iter().any(|p| p == &rel) {
            return Err(format!(
                "没有这个被覆盖的本地版本：{}（先用 work_lost 看可恢复清单）",
                rel
            ));
        }
        let Some(backup) = self.store.scan_under(&lost, &rel, None)? else {
            return Err(format!("备份文件不存在：{}", rel));
        };
        let current = self.store.scan_under(&sandbox, &rel, None)?;
        let current_hash = current.as_ref().map(|v| v.hash.clone());
        let matches = match current_hash.as_deref() {
            None => expect_current.is_empty(),
            Some(hash) => hash == expect_current,
        };
        if !matches {
            return Err(format!(
                "当前沙箱内容与 expect_current 不符：当前={}；先 work_lost 重新看当前 hash，确认后再恢复。",
                current_hash.as_deref().unwrap_or("不存在")
            ));
        }
        let size = self
            .store
            .copy_under_to_under(&lost, &rel, &sandbox, &rel, &backup.hash)?;
        Ok(RestoreLostReport {
            path: rel,
            hash: backup.hash,
            size,
            previous: current_hash,
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

        // 先让旧布局的 head 升级成 refs/main，再只从引用出发算活集合：
        // 引用指向的提交 + 祖先链；不在这条集合里的提交与对象都可回收。
        let _ = self.store.head(&store)?;
        let refs = self.store.list_refs(&store)?;
        let mut stack: Vec<u64> = Vec::new();
        for r in refs.values() {
            stack.extend(r.target);
            stack.extend(r.heads.values().flatten().copied());
        }
        let mut live: BTreeSet<u64> = BTreeSet::new();
        let mut reachable: BTreeSet<String> = BTreeSet::new();
        while let Some(id) = stack.pop() {
            if !live.insert(id) {
                continue;
            }
            let Some(commit) = self.store.read_commit(&store, id)? else {
                continue;
            };
            for ch in &commit.changes {
                if !ch.hash.is_empty() {
                    reachable.insert(ch.hash.clone());
                }
            }
            if let Some(tree) = &commit.tree {
                reachable.extend(tree.values().cloned());
            }
            if let Some(parent) = commit.parent {
                stack.push(parent);
            }
        }
        // 只被基线引用的对象也必须保留。
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
        if apply {
            let ids = self.store.list_commits(&store)?;
            for id in ids {
                if !live.contains(&id) {
                    self.store.remove_commit(&store, id)?;
                    report.removed_commits += 1;
                }
            }
        }
        Ok(report)
    }

    fn work_restore(&self, work: &str, commit: u64) -> Result<(), String> {
        self.restore_impl(work, commit, false)
    }

    fn work_restore_force(&self, work: &str, commit: u64) -> Result<(), String> {
        self.restore_impl(work, commit, true)
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
        let _shared = self.dirs.lock_shared(work)?;
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        // 只沿**当前 head 的祖先链**选点：用户回滚离开过的提交仍在库里，但不在当前线上，
        // 不能被后续转录回档误选回来。
        let head = self.store.head(&store)?;
        let target = self.archive_point(&store, head, keep_by_agent)?;
        let conflicts = self.external_materialize_conflicts(work, &shared, &store, target)?;
        if !conflicts.is_empty() {
            return Err(format!(
                "回档会覆盖工作区里被外部改动的内容：{}——先 work_sync 采纳外部版本；强制覆盖需要用户危险确认。",
                conflicts.join("、")
            ));
        }
        self.pin_head(&store)?;
        let pending = Pending::Restore { target };
        self.store.write_pending(&store, &pending)?;
        let result = WorkspaceService::materialize(self.store.as_ref(), &shared, &store, target);
        self.settle(work, &shared, &store, result)?;
        Ok(target)
    }

    fn work_archive(
        &self,
        work: &str,
        keep_by_agent: &BTreeMap<String, u64>,
        agents: &[String],
        mark: u64,
    ) -> Result<ArchiveReport, String> {
        if mark == 0 {
            return Err("档案号不能是 0".to_string());
        }
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let _shared = self.dirs.lock_shared(work)?;
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        let name = format!("archive/{}", mark);
        if self.store.read_ref(&store, &name)?.is_some() {
            return Err(format!("档案 {} 已存在", mark));
        }
        let head = self.store.head(&store)?;
        let target = self.archive_point(&store, head, keep_by_agent)?;
        let tree = match target {
            Some(id) => Self::tree_of(self.store.as_ref(), &store, id)?,
            None => Tree::new(),
        };
        let conflicts = self.external_materialize_conflicts(work, &shared, &store, target)?;
        if !conflicts.is_empty() {
            return Err(format!(
                "留档会覆盖工作区里被外部改动的内容：{}——先 work_sync 采纳外部版本。",
                conflicts.join("、")
            ));
        }
        let mut uniq: BTreeSet<String> = BTreeSet::new();
        for agent in agents {
            if !agent.is_empty() {
                uniq.insert(agent.clone());
            }
        }
        // 引用先落：它记着留档前的主线；此时文件还没动，失败可以整次撤销。
        let mut heads: BTreeMap<String, Option<u64>> = BTreeMap::new();
        for agent in &uniq {
            heads.insert(agent.clone(), head);
        }
        self.store.write_ref(
            &store,
            &name,
            &Ref {
                kind: RefKind::Archive,
                target: head,
                heads: heads.clone(),
            },
        )?;
        let mut backed_up: BTreeMap<String, usize> = BTreeMap::new();
        let result = (|| -> Result<(), String> {
            for agent in &uniq {
                let (_, _, sandbox) = self.agent_roots(work, agent)?;
                let prefix = format!("archive-{}", mark);
                let n = self.lay_down_tree(&store, &sandbox, &tree, agent, Some(&prefix))?;
                self.reset_agent_point(&store, agent, target, &tree)?;
                backed_up.insert(agent.clone(), n);
            }
            let pending = Pending::Restore { target };
            self.store.write_pending(&store, &pending)?;
            let result =
                WorkspaceService::materialize(self.store.as_ref(), &shared, &store, target);
            self.settle(work, &shared, &store, result)
        })();
        if result.is_err() {
            let _ = self.store.remove_ref(&store, &name);
        }
        result?;
        Ok(ArchiveReport {
            mark,
            shared: head,
            point: target,
            backed_up,
        })
    }

    fn work_restore_archive(&self, work: &str, mark: u64) -> Result<Option<u64>, String> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let _shared = self.dirs.lock_shared(work)?;
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        let name = format!("archive/{}", mark);
        let Some(reference) = self.store.read_ref(&store, &name)? else {
            return Err(format!("没有档案 {}（可能已恢复）", mark));
        };
        let target = reference.target;
        if let Some(id) = target {
            if self.store.read_commit(&store, id)?.is_none() {
                return Err(format!("档案 {} 指向的提交 {} 不在了", mark, id));
            }
        }
        let tree = match target {
            Some(id) => Self::tree_of(self.store.as_ref(), &store, id)?,
            None => Tree::new(),
        };
        let conflicts = self.external_materialize_conflicts(work, &shared, &store, target)?;
        if !conflicts.is_empty() {
            return Err(format!(
                "恢复会覆盖工作区里被外部改动的内容：{}——先 work_sync 采纳外部版本。",
                conflicts.join("、")
            ));
        }
        // 先把当前线钉住，再动文件：剪叉不靠"删提交"保证可恢复。
        self.pin_head(&store)?;
        for agent in reference.heads.keys() {
            let (_, _, sandbox) = self.agent_roots(work, agent)?;
            let prefix = format!("restore-{}", mark);
            self.lay_down_tree(&store, &sandbox, &tree, agent, Some(&prefix))?;
            self.reset_agent_point(&store, agent, target, &tree)?;
        }
        let pending = Pending::Restore { target };
        self.store.write_pending(&store, &pending)?;
        let result = WorkspaceService::materialize(self.store.as_ref(), &shared, &store, target);
        self.settle(work, &shared, &store, result)?;
        self.store.remove_ref(&store, &name)?;
        Ok(target)
    }

    #[cfg(test)]
    fn work_restore_point(&self, work: &str, commit: Option<u64>) -> Result<(), String> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let _shared = self.dirs.lock_shared(work)?;
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        if let Some(id) = commit {
            if self.store.read_commit(&store, id)?.is_none() {
                return Err(format!("没有提交点 {}", id));
            }
        }
        let conflicts = self.external_materialize_conflicts(work, &shared, &store, commit)?;
        if !conflicts.is_empty() {
            return Err(format!(
                "回档会覆盖工作区里被外部改动的内容：{}——先 work_sync 采纳外部版本；强制覆盖需要用户危险确认。",
                conflicts.join("、")
            ));
        }
        self.pin_head(&store)?;
        let pending = Pending::Restore { target: commit };
        self.store.write_pending(&store, &pending)?;
        let result = WorkspaceService::materialize(self.store.as_ref(), &shared, &store, commit);
        self.settle(work, &shared, &store, result)
    }

    fn work_discard_after(&self, work: &str, keep: Option<u64>) -> Result<(), String> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let _shared = self.dirs.lock_shared(work)?;
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        if let Some(id) = keep {
            if self.store.read_commit(&store, id)?.is_none() {
                return Err(format!("没有提交点 {}", id));
            }
        }
        let conflicts = self.external_materialize_conflicts(work, &shared, &store, keep)?;
        if !conflicts.is_empty() {
            return Err(format!(
                "回档会覆盖工作区里被外部改动的内容：{}——先 work_sync 采纳外部版本；强制覆盖需要用户危险确认。",
                conflicts.join("、")
            ));
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
