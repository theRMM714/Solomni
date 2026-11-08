//! 入站能力面：**其它能力与呈现层只准用这里**（不许碰 `domain` / `ports`）。

pub use crate::capabilities::workspace::domain::asset::{
    media_hint, FileVersion, MAX_ASSET_BYTES, VERSION_TEXT_LIMIT,
};
pub use crate::capabilities::workspace::domain::base::CommitRefusal;
pub use crate::capabilities::workspace::domain::hash::content_hash;
pub use crate::capabilities::workspace::domain::readfact::ReadFact;
pub use crate::capabilities::workspace::domain::workspace::{
    safe_file_name, AreaUsage, Place, Sandbox, Sandboxes, WorkFiles, WorkRoots, WorkUsage,
};
pub use crate::capabilities::workspace::domain::workstore::{
    Change, ChangeKind, CommitReport, CommitRequest, CommitSummary, GcReport, LostEntry,
    LostReport, PullReport, ResolveAction, ResolveEntry, ResolveReport, ResolveRequest,
    ResolveTake, ResolveWarning, ResolveWarningKind, RestoreLostReport, StatusEntry, StatusReport,
    SyncReport, VersionView,
};

/// 目的：一条待判定的读取（一次 `read` 读到的行区间 + 当时的整份指纹）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadQuery {
    /// 目的：工作树内的相对路径（斜杠分隔）。
    pub path: String,
    /// 目的：读的那一刻整份内容的指纹。
    pub blob: String,
    /// 目的：读到的第一行（从 1 数起）。
    pub from: u64,
    /// 目的：读到的最后一行（含）。
    pub to: u64,
}

/// 目的：沙箱里改了、却没被采纳的一条路径（模型据此先 pull 再重做）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldChange {
    /// 目的：工作树内的相对路径。
    pub path: String,
    /// 目的：不采纳的原因（给模型看的一句话）。
    pub why: String,
}

/// 目的：一次沙箱采纳的结果：一次提交只带一批改动；没有可采纳的改动时 `commit = None`。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdoptReport {
    /// 目的：采纳后的新提交号；None = 沙箱与主线一致。
    pub commit: Option<u64>,
    /// 目的：已采纳的路径（新增 / 修改 / 删除）。
    pub changed: Vec<String>,
    /// 目的：沙箱改了但基线过期 / 未拉取 / 三方冲突的路径（没有采纳）。
    pub held: Vec<HeldChange>,
}

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
    /// 目的：把根会话的工作区绑定落成布局投影（`Some` = 外部工作区根；`None` = 产品内布局）。
    /// 约束：会话 meta 是绑定的唯一真相；本调用只刷新会话目录里的布局投影，不碰工作区文件。
    fn bind(&self, session: &str, root: Option<&std::path::Path>) -> Result<(), String>;
    /// 准备工作区：建 session/<工作名>/work 与每个 agent 的沙箱目录。
    fn prepare(&self, session: &str, agents: &[String]) -> Result<(), String>;
    /// 撤销一次失败的准备：只删本产品建出的空沙箱 / 空 work/、布局投影与版本库 `.work`。
    /// 约束：调用方只在会话**从未落盘**时调用；用户文件绝不因为一次失败的创建被删。
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
    /// **自动提交**：工具写完之后提交，只接受快进——待提交路径的基线必须等于主线当前头。
    /// 约束：不 pull、不合并、不降级；基线过期即 `CommitRefusal::Stale`，调用方只能上报或让模型先 pull。
    /// `Ok(None)` = 内容与主线一致、没有可发布的改动（不算失败）。
    fn work_commit_auto(
        &self,
        work: &str,
        agent: &str,
        session: &str,
        req: &CommitRequest,
    ) -> Result<Option<CommitReport>, CommitRefusal>;
    /// **外部同步**：读工作区磁盘，把与 head 不同的路径采纳成一条 `author=external` 的提交。
    /// 约束：不改任何沙箱与工作区内容；同路径沙箱里有未提交改动时留给后续提交判冲突。
    fn work_sync(&self, work: &str, paths: &[String], time: i64) -> Result<SyncReport, String>;
    /// 目的：把当前 head 树里所有受管路径的外部改动一次性采纳（确认覆盖前的保护动作）。
    /// 约束：无受管路径 = 无操作；不碰未跟踪文件。
    fn work_sync_tracked(&self, work: &str, time: i64) -> Result<SyncReport, String>;
    /// 目的：首次绑定外部工作区时把工作区现有文件导入成初始提交（有上限、忽略 VCS / 依赖目录）。
    /// 约束：已有 head 时无操作；单文件超限 / 总上限截断时只导入可导入的部分。
    fn work_import_root(&self, work: &str, time: i64) -> Result<SyncReport, String>;
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
    /// **沙箱采纳**：把该 agent 沙箱里不是 write / edit / patch 落下的改动（shell、模块工具、外部写入）
    /// 采纳成一条 `source=external` 的提交，锚在这条转录行上。
    /// 约束：只收快进——基线过期、未拉取、三方冲突的路径不采纳，逐条放进 `held` 交调用方回执；
    ///   不改任何沙箱内容；外部工作区不采纳（它的真相是磁盘，走 `work_sync`）。
    fn work_adopt_sandbox(
        &self,
        work: &str,
        agent: &str,
        session: &str,
        time: i64,
        line: u64,
    ) -> Result<AdoptReport, String>;
    /// **读取是否过期**：一批读取事实（工作树、路径、行区间、当时的指纹）逐个判定。
    /// 返回：与 `queries` 同序的布尔值；true = 读过的行在主副本里已经变了（这段内容该被替换成占位符）。
    /// 约束：判据是**内容**不是时间——路径没了算过期；指纹相同算不过期；指纹不同才比行区间。
    ///   旧版本从内容寻址对象库里取；对象缺失时按过期处理（宁可供模型重读，不供奉可能错的旧内容）。
    fn work_stale_reads(&self, work: &str, queries: &[ReadQuery]) -> Result<Vec<bool>, String>;
    /// 目的：把主副本物化回某个提交点（只改共享区，不动各会话）。
    /// 约束：非破坏性——目标之后的提交仍保留，可再次回滚。
    fn work_restore(&self, work: &str, commit: u64) -> Result<(), String>;
    /// **确认后覆盖**：与 `work_restore` 相同，但允许覆盖工作区里被外部改过的受管路径。
    /// 约束：只给用户经危险确认后的动作；权限与生成中检查仍在调用方。
    fn work_restore_force(&self, work: &str, commit: u64) -> Result<(), String>;
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
