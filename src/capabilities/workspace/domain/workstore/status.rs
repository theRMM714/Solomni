//! 目的：一条路径在"基线 / 上游 / 本地"下的状态，以及状态、拉取、提交、同步的回执形状。
//! 管：`StatusState` / `status_of` / `StatusEntry` / `PullReport` / `CommitRequest` / `CommitReport` / `SyncReport` / `StatusReport`。
//! 不管：判定本身（提交侧在 `merge.rs`、解决侧在 `resolve.rs`）；回执渲染在呈现层。
//! 联动：`work_status` / `work_pull` / `work_commit` / `work_sync` 的入参与回执（docs/workspace/README.md）。

use super::*;

/// 目的：一条路径在 agent 眼里的状态。
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
    /// 拉过、上游没动，但沙箱里的文件被删了（提交删除要显式放进 deletes）。
    LocalDeleted,
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
            StatusState::LocalDeleted => "本地已删除",
            StatusState::UpstreamMoved => "上游有更新",
            StatusState::UpstreamDeleted => "上游已删除",
            StatusState::Conflict => "冲突",
        }
    }
}

/// 目的：work_status 的逐条结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    pub path: String,
    pub state: StatusState,
}

/// 目的：一条路径在"基线 / 上游 / 本地"下的状态（只读展示用，与提交判定分开）。
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
                    StatusState::LocalDeleted
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

/// 目的：work_pull 的逐条结果（调用方据此渲染回执）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PullReport {
    /// 目的：拉取后到的主副本提交点。
    pub commit: u64,
    /// 目的：写进沙箱并更新基线的路径。
    pub updated: Vec<String>,
    /// 目的：沙箱里已经是主副本这一版：只更新基线。
    pub already: Vec<String>,
    /// 目的：什么都没变（上游没动）。
    pub no_change: Vec<String>,
    /// 目的：沙箱里已有未跟踪 / 无关的同名文件，未覆盖。
    pub occupied: Vec<String>,
    /// 目的：本地已改、上游也改了，未覆盖；逐条带冲突类型。
    pub conflicts: Vec<ConflictAt>,
    /// 目的：主副本里没有这些路径。
    pub missing: Vec<String>,
    /// 目的：工作区磁盘内容与 head 不一致（外部改动或删除）；这些路径未拉取，先 `work_sync`。
    pub external: Vec<String>,
}

/// 目的：一次提交的请求（收口成对象：路径 + 显式删除 + 说明 + 时间）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommitRequest {
    pub paths: Vec<String>,
    pub deletes: Vec<String>,
    pub message: String,
    /// 目的：提交时间（由调用方给：本能力不持时钟）。
    pub time: i64,
    /// 目的：提交锚的行号（调用方给：这一席下一条转录行的 id）。
    pub line: u64,
}

/// 目的：work_commit 的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitReport {
    pub id: u64,
    pub changes: Vec<Change>,
}

/// 目的：`work_sync` 的结果——把工作区当前内容采纳成一条 author=external 的提交。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// 目的：采纳后的新 head；没有可同步的改动 = None。
    pub commit: Option<u64>,
    /// 目的：采纳的变更（新增 / 修改 / 删除）。
    pub changes: Vec<Change>,
    /// 目的：磁盘与 head 一致、无需变更的路径。
    pub unchanged: Vec<String>,
    /// 目的：请求里工作区与版本库都没有的路径。
    pub missing: Vec<String>,
}

/// 目的：work_status 的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatusReport {
    pub head: Option<CommitSummary>,
    /// 目的：主副本在 head 上的路径清单（有上限；tree_truncated = 如实标注是否截断）。
    pub tree: Vec<String>,
    pub tree_truncated: bool,
    pub entries: Vec<StatusEntry>,
    /// 目的：显式给了 paths 但基线、主副本与沙箱里都没有的路径。
    pub missing: Vec<String>,
    /// 目的：沙箱里有、基线与主副本都没有的路径（下一次提交会成为新增；有上限）。
    pub local_only: Vec<String>,
    /// 目的：`local_only` 是否被截断（如实标注，不假装列全）。
    pub local_only_truncated: bool,
    /// 目的：工作区磁盘与 head 不一致的路径（外部改动 / 删除；先 work_sync）。
    pub external: Vec<String>,
    /// 目的：最近几条提交（新→旧）。
    pub recent: Vec<CommitSummary>,
}
