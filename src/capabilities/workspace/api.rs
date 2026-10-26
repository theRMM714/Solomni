//! 入站能力面：**其它能力与呈现层只准用这里**（不许碰 `domain` / `ports`）。

pub use crate::capabilities::workspace::domain::asset::{
    media_hint, FileVersion, MAX_ASSET_BYTES, VERSION_TEXT_LIMIT,
};
pub use crate::capabilities::workspace::domain::hash::content_hash;
pub use crate::capabilities::workspace::domain::workspace::{
    safe_file_name, AreaUsage, Place, Sandbox, Sandboxes, WorkFiles, WorkRoots, WorkUsage,
};
pub use crate::capabilities::workspace::domain::workstore::{
    Change, ChangeKind, CommitReport, CommitRequest, CommitSummary, GcReport, LostEntry,
    LostReport, PullReport, ResolveAction, ResolveEntry, ResolveReport, ResolveRequest,
    ResolveTake, ResolveWarning, ResolveWarningKind, RestoreLostReport, StatusEntry, StatusReport,
    VersionView,
};

/// 目的：提交历史的一条（不含整棵树，列表可安全分页）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct WorkLogEntry {
    pub id: u64,
    pub parent: Option<u64>,
    pub author: String,
    pub session: String,
    pub time: i64,
    pub message: String,
    /// 目的：这次提交改了几条路径。
    pub changes: usize,
    /// 目的：这条提交在**当前 head 的祖先链**上吗？用户回滚离开过的提交仍保留，但会标 false。
    pub on_head: bool,
}

/// 目的：某个工作的提交历史视图（分页；当前 head 单独给出）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct WorkLog {
    pub head: Option<CommitSummary>,
    pub commits: Vec<WorkLogEntry>,
    /// 目的：该工作现存提交总数（含已离开当前线的）。
    pub total: usize,
    /// 目的：还有更早的提交没放进这一页。
    pub truncated: bool,
}

/// 目的：提交详情里的一条改动（带对象大小与媒体提示）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ChangeView {
    pub path: String,
    pub kind: ChangeKind,
    pub hash: String,
    /// 目的：删除 = 0；新增/修改 = 该对象字节数。
    pub size: u64,
    pub media_type: Option<String>,
}

/// 目的：一条提交的详情（改动清单有上限，超出如实标注）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CommitDetail {
    pub id: u64,
    pub parent: Option<u64>,
    pub author: String,
    pub session: String,
    pub time: i64,
    pub message: String,
    pub changes: Vec<ChangeView>,
    pub changes_truncated: bool,
}

/// 目的：一条提交里某个文件的内容预览（文本最多 VERSION_TEXT_LIMIT；二进制只给元信息）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CommitFileView {
    pub path: String,
    pub hash: String,
    pub size: u64,
    pub media_type: Option<String>,
    pub text: Option<String>,
    pub truncated: bool,
    pub too_large: bool,
}

/// 工作区的**用例面**（`service.rs` 实现）：别的能力要工作区目录与版本化，走这里；
/// 出站端口（`Workdirs` / `WorkStore`）**只由它持有**（R12）。
pub trait Workspace: Send + Sync {
    /// 准备工作区：建 session/<工作名>/work 与每个 agent 的沙箱目录。
    fn prepare(&self, session: &str, agents: &[String]) -> Result<(), String>;
    /// 撤销一次准备：只删**空**目录（有什么都不删）——建会话失败时用。
    fn discard(&self, session: &str, agents: &[String]) -> Result<(), String>;
    /// work/ 下是否已有同名文件（上传同名冲突判定）。
    fn work_has(&self, session: &str, name: &str) -> bool;
    /// 列出本工作可引用的文件（work/ 与各 agent 沙箱；相对路径、/ 分隔、排序稳定）。
    fn files(&self, session: &str, agents: &[String]) -> Result<WorkFiles, String>;
    /// 统计工作区用量（共享区 + 各 agent 沙箱的文件数与总字节）：删除前如实交代用。
    fn usage(&self, session: &str, agents: &[String]) -> Result<WorkUsage, String>;
    /// 沙箱寻址根（work 与各 agent 私有区）：布局机制在适配层，拼接与越界校验在本能力。
    fn roots(&self, session: &str, agents: &[String]) -> Result<WorkRoots, String>;

    /// **拉取**：把主副本（head）的路径写进该 agent 的沙箱同相对路径，并更新它的拉取基线。
    /// 本地已改且上游也改的路径**不覆盖**，只报冲突；中途失败时保留已完成部分的基线。
    fn work_pull(&self, work: &str, agent: &str, paths: &[String]) -> Result<PullReport, String>;
    /// **提交**：文件级三方比较（基线 / 沙箱 / head）。任一条冲突 → 整个提交拒绝、逐条点名。
    fn work_commit(
        &self,
        work: &str,
        agent: &str,
        session: &str,
        req: &CommitRequest,
    ) -> Result<CommitReport, String>;
    /// 目的：把该工作未完成的版本库事务重放到一致状态；没有待恢复事务 = 无操作。
    fn work_recover(&self, work: &str) -> Result<(), String>;
    /// 目的：回收该工作中不再可达的内容对象；apply=false 只统计不删除。
    fn work_gc(&self, work: &str, apply: bool) -> Result<GcReport, String>;
    /// **用户投喂**：一次权威提交（作者 = user），不做冲突拦截；返回提交号。
    fn work_commit_user(
        &self,
        work: &str,
        path: &str,
        bytes: &[u8],
        time: i64,
        line: u64,
    ) -> Result<u64, String>;
    /// **状态**：head、主副本在 head 上的路径清单、以及逐条基线状态；`paths` 省略 = 全部相关路径。
    fn work_status(
        &self,
        work: &str,
        agent: &str,
        paths: &[String],
    ) -> Result<StatusReport, String>;
    /// 目的：一条路径的三方版本（基线 / 主副本 / 沙箱内容或指纹）+ 主副本提交 id。
    /// 约束：文本有上限预览；二进制只给指纹与大小；truncated 时不能据此合并。
    fn work_versions(&self, work: &str, agent: &str, path: &str) -> Result<VersionView, String>;
    /// 目的：提交历史（新→旧，可按 id 分页）+ 当前 head + 每条是否在当前祖先链上。
    /// 约束：只读；不返回整棵树或文件内容。
    fn work_log(&self, work: &str, limit: usize, before: Option<u64>) -> Result<WorkLog, String>;
    /// 目的：一条提交的详情（改动清单、对象大小与媒体提示）。
    fn work_commit_view(&self, work: &str, commit: u64) -> Result<CommitDetail, String>;
    /// 目的：该提交里某个文件的内容预览；路径必须真在这条提交的树里。
    fn work_commit_file(
        &self,
        work: &str,
        commit: u64,
        path: &str,
    ) -> Result<CommitFileView, String>;
    /// 目的：解决冲突——取主副本覆盖本地，或保留本地并把基线推进到主副本。
    /// 约束：不做自动合并；take=local 要带 expect_head（主副本提交 id），且本地与基线一致时拒绝；
    ///       take=upstream 覆盖/删除本地文件前先备份到版本库 lost/ 下。
    fn work_resolve(
        &self,
        work: &str,
        agent: &str,
        req: &ResolveRequest,
    ) -> Result<ResolveReport, String>;
    /// 目的：列出该 agent 被 take=upstream 覆盖/删除的本地版本；path 省略 = 全部。
    fn work_lost(&self, work: &str, agent: &str, path: Option<&str>) -> Result<LostReport, String>;
    /// 目的：把一份备份复制回沙箱同路径，并校验 expect_current（文件不存在传空串）。
    /// 约束：恢复不消费备份；不匹配就拒绝，避免覆盖后来写的新内容。
    fn work_restore_lost(
        &self,
        work: &str,
        agent: &str,
        path: &str,
        expect_current: &str,
    ) -> Result<RestoreLostReport, String>;
    /// 目的：把主副本物化回某个提交点（只改共享区，不动各会话）。
    /// 约束：非破坏性——目标之后的提交仍保留，可再次回滚。
    fn work_restore(&self, work: &str, commit: u64) -> Result<(), String>;
    /// 共享区当前提交点（留档记 after 用）。
    fn work_head(&self, work: &str) -> Result<Option<u64>, String>;
    /// 物化到"各会话都还没越过自己保留行"的最后一个提交点；返回选中的提交（None = 空共享区）。
    fn work_rewind_to(
        &self,
        work: &str,
        keep_by_agent: &std::collections::BTreeMap<String, u64>,
    ) -> Result<Option<u64>, String>;
    /// 物化到某个提交点（None = 清空），不改提交记录（恢复用）。
    fn work_restore_point(&self, work: &str, commit: Option<u64>) -> Result<(), String>;
    /// 丢弃不是 keep 祖先的提交记录并物化到 keep（删除 / 恢复的"丢弃历史"）。
    fn work_discard_after(&self, work: &str, keep: Option<u64>) -> Result<(), String>;
}
