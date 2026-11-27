//! 目的：冲突解决的判定与"丢失版本"的事实——解决一条路径会发生什么、覆盖前要不要备份。
//! 管：`ResolveRequest` / `ResolveReport` / `ResolveTake` / `local_take_verdict` / `needs_backup` / `LostEntry` / `LostReport`。
//! 不管：改写工作区文件与写备份（`workspace/service` 与 `workspace::detail`）。
//! 联动：`work_resolve` 与 `work_restore_lost` 的入参与回执（docs/workspace/README.md）。

use crate::units::business::versions::domain::asset::FileVersion;

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
                "本地没有这个文件；take=local 只推进基线、不写沙箱。主线没改过这个路径时，提交时用 deletes 显式列出就能把它从共享区删掉；主线改过就只能 take=upstream 接受新版，或用 work_reset 撤回那条删除"
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

/// 目的：一份被覆盖/删除的本地版本（`work_lost` 的一条）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LostEntry {
    /// 目的：共享区相对路径（原路径；恢复也用它）。
    pub path: String,
    /// 目的：备份在 `lost/` 下的存储位置（`@<来源>/<原路径>`；同名多份备份时用它点名）。
    pub backup: String,
    /// 目的：备份的产点（`resolve` / `reset-paths` / `delete-3` / `archive-2` …）；旧备份没标记 = None。
    pub origin: Option<String>,
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
    /// 目的：恢复到的共享区相对路径（原路径）。
    pub path: String,
    /// 目的：用的是哪一份备份（`@<来源>/<原路径>`）。
    pub backup: String,
    /// 目的：恢复的备份内容指纹。
    pub hash: String,
    /// 目的：恢复的备份内容字节数。
    pub size: u64,
    /// 目的：覆盖前沙箱同路径的指纹；之前不存在 = None。
    pub previous: Option<String>,
}
