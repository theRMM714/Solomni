//! 目的：一条路径在"基线 / 上游 / 本地"下的状态，以及状态、拉取、提交、同步的回执形状。
//! 管：`StatusState`（同步面）/ `WorkState`（工作面）/ `status_of` / `work_state_of` / `StatusEntry` / `PullReport` / `CommitRequest` / `CommitReport` / `SyncReport` / `StatusReport` / `WorkLineEntry`。
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

/// 目的：work_status 逐条状态的五态（外加「与主线一致」与「主线有、还没拉取」两种展示态）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkState {
    /// 目的：工作线里有这条路径，且主线头与工作线头一致。
    Committed,
    /// 目的：工作线里有，但主线还没有这一版（`work_push` 之后才会进共享区）。
    Unpushed,
    /// 目的：工作线里有，沙箱里的内容又不是那一版（提交后又改了）。
    Modified,
    /// 目的：工作线与主线都没有，沙箱里新写了。
    Untracked,
    /// 目的：工作线里有，沙箱里被删了（还没提交删除）。
    Deleted,
    /// 目的：主线有、沙箱里还没有（没拉取）。
    NotPulled,
    /// 目的：沙箱内容就是主线这一版，工作线上没有它（拉过没改、也没提交过）。
    Synced,
}

impl WorkState {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkState::Committed => "已提交（与主线一致）",
            WorkState::Unpushed => "已提交（未推送）",
            WorkState::Modified => "提交后又改了",
            WorkState::Untracked => "未提交新增",
            WorkState::Deleted => "未提交删除",
            WorkState::NotPulled => "主线有、还没拉取",
            WorkState::Synced => "与主线一致（没改）",
        }
    }
}

/// 目的：一条路径相对「我的工作线头 / 主线头 / 沙箱」的状态；只用于展示，不参与提交判定。
pub fn work_state_of(work: Option<&str>, main: Option<&str>, local: Option<&str>) -> WorkState {
    match (work, local) {
        (Some(_), None) => WorkState::Deleted,
        (Some(w), Some(l)) => {
            if l != w {
                WorkState::Modified
            } else if main == Some(w) {
                WorkState::Committed
            } else {
                WorkState::Unpushed
            }
        }
        (None, Some(l)) => match main {
            Some(m) if m == l => WorkState::Synced,
            Some(_) => WorkState::Modified,
            None => WorkState::Untracked,
        },
        (None, None) => match main {
            Some(_) => WorkState::NotPulled,
            None => WorkState::Synced,
        },
    }
}

/// 目的：work_status 的逐条结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    pub path: String,
    pub state: WorkState,
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
    /// 目的：什么都没变（上游没动、本地也没动）。
    pub no_change: Vec<String>,
    /// 目的：上游没动、本地改了：沙箱原样保留，未进基线（要发布得 work_commit + work_push）。
    pub local_modified: Vec<String>,
    /// 目的：上游没动、本地删了：保持删除，未进基线。
    pub local_deleted: Vec<String>,
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
    /// 目的：提交时间（由调用方给：本单元不持时钟）。
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

/// 目的：work_push 的请求：squash 进主线的那条提交的说明与转录行。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PushRequest {
    pub message: String,
    /// 目的：提交时间（由调用方给：本单元不持时钟）。
    pub time: i64,
    /// 目的：提交锚的行号（这一席下一条转录行的 id）。
    pub line: u64,
}

/// 目的：work_push 的结果：`commit` 为空 = 没有可推送的改动。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PushReport {
    pub commit: Option<u64>,
    /// 目的：这次落到主线的改动。
    pub changes: Vec<Change>,
    /// 目的：本次推送之后工作线还领先主线几条提交（成功推送 = 0；空结果时如实给出，
    ///   用来说明"没有可推送的改动"是内容已收敛，而不是提交丢了）。
    pub unpushed: usize,
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

/// 目的：一条工作线提交（work_status 的「你的工作线最近提交」用）。
/// 约束：沿调用方工作线的父链取，**不含**别人工作线上的提交；父提交可能是主线提交（工作线分叉点）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkLineEntry {
    pub id: u64,
    /// 目的：父提交 id——「撤回某条提交」与 work_reset to= 的目标都从这里取。
    pub parent: Option<u64>,
    pub time: i64,
    pub message: String,
    /// 目的：这条提交已经在主线里吗（主线提交本身，或已被某条主线提交的 sources 记下）。
    pub pushed: bool,
}

/// 目的：work_status 的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatusReport {
    pub head: Option<CommitSummary>,
    /// 目的：该 agent 工作线当前的提交（与主线不同就是有未推送的内容）。
    pub work_head: Option<CommitSummary>,
    /// 目的：工作线上还没有进主线的提交条数（辅助水位：内容可能已收敛）。
    pub unpushed: usize,
    /// 目的：work_push 会落进主线的净改动路径数（与 work_push 同一次判定）。
    pub pending: usize,
    /// 目的：work_push 会撞上的冲突路径数（>0 时推送会整次被拒）。
    pub conflicts: usize,
    /// 目的：主副本在 head 上的路径清单（有上限；tree_truncated = 如实标注是否截断）。
    pub tree: Vec<String>,
    pub tree_truncated: bool,
    pub entries: Vec<StatusEntry>,
    /// 目的：显式给了 paths 但基线、主副本与沙箱里都没有的路径。
    pub missing: Vec<String>,
    /// 目的：工作区磁盘与 head 不一致的路径（外部改动 / 删除；先 work_sync）。
    pub external: Vec<String>,
    /// 目的：最近几条主线提交（新→旧）。
    pub recent: Vec<CommitSummary>,
    /// 目的：工作线与主线不一致时的最近几条工作线提交（新→旧，带父提交）；
    ///   与主线一致时为空——那种情况下「最近提交」已经够看，不重复列。
    pub work_recent: Vec<WorkLineEntry>,
}
