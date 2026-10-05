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

use crate::capabilities::workspace::domain::hash::content_hash;
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

/// 文件级三方比较（提交侧）：`locals` = 路径 → 要写的字节（`None` = 显式删除）。
/// Ok(改动清单) 为空 = 没有可提交的改动；Err = 冲突清单（**整个提交都拒绝**）。
pub fn plan_commit(
    index: &Index,
    tree: &Tree,
    locals: &BTreeMap<String, Option<Vec<u8>>>,
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
            Some(bytes) => {
                let local_hash = content_hash(bytes);
                match (base, head) {
                    (None, None) => changes.push(Change {
                        path: path.clone(),
                        kind: ChangeKind::Add,
                        hash: local_hash,
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
                                hash: local_hash,
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
    /// 共享区有，但本 agent 没拉过。
    NotPulled,
    /// 拉过且本地没改、上游没动。
    Clean,
    /// 拉过之后本地改了。
    LocalModified,
    /// 拉过之后上游又动了。
    UpstreamMoved,
    /// 本地改了、上游也动了。
    Conflict,
}

impl StatusState {
    pub fn as_str(&self) -> &'static str {
        match self {
            StatusState::NotPulled => "未拉取",
            StatusState::Clean => "一致",
            StatusState::LocalModified => "本地有修改",
            StatusState::UpstreamMoved => "上游有更新",
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
        (None, Some(_)) => StatusState::NotPulled,
        (None, None) => StatusState::Clean,
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
            None => StatusState::Clean,
            Some(l) if l == b => StatusState::UpstreamMoved,
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
    /// 本地已改、上游也改了，未覆盖。
    pub conflicts: Vec<String>,
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
    fn locals(entries: &[(&str, Option<&str>)]) -> BTreeMap<String, Option<Vec<u8>>> {
        entries
            .iter()
            .map(|(p, v)| (p.to_string(), v.map(|s| s.as_bytes().to_vec())))
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
                hash: content_hash(b"h2")
            }]
        );
        // 本地没改 → 没得提交。
        let changes = plan_commit(
            &index(&[("f1", &content_hash(b"h2"))]),
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
    }
}
