//! 共享区**版本化工作区**的纯逻辑：内容寻址的提交记录、每个 agent 的拉取基线（index）、
//! 文件级三方比较、路径校验与路径展开。
//!
//! 规则（见 PRODUCT.md 的「共享区是版本化工作区」与 docs/workspace 的细则）：
//! - 共享主副本 `work/` 只在提交时被改写；agent 只写自己的沙箱，pull 把主副本内容拉到沙箱同相对路径；
//! - 提交的基准是**这个 agent 上次拉取的那一版**（index），不是全局的 merge-base——
//!   单条共享主线 + 选择性拉取，前者才是它真正看过的版本；
//! - 两边都改了同一文件且结果不同 = 冲突，整个提交拒绝，不做自动合并。
//!
//! 本文件没有 IO，也不加 trait（纯派生逻辑）。

use crate::capabilities::workspace::domain::asset::FileVersion;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// 每个 agent 的拉取基线：共享区相对路径 → 当时的内容指纹。
pub type Index = BTreeMap<String, String>;
/// 一个提交点的整棵树：共享区相对路径 → 内容指纹。
pub type Tree = BTreeMap<String, String>;

/// 一次改动是新增、修改还是删除。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Add,
    Modify,
    Delete,
}

/// 提交记录里的一条改动。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub path: String,
    pub kind: ChangeKind,
    /// 新内容的指纹；删除为空串。
    #[serde(default)]
    pub hash: String,
}

/// 提交锚：这个提交点是**哪个会话（agent 实例名）的哪一行**触发的。
/// **空 agent = 主会话/用户投喂**。回档按它把共享区物化回某一行那一刻。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitAnchor {
    #[serde(default)]
    pub agent: String,
    #[serde(default)]
    pub line: u64,
}

/// 一个提交点（提交记录 + 该点的整棵树）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    pub id: u64,
    pub parent: Option<u64>,
    /// 作者：agent 实例名，或 `user`（界面投喂）。
    pub author: String,
    /// 提交所属会话（界面投喂为空串）。
    #[serde(default)]
    pub session: String,
    pub time: i64,
    pub message: String,
    pub changes: Vec<Change>,
    pub tree: Tree,
    /// 转录行锚（旧记录没有 = None，按"总在回档点之前"保守处理）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<CommitAnchor>,
}

/// 冲突的四种形态（回执要能点名"哪种冲突"，不是笼统一句"冲突"）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conflict {
    /// 两边都改了同一文件，结果不同。
    BothModified,
    /// 两边都新增了同一路径，内容不同。
    BothAdded,
    /// 上游删了、本地改了。
    ModifyVsDelete,
    /// 本地删了、上游改了。
    DeleteVsModify,
    /// 本地要删一个自己没拉取过的路径——没有基线，无法确认删的是哪一版。
    DeleteWithoutBase,
}

impl Conflict {
    pub fn reason(&self) -> &'static str {
        match self {
            Conflict::BothModified => "两边都改了",
            Conflict::BothAdded => "两边都新增了",
            Conflict::ModifyVsDelete => "上游删了、本地改了",
            Conflict::DeleteVsModify => "本地删了、上游改了",
            Conflict::DeleteWithoutBase => "没拉取过，无法确认删除的是哪一版",
        }
    }
}

/// 冲突要能点名是**哪个路径**的哪种冲突（回执据此让 agent 逐条处理）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictAt {
    pub path: String,
    pub kind: Conflict,
}

/// 一条拉取的判定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullVerdict {
    /// 把主副本这一版写进沙箱，并更新基线。
    Update,
    /// 沙箱里已经是主副本这一版：只更新基线。
    AlreadySame,
    /// 沙箱里有一个未被跟踪、或与基线无关的本地文件：不覆盖。
    Occupied,
    /// 本地已改、上游也改了：不覆盖，报冲突。
    Conflict,
    /// 主副本没有这个路径。
    Missing,
    /// 什么都没变（上游没动）。
    NoChange,
}

/// 校验一条共享区相对路径：非空、以 `/` 分隔、无空段、无 `.` / `..`、无控制字符、无反斜杠。
/// 只做路径本身的事；落在哪个根之内由调用方用 `join_rel` 之后再校验。
pub fn clean_rel(raw: &str) -> Result<String, String> {
    if raw.is_empty() {
        return Err("路径不能为空".to_string());
    }
    if raw.contains('\\') {
        return Err(format!("路径要用 / 分隔：{}", raw));
    }
    if raw.starts_with('/') {
        return Err(format!("路径必须是共享区内的相对路径：{}", raw));
    }
    let mut parts: Vec<&str> = Vec::new();
    for seg in raw.split('/') {
        if seg.is_empty() {
            return Err(format!("路径不能有空段或首尾斜杠：{}", raw));
        }
        if seg == "." || seg == ".." {
            return Err(format!("路径不能包含 . 或 ..：{}", raw));
        }
        if seg.chars().any(|c| c.is_control()) {
            return Err(format!("路径不能包含控制字符：{}", raw));
        }
        parts.push(seg);
    }
    Ok(parts.join("/"))
}

/// 把一条已校验的相对路径拼到一个根下（一律用路径组件，禁止把分隔符写进字符串再拼）。
pub fn join_rel(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let clean = clean_rel(rel)?;
    let mut p = root.to_path_buf();
    for seg in clean.split('/') {
        p.push(seg);
    }
    Ok(p)
}

/// 把 `paths`（文件或文件夹）展开成具体文件集合：路径本身是文件就取它，
/// 是文件夹就取该前缀下的全部文件（available 是候选文件全集）。返回（展开后的路径，未命中的请求）。
pub fn expand_paths(paths: &[String], available: &BTreeSet<String>) -> (Vec<String>, Vec<String>) {
    let mut out: BTreeSet<String> = BTreeSet::new();
    let mut missing: Vec<String> = Vec::new();
    for raw in paths {
        let Ok(rel) = clean_rel(raw) else {
            missing.push(raw.clone());
            continue;
        };
        if available.contains(&rel) {
            out.insert(rel);
            continue;
        }
        let prefix = format!("{}/", rel);
        let hit = available.iter().any(|p| p.starts_with(&prefix));
        if hit {
            for p in available.iter().filter(|p| p.starts_with(&prefix)) {
                out.insert(p.clone());
            }
        } else {
            missing.push(rel);
        }
    }
    (out.into_iter().collect(), missing)
}

/// 文件级三方比较（提交侧）：`locals` = 路径 → 要写的**内容指纹**（`None` = 显式删除）。
/// Ok(改动清单) 为空 = 没有可提交的改动；Err = 冲突清单（**整个提交都拒绝**）。
pub fn plan_commit(
    index: &Index,
    tree: &Tree,
    locals: &BTreeMap<String, Option<String>>,
) -> Result<Vec<Change>, Vec<ConflictAt>> {
    let mut changes: Vec<Change> = Vec::new();
    let mut conflicts: Vec<(String, Conflict)> = Vec::new();
    for (path, local) in locals {
        let base = index.get(path).map(String::as_str);
        let head = tree.get(path).map(String::as_str);
        match local {
            None => {
                // 显式删除：必须有基线，且上游没在拉取之后动过。
                match (base, head) {
                    (None, _) => conflicts.push((path.clone(), Conflict::DeleteWithoutBase)),
                    (Some(_), None) => {}
                    (Some(b), Some(h)) if b == h => changes.push(Change {
                        path: path.clone(),
                        kind: ChangeKind::Delete,
                        hash: String::new(),
                    }),
                    (Some(_), Some(_)) => conflicts.push((path.clone(), Conflict::DeleteVsModify)),
                }
            }
            Some(local_hash) => {
                let local_hash = local_hash.as_str();
                match (base, head) {
                    (None, None) => changes.push(Change {
                        path: path.clone(),
                        kind: ChangeKind::Add,
                        hash: local_hash.to_string(),
                    }),
                    (None, Some(h)) => {
                        if local_hash == h {
                            // 与主副本一致：无需提交
                        } else {
                            conflicts.push((path.clone(), Conflict::BothAdded));
                        }
                    }
                    (Some(b), None) => {
                        if local_hash == b {
                            // 本地没改，上游已删：无需提交
                        } else {
                            conflicts.push((path.clone(), Conflict::ModifyVsDelete));
                        }
                    }
                    (Some(b), Some(h)) => {
                        if local_hash == b {
                            // 本地没改
                        } else if local_hash == h {
                            // 已经和主副本一致
                        } else if b == h {
                            changes.push(Change {
                                path: path.clone(),
                                kind: ChangeKind::Modify,
                                hash: local_hash.to_string(),
                            });
                        } else {
                            conflicts.push((path.clone(), Conflict::BothModified));
                        }
                    }
                }
            }
        }
    }
    if conflicts.is_empty() {
        Ok(changes)
    } else {
        Err(conflicts
            .into_iter()
            .map(|(path, kind)| ConflictAt { path, kind })
            .collect())
    }
}

/// 文件级三方比较（拉取侧）：给哈希（本地文件的内容指纹），返回该路径该怎么处理。
pub fn pull_verdict(base: Option<&str>, head: Option<&str>, local: Option<&str>) -> PullVerdict {
    let Some(head) = head else {
        return PullVerdict::Missing;
    };
    match base {
        None => match local {
            None => PullVerdict::Update,
            Some(l) if l == head => PullVerdict::AlreadySame,
            Some(_) => PullVerdict::Occupied,
        },
        Some(b) => match local {
            // 本地删了自己的副本：上游没动就保持删除，上游动了就没法自动合。
            None => {
                if b == head {
                    PullVerdict::NoChange
                } else {
                    PullVerdict::Conflict
                }
            }
            Some(l) => {
                if l == b {
                    if b == head {
                        PullVerdict::NoChange
                    } else {
                        PullVerdict::Update
                    }
                } else if l == head {
                    PullVerdict::AlreadySame
                } else if b == head {
                    // 上游没动、本地改了：不动本地
                    PullVerdict::NoChange
                } else {
                    PullVerdict::Conflict
                }
            }
        },
    }
}

/// 目的：一条路径的三方版本：基线（上次拉取/提交）/ 主副本当前 / 沙箱本地。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionView {
    /// 目的：共享区相对路径。
    pub path: String,
    /// 目的：主副本当前提交 id（空共享区 = None）；take=local 时作为 head 传回。
    pub head_commit: Option<u64>,
    /// 目的：按扩展名给出的媒体类型提示；未知 = None（内容以 hash / size 为准）。
    pub media_type: Option<String>,
    /// 目的：基线版本；没拉取过 = None。
    pub base: Option<FileVersion>,
    /// 目的：主副本当前版本；主副本已删除 = None。
    pub head: Option<FileVersion>,
    /// 目的：沙箱本地版本；本地不存在 = None。
    pub local: Option<FileVersion>,
}

/// 目的：解决冲突时的取法：取主副本，或保留本地并把基线推进到主副本。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveTake {
    Upstream,
    Local,
}

impl ResolveTake {
    pub fn as_str(&self) -> &'static str {
        match self {
            ResolveTake::Upstream => "upstream",
            ResolveTake::Local => "local",
        }
    }
}

/// 目的：一条路径被解决后实际发生了什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveAction {
    /// 目的：用主副本覆盖了沙箱（或写回了已删除的本地文件）。
    WroteUpstream,
    /// 目的：主副本已删除，删掉了沙箱里的本地文件。
    RemovedLocal,
    /// 目的：没有动文件，只把基线推进到主副本。
    Marked,
    /// 目的：主副本已删除，清掉了基线（本地文件保留时，下一步会成为新增）。
    Cleared,
    /// 目的：本来就已经一致，什么都没做。
    NoChange,
}

impl ResolveAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            ResolveAction::WroteUpstream => "用主副本覆盖沙箱",
            ResolveAction::RemovedLocal => "删掉本地文件",
            ResolveAction::Marked => "推进基线到主副本",
            ResolveAction::Cleared => "清掉基线",
            ResolveAction::NoChange => "无需改动",
        }
    }
}

/// 目的：一条路径的解决结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveEntry {
    /// 目的：共享区相对路径。
    pub path: String,
    /// 目的：实际发生的动作。
    pub action: ResolveAction,
    /// 目的：take=upstream 覆盖/删除前写入的本地版本备份（相对版本库 `.work/` 的路径）；
    ///   没有备份 = None。
    pub backup: Option<String>,
}

/// 目的：work_resolve 的请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveRequest {
    /// 目的：要解决的路径（文件或文件夹）。
    pub paths: Vec<String>,
    /// 目的：取主副本还是保留本地。
    pub take: ResolveTake,
    /// 目的：take=local 时调用方看到的主副本提交 id；用来拒绝“查看后主副本又前进”。
    /// 约束：take=local 且工作已有提交时必填；take=upstream 忽略。
    pub expect_head: Option<u64>,
}

/// 目的：一条路径的风险提示：不阻止执行，但回执必须显式说清。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveWarningKind {
    /// 目的：主副本有这一版，但沙箱里没有文件；take=local 只推基线，不会写回文件。
    LocalMissing,
    /// 目的：没有基线（从未拉取过），take=local 会把主副本当前版当作已合并。
    NoBase,
}

impl ResolveWarningKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ResolveWarningKind::LocalMissing => {
                "本地没有这个文件；take=local 只推进基线，不会写入沙箱；要删除主副本这一版，提交时用 deletes 显式列出"
            }
            ResolveWarningKind::NoBase => {
                "从未拉取过（没有基线）；take=local 会把主副本当前版当作已合并，提交会覆盖它"
            }
        }
    }
}

/// 目的：一条路径的风险提示。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveWarning {
    /// 目的：共享区相对路径。
    pub path: String,
    /// 目的：提示类型。
    pub kind: ResolveWarningKind,
}

/// 目的：work_resolve 的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveReport {
    /// 目的：本次使用的取法。
    pub take: ResolveTake,
    /// 目的：本次读到的主副本提交 id（空共享区 = None）。
    pub head: Option<u64>,
    /// 目的：逐条路径的解决结果。
    pub entries: Vec<ResolveEntry>,
    /// 目的：请求了、但基线 / 主副本 / 沙箱三方都没有的路径。
    pub missing: Vec<String>,
    /// 目的：逐条路径的风险提示；不阻止执行，但回执必须列出。
    pub warnings: Vec<ResolveWarning>,
}

/// 目的：take=local 的纯判定：拒绝“本地与基线一致、上游却已更新”的伪解决。
/// 返回：DenyUnchanged = 本地没改过，take=local 只会用旧内容覆盖上游；
///   Warn* = 可以继续，但回执要显式提示风险。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalTakeVerdict {
    Ok,
    WarnLocalMissing,
    WarnNoBase,
    DenyUnchanged,
}

/// 目的：按基线 / 主副本 / 沙箱三方给出 take=local 的判定。
/// 约束：head=None（上游已删）不算覆盖上游，不拒绝也不提示。
pub fn local_take_verdict(
    base: Option<&str>,
    head: Option<&str>,
    local: Option<&str>,
) -> LocalTakeVerdict {
    let Some(head) = head else {
        return LocalTakeVerdict::Ok;
    };
    match (base, local) {
        (_, None) => LocalTakeVerdict::WarnLocalMissing,
        (Some(b), Some(l)) if b != head && l == b => LocalTakeVerdict::DenyUnchanged,
        (None, Some(l)) if l != head => LocalTakeVerdict::WarnNoBase,
        _ => LocalTakeVerdict::Ok,
    }
}

/// 目的：take=upstream 前是否需要备份本地文件。
/// 返回：本地存在且与主副本当前版不同 = true（覆盖或删除都会丢内容）。
pub fn needs_backup(head: Option<&str>, local: Option<&str>) -> bool {
    local.is_some() && local != head
}

/// 目的：把一个 agent 的一条路径映射到版本库里的备份相对路径：`lost/<agent>/<rel>`。
/// 约束：agent 必须能作为单个路径组件；rel 必须是干净的共享区相对路径。
pub fn lost_rel(agent: &str, rel: &str) -> Result<String, String> {
    let rel = clean_rel(rel)?;
    if agent.is_empty()
        || agent == "."
        || agent == ".."
        || agent.contains('/')
        || agent.contains('\\')
    {
        return Err(format!("agent 名不能用作备份目录：{}", agent));
    }
    Ok(format!("lost/{}/{}", agent, rel))
}

/// 目的：一份被 work_resolve take=upstream 覆盖/删除的本地版本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LostEntry {
    /// 目的：共享区相对路径。
    pub path: String,
    /// 目的：备份内容的指纹。
    pub hash: String,
    /// 目的：备份内容的字节数。
    pub size: u64,
    /// 目的：当前沙箱同路径的内容指纹；不存在 = None（恢复时 expect_current 传空串）。
    pub current_hash: Option<String>,
}

/// 目的：work_lost 的清单结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LostReport {
    /// 目的：这份清单属于哪个 agent（备份目录 `lost/<agent>/`）。
    pub agent: String,
    /// 目的：过滤后、截断前的总份数。
    pub total: usize,
    /// 目的：本次列出的份数（最多到上限）。
    pub entries: Vec<LostEntry>,
    /// 目的：是否因上限只列出了一部分。
    pub truncated: bool,
}

/// 目的：work_restore_lost 的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreLostReport {
    /// 目的：恢复到的共享区相对路径。
    pub path: String,
    /// 目的：恢复的备份内容指纹。
    pub hash: String,
    /// 目的：恢复的备份内容字节数。
    pub size: u64,
    /// 目的：覆盖前沙箱同路径的指纹；之前不存在 = None。
    pub previous: Option<String>,
}

/// 目的：一个 agent 的基线与 agent 名的配对，供待恢复事务写回。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingIndex {
    /// 目的：agent 实例名。
    pub agent: String,
    /// 目的：这次提交后的新基线。
    pub index: Index,
}

/// 目的：版本库写事务的待恢复日志；一个工作同时最多一条。
/// 约束：重放必须幂等；成功后由恢复方删除 pending。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum Pending {
    /// 目的：一次提交（主副本改动 + 可选基线更新）。
    Commit {
        /// 目的：已经算好的提交记录（box 住，避免 pending 枚举被整棵提交树撑大）。
        commit: Box<Commit>,
        /// 目的：提交后的 agent 基线；用户投喂为 None。
        index: Option<PendingIndex>,
    },
    /// 目的：把主副本物化到某个提交点（None = 清空）。
    Restore {
        /// 目的：目标提交号。
        target: Option<u64>,
    },
    /// 目的：丢弃 keep 之外的提交并物化到 keep。
    Discard {
        /// 目的：要保留的提交点。
        keep: Option<u64>,
    },
}

/// 目的：一个内容寻址对象的事实（对象回收用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectInfo {
    /// 目的：内容指纹（文件名）。
    pub hash: String,
    /// 目的：对象字节数。
    pub size: u64,
}

/// 目的：一次对象回收的报告；apply=false 时只统计不删除。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GcReport {
    /// 目的：是否真的执行了删除。
    pub applied: bool,
    /// 目的：对象总数。
    pub objects: usize,
    /// 目的：对象总字节。
    pub bytes: u64,
    /// 目的：可达对象数。
    pub reachable_objects: usize,
    /// 目的：可达对象字节。
    pub reachable_bytes: u64,
    /// 目的：不可达对象数。
    pub unreachable_objects: usize,
    /// 目的：不可达对象字节。
    pub unreachable_bytes: u64,
    /// 目的：本次实际删除的对象数。
    pub removed_objects: usize,
    /// 目的：本次实际删除的字节。
    pub removed_bytes: u64,
    /// 目的：对象目录里不是内容指纹的杂项文件数（临时文件等）。
    pub ignored_files: usize,
    /// 目的：本次实际清掉的杂项文件数。
    pub removed_ignored: usize,
}

/// 提交点摘要（work_status 用）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommitSummary {
    pub id: u64,
    pub author: String,
    pub time: i64,
    pub message: String,
}

impl From<&Commit> for CommitSummary {
    fn from(c: &Commit) -> CommitSummary {
        CommitSummary {
            id: c.id,
            author: c.author.clone(),
            time: c.time,
            message: c.message.clone(),
        }
    }
}

/// 一条路径在 agent 眼里的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusState {
    /// 共享区有，但本 agent 没拉过，沙箱里也没有同名文件。
    NotPulled,
    /// 目的：共享区有、本 agent 没拉过，但沙箱里已有同名且内容不同的文件（pull 会跳过）。
    BothPresent,
    /// 拉过且本地没改、上游没动。
    Clean,
    /// 目的：沙箱里有、基线与主副本都没有（下一次提交会成为新增）。
    LocalOnly,
    /// 拉过之后本地改了。
    LocalModified,
    /// 拉过之后上游又动了。
    UpstreamMoved,
    /// 目的：上游删了，本地没有改过（本地可能还留着旧副本）。
    UpstreamDeleted,
    /// 本地改了、上游也动了（或一边删、一边改）。
    Conflict,
}

impl StatusState {
    pub fn as_str(&self) -> &'static str {
        match self {
            StatusState::NotPulled => "未拉取",
            StatusState::BothPresent => "未拉取（沙箱有同名且内容不同）",
            StatusState::Clean => "一致",
            StatusState::LocalOnly => "本地新增（未跟踪）",
            StatusState::LocalModified => "本地有修改",
            StatusState::UpstreamMoved => "上游有更新",
            StatusState::UpstreamDeleted => "上游已删除",
            StatusState::Conflict => "冲突",
        }
    }
}

/// work_status 的逐条结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    pub path: String,
    pub state: StatusState,
}

/// 一条路径在"基线 / 上游 / 本地"下的状态（只读展示用，与提交判定分开）。
pub fn status_of(base: Option<&str>, head: Option<&str>, local: Option<&str>) -> StatusState {
    match (base, head) {
        (None, None) => match local {
            None => StatusState::Clean,
            Some(_) => StatusState::LocalOnly,
        },
        (None, Some(h)) => match local {
            None => StatusState::NotPulled,
            Some(l) if l == h => StatusState::NotPulled,
            Some(_) => StatusState::BothPresent,
        },
        (Some(b), Some(h)) => match local {
            None => {
                if b == h {
                    StatusState::LocalModified
                } else {
                    StatusState::Conflict
                }
            }
            Some(l) if l == b => {
                if b == h {
                    StatusState::Clean
                } else {
                    StatusState::UpstreamMoved
                }
            }
            Some(l) if l == h => StatusState::Clean,
            Some(_) => {
                if b == h {
                    StatusState::LocalModified
                } else {
                    StatusState::Conflict
                }
            }
        },
        (Some(b), None) => match local {
            None => StatusState::UpstreamDeleted,
            Some(l) if l == b => StatusState::UpstreamDeleted,
            Some(_) => StatusState::Conflict,
        },
    }
}

/// work_pull 的逐条结果（调用方据此渲染回执）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PullReport {
    /// 拉取后到的主副本提交点。
    pub commit: u64,
    /// 写进沙箱并更新基线的路径。
    pub updated: Vec<String>,
    /// 沙箱里已经是主副本这一版：只更新基线。
    pub already: Vec<String>,
    /// 什么都没变（上游没动）。
    pub no_change: Vec<String>,
    /// 沙箱里已有未跟踪 / 无关的同名文件，未覆盖。
    pub occupied: Vec<String>,
    /// 目的：本地已改、上游也改了，未覆盖；逐条带冲突类型。
    pub conflicts: Vec<ConflictAt>,
    /// 主副本里没有这些路径。
    pub missing: Vec<String>,
}

/// 一次提交的请求（收口成对象：路径 + 显式删除 + 说明 + 时间）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommitRequest {
    pub paths: Vec<String>,
    pub deletes: Vec<String>,
    pub message: String,
    /// 提交时间（由调用方给：本能力不持时钟）。
    pub time: i64,
    /// 提交锚的行号（调用方给：这一席下一条转录行的 id）。
    pub line: u64,
}

/// work_commit 的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitReport {
    pub id: u64,
    pub changes: Vec<Change>,
}

/// work_status 的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatusReport {
    pub head: Option<CommitSummary>,
    /// 主副本在 head 上的路径清单（有上限；tree_truncated = 如实标注是否截断）。
    pub tree: Vec<String>,
    pub tree_truncated: bool,
    pub entries: Vec<StatusEntry>,
    /// 目的：沙箱里有、基线与主副本都没有的路径（下一次提交会成为新增；有上限）。
    pub local_only: Vec<String>,
    /// 目的：`local_only` 是否被截断（如实标注，不假装列全）。
    pub local_only_truncated: bool,
    /// 最近几条提交（新→旧）。
    pub recent: Vec<CommitSummary>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(entries: &[(&str, &str)]) -> Tree {
        entries
            .iter()
            .map(|(p, h)| (p.to_string(), h.to_string()))
            .collect()
    }
    fn index(entries: &[(&str, &str)]) -> Index {
        entries
            .iter()
            .map(|(p, h)| (p.to_string(), h.to_string()))
            .collect()
    }
    fn locals(entries: &[(&str, Option<&str>)]) -> BTreeMap<String, Option<String>> {
        entries
            .iter()
            .map(|(p, v)| (p.to_string(), v.map(|s| s.to_string())))
            .collect()
    }

    #[test]
    fn clean_relative_paths_only() {
        assert_eq!(clean_rel("a/b.txt").unwrap(), "a/b.txt");
        for bad in ["", "/abs", "a//b", "a/./b", "a/../b", "a\\b", "a/"] {
            assert!(clean_rel(bad).is_err(), "{}", bad);
        }
    }

    #[test]
    fn folders_expand_and_missing_is_reported() {
        let available: BTreeSet<String> = ["a.md", "dir/x", "dir/sub/y"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (got, missing) = expand_paths(&["dir".to_string(), "a.md".to_string()], &available);
        assert_eq!(got, vec!["a.md", "dir/sub/y", "dir/x"]);
        assert!(missing.is_empty());
        let (_, missing) = expand_paths(&["nope".to_string()], &available);
        assert_eq!(missing, vec!["nope"]);
    }

    #[test]
    fn commit_rules_cover_add_modify_delete_and_noop() {
        // 主副本有 f1（h1），本地改成 h2 → modify。
        let changes = plan_commit(
            &index(&[("f1", "h1")]),
            &tree(&[("f1", "h1")]),
            &locals(&[("f1", Some("h2"))]),
        )
        .unwrap();
        assert_eq!(
            changes,
            vec![Change {
                path: "f1".into(),
                kind: ChangeKind::Modify,
                hash: "h2".to_string()
            }]
        );
        // 本地没改 → 没得提交。
        let changes = plan_commit(
            &index(&[("f1", "h2")]),
            &tree(&[("f1", "h1")]),
            &locals(&[("f1", Some("h2"))]),
        )
        .unwrap();
        assert!(changes.is_empty());
        // 基线 h1、上游 h2、本地 h3：两边都改了 → 冲突。
        let err = plan_commit(
            &index(&[("f1", "h1")]),
            &tree(&[("f1", "h2")]),
            &locals(&[("f1", Some("h3"))]),
        )
        .unwrap_err();
        assert_eq!(
            err,
            vec![ConflictAt {
                path: "f1".into(),
                kind: Conflict::BothModified
            }]
        );
        // 显式删除：基线 == 上游 → delete；上游动过 → 冲突。
        let changes = plan_commit(
            &index(&[("f1", "h1")]),
            &tree(&[("f1", "h1")]),
            &locals(&[("f1", None)]),
        )
        .unwrap();
        assert_eq!(changes[0].kind, ChangeKind::Delete);
        assert!(plan_commit(
            &index(&[("f1", "h1")]),
            &tree(&[("f1", "h9")]),
            &locals(&[("f1", None)])
        )
        .is_err());
        // 没拉取过就删 → 冲突（不是静默删）。
        assert_eq!(
            plan_commit(
                &index(&[]),
                &tree(&[("f1", "h1")]),
                &locals(&[("f1", None)])
            )
            .unwrap_err(),
            vec![ConflictAt {
                path: "f1".into(),
                kind: Conflict::DeleteWithoutBase
            }]
        );
        // 两边都新增同一路径、内容不同 → 冲突。
        assert_eq!(
            plan_commit(
                &index(&[]),
                &tree(&[("f1", "h1")]),
                &locals(&[("f1", Some("h2"))])
            )
            .unwrap_err(),
            vec![ConflictAt {
                path: "f1".into(),
                kind: Conflict::BothAdded
            }]
        );
    }

    #[test]
    fn pull_rules_never_clobber_local_work() {
        assert_eq!(pull_verdict(None, Some("h1"), None), PullVerdict::Update);
        assert_eq!(
            pull_verdict(None, Some("h1"), Some("h9")),
            PullVerdict::Occupied
        );
        assert_eq!(
            pull_verdict(Some("h1"), Some("h1"), Some("h1")),
            PullVerdict::NoChange
        );
        assert_eq!(
            pull_verdict(Some("h1"), Some("h2"), Some("h1")),
            PullVerdict::Update
        );
        assert_eq!(
            pull_verdict(Some("h1"), Some("h2"), Some("h3")),
            PullVerdict::Conflict
        );
        assert_eq!(
            pull_verdict(Some("h1"), None, Some("h1")),
            PullVerdict::Missing
        );
    }

    #[test]
    fn status_separates_local_change_from_upstream_change() {
        assert_eq!(status_of(None, Some("h1"), None), StatusState::NotPulled);
        assert_eq!(
            status_of(Some("h1"), Some("h1"), Some("h1")),
            StatusState::Clean
        );
        assert_eq!(
            status_of(Some("h1"), Some("h1"), Some("h2")),
            StatusState::LocalModified
        );
        assert_eq!(
            status_of(Some("h1"), Some("h2"), Some("h1")),
            StatusState::UpstreamMoved
        );
        assert_eq!(
            status_of(Some("h1"), Some("h2"), Some("h3")),
            StatusState::Conflict
        );
        assert_eq!(
            status_of(None, Some("h1"), Some("h2")),
            StatusState::BothPresent
        );
        assert_eq!(status_of(None, None, Some("h1")), StatusState::LocalOnly);
        assert_eq!(
            status_of(Some("h1"), None, None),
            StatusState::UpstreamDeleted
        );
        assert_eq!(
            status_of(Some("h1"), None, Some("h1")),
            StatusState::UpstreamDeleted
        );
        assert_eq!(
            status_of(Some("h1"), None, Some("h2")),
            StatusState::Conflict
        );
    }

    #[test]
    fn local_take_verdict_rejects_unmerged_and_warns_on_blind_states() {
        use LocalTakeVerdict::*;
        assert_eq!(
            local_take_verdict(Some("h1"), Some("h2"), Some("h1")),
            DenyUnchanged
        );
        assert_eq!(local_take_verdict(Some("h1"), Some("h2"), Some("h3")), Ok);
        assert_eq!(
            local_take_verdict(Some("h1"), Some("h1"), Some("h1")),
            Ok,
            "上游没动时 take=local 只是推进基线，不是覆盖"
        );
        assert_eq!(local_take_verdict(None, Some("h2"), None), WarnLocalMissing);
        assert_eq!(
            local_take_verdict(Some("h1"), Some("h2"), None),
            WarnLocalMissing
        );
        assert_eq!(local_take_verdict(None, Some("h2"), Some("h1")), WarnNoBase);
        assert_eq!(
            local_take_verdict(None, Some("h2"), Some("h2")),
            Ok,
            "本地已经等于主副本时无需警告"
        );
        assert_eq!(
            local_take_verdict(Some("h1"), None, Some("h1")),
            Ok,
            "上游已删时保留本地不是覆盖上游"
        );
    }

    #[test]
    fn backup_only_when_local_exists_and_differs_from_head() {
        assert!(needs_backup(Some("h2"), Some("h1")));
        assert!(needs_backup(None, Some("h1")));
        assert!(!needs_backup(Some("h1"), Some("h1")));
        assert!(!needs_backup(Some("h1"), None));
        assert!(!needs_backup(None, None));
    }

    #[test]
    fn lost_rel_is_one_clean_agent_component_plus_a_clean_rel() {
        assert_eq!(lost_rel("a", "dir/x.md").unwrap(), "lost/a/dir/x.md");
        assert!(lost_rel("", "x").is_err());
        assert!(lost_rel("..", "x").is_err());
        assert!(lost_rel("a/b", "x").is_err());
        assert!(lost_rel("a\\b", "x").is_err());
        assert!(lost_rel("a", "../x").is_err());
    }
}
