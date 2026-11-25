//! 目的：工作区能力的入站能力面——其它能力与呈现层只准用这里（不许碰 `domain` / `ports`）。
//! 管：`Workspace` 用例 trait 与它的视图类型（沙箱寻址、文件视图、提交 / 拉取 / 还原 / 状态 / 回收），并重导出领域共享类型。
//! 不管：用例实现（在 `service/`）；寻址与版本化的纯规则（在 `domain/`）；落盘机制（在 `detail/`）。
//! 联动：实现见 `service.rs`；`tools` 借 `Place` / `Sandbox` 定位读写落点。

pub use crate::units::workspace::domain::asset::{
    FileVersion, MAX_ASSET_BYTES, VERSION_TEXT_LIMIT, media_hint,
};
pub use crate::units::workspace::domain::freshness::ReadVerdict;
pub use crate::units::workspace::domain::hash::{Sha256, content_hash};
/// 目的：工作区过滤的唯一名单与匹配规则——`tools` 的 grep 与内容面的导入 / 清单都只从这里取。
#[allow(unused_imports)] // 入站面：规则类型给调用方与测试取用
pub use crate::units::workspace::domain::path_filter::{
    MAX_RULE_BYTES, MAX_RULE_CHARS, MAX_RULE_LINES, Rules, WORKIGNORE, WORKIGNORE_SEED, check_rule,
};
pub use crate::units::workspace::domain::readfact::ReadFact;
/// 目的：共享区路径归属（读 / 写范围与黑名单）——\`permission\` 只管档位与网络，归属规则在这里。
#[allow(unused_imports)] // 入站面：规则类型给调用方与测试取用
pub use crate::units::workspace::domain::scope::{
    ScopeMode, SharedEdit, SharedScope, hit, normalize, validate_entry,
};
/// 目的：`work_*` 工具的名字、取参与回执拼装——`tools` 与 `turn` 只从这里取，不碰 domain。
#[allow(unused_imports)]
pub use crate::units::workspace::domain::work_tools::{
    COMMIT, DIFF, IGNORE, IgnoreReport, LOST, PULL, PUSH, RESET, RESOLVE, STATUS, SYNC, VERSIONS,
    WORK_TOOLS, is_work_tool, render_commit, render_diff, render_ignore, render_lost, render_pull,
    render_push, render_reset, render_resolve, render_restore_lost, render_status, render_sync,
    render_versions, str_arg, str_list,
};
pub use crate::units::workspace::domain::workspace::{
    AreaUsage, Place, Sandbox, Sandboxes, WorkFiles, WorkRoots, WorkUsage, safe_file_name,
};
/// 目的：版本库共享类型（提交 / 拉取 / 推送 / 状态 / 差异 / 回退）——`tools`、呈现层与测试只从这里取。
/// 约束：`WorkState` 等类型只在测试与呈现层命名，本 crate 的实现不直接用，故整组允许未使用告警。
#[allow(unused_imports)]
pub use crate::units::workspace::domain::workstore::{
    Change, ChangeKind, CommitAnchor, CommitReport, CommitRequest, CommitSummary, DiffReport,
    DiffRequest, DiffSign, LostEntry, LostReport, PullReport, PushReport, PushRequest, ResetReport,
    ResetRequest, ResolveAction, ResolveEntry, ResolveReport, ResolveRequest, ResolveTake,
    ResolveWarning, ResolveWarningKind, RestoreLostReport, StatusEntry, StatusReport, SyncReport,
    VersionView, WorkGcReport, WorkLineEntry, WorkState,
};

/// 目的：外部工作区根的策略（判定 / 创建校验 / 加载复检）——`runtime` 建会话与重开时只经这里取。
#[allow(unused_imports)] // 入站面：策略函数给调用方与测试取用
pub use crate::units::workspace::service::roots::{
    ambient, policy, product_root, recheck, resolve,
};

/// 目的：`work_*` 工具执行者——`turn` 装配工具面时按席位面发放的工具 id 现取。
#[allow(unused_imports)]
pub use crate::units::workspace::service::work_tools::{WorkHandler, work_handlers};

/// 目的：写入门禁用的一份版本身份：只关心"这份内容是不是同一份"，不返回正文。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GateFile {
    /// 目的：整份内容的 SHA-256（与 `ReadFact.blob` / `Change.hash` 同源）。
    pub hash: String,
    /// 目的：文件字节数；来自对象树的基线版本可能没有大小（门禁只用 hash）。
    pub size: Option<u64>,
}

/// 目的：一次写入门禁查询的结果：路径是否受管、当前内容与基线版本各是什么。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathGateInfo {
    /// 目的：工作树内的相对路径（斜杠分隔）。
    pub path: String,
    /// 目的：主线 head 树或该 agent 工作线 head 树里有这个路径（受管）。
    /// 约束：受管才要求观察；新建 / 构建产物 / scratch 不受门禁。
    pub tracked: bool,
    /// 目的：沙箱当前内容的版本；文件不存在（含已删除）= None。
    pub current: Option<GateFile>,
    /// 目的：受管版本的基线：优先该 agent 工作线 head，其次主线 head；都没有 = None。
    pub base: Option<GateFile>,
}

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
    /// 目的：这次读发生在该 agent 的私有沙箱里——对照物是沙箱当前内容，不是主副本。
    /// 约束：沙箱里已经没这条路径 = 过期（读到的内容不在了，模型该重读）。
    pub sandbox: bool,
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

/// 目的：一次留档（全局检查点）的结果：主副本与各沙箱一起物化到留档点。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchiveReport {
    /// 目的：档案号（转录标记与 refs/archive/<id> 共用）。
    pub mark: u64,
    /// 目的：留档前的主副本头（None = 空共享区）。
    pub shared: Option<u64>,
    /// 目的：物化到的提交点（None = 清空）。
    pub point: Option<u64>,
    /// 目的：各工作树备份进 lost/ 的未提交文件数（agent 名 → 条数）。
    pub backed_up: std::collections::BTreeMap<String, usize>,
}

/// 目的：一次删除硬回退的结果：主副本与各沙箱一起落到的提交点 + 各工作树备份条数。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeleteReport {
    /// 目的：物化到的提交点（None = 清空）。
    pub point: Option<u64>,
    /// 目的：各工作树备份进 lost/ 的未提交文件数（agent 名 → 条数）。
    pub backed_up: std::collections::BTreeMap<String, usize>,
    /// 目的：这次一并作废的留档号。
    pub pruned: Vec<u64>,
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
    /// 目的：这条提交已经在主线里了吗（工作线提交未推送时 false）。
    pub pushed: bool,
}

/// 目的：某个工作的提交历史视图（分页；当前 head 单独给出）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct WorkLog {
    pub head: Option<CommitSummary>,
    pub commits: Vec<WorkLogEntry>,
    /// 目的：各 agent 工作线上的本地提交（新→旧，带 pushed 标记）：回答「我提交了什么、推了没有」。
    pub work: Vec<WorkLogEntry>,
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

/// 目的：工作区的**用例面**（`service.rs` 实现）：别的能力要工作区目录与版本化，走这里。
/// 约束：出站端口（`Workdirs` / `WorkStore`）**只由它持有**（R12）。
pub trait Workspace: Send + Sync {
    /// 目的：把根会话的工作区绑定落成布局投影（`Some` = 外部工作区根；`None` = 产品内布局）。
    /// 约束：会话 meta 是绑定的唯一真相；本调用只刷新会话目录里的布局投影，不碰工作区文件。
    fn bind(&self, session: &str, root: Option<&std::path::Path>) -> Result<(), String>;
    /// 按会话铺布局：建 session/<工作名>/work 与每个 agent 的沙箱目录。
    /// 约束：只落目录与版本库种子，不产生会话 / 工作名 / 通道 / guest——建工作的唯一入口在 work。
    fn ensure_layout(&self, session: &str, agents: &[String]) -> Result<(), String>;
    /// 撤销一次失败的铺布局：只删本产品建出的空沙箱 / 空 work/、布局投影与版本库 `.work`。
    /// 约束：调用方只在会话**从未落盘**时调用；用户文件绝不因为一次失败的创建被删。
    fn discard_layout(&self, session: &str, agents: &[String]) -> Result<(), String>;
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
    /// **推送**：把工作线上已提交的改动三方合并进主线（`base` = 我这条线与主线的**分叉点**提交树，
    ///   分叉点之后我拉过却没在工作线提交过的路径按拉取基线补录；`mine` = 我的工作线头，`theirs` = 主线头），
    ///   干净就把结果 squash 成主线一条提交、物化共享区、把工作线快进过去。
    /// 约束：不重写工作线历史；冲突整次拒绝并逐条点名（先 work_pull 再合并重推）。
    fn work_push(
        &self,
        work: &str,
        agent: &str,
        session: &str,
        req: &PushRequest,
    ) -> Result<PushReport, String>;
    /// **存档**：工具写完之后把沙箱改动记进该 agent 的存档线（`refs/agents/<agent>/saves`）。
    /// 约束：这不是发布——共享区与 `refs/main` 一个字都不动，改动要进主线必须再 work_commit；
    ///   存档无准入门槛（不进主线就不会覆盖别人的内容）。`Ok(None)` = 与上次存档一致（不算失败）。
    fn work_commit_auto(
        &self,
        work: &str,
        agent: &str,
        session: &str,
        req: &CommitRequest,
    ) -> Result<Option<CommitReport>, String>;
    /// **外部同步**：读工作区磁盘，把与 head 不同的路径采纳成一条 `author=external` 的提交。
    /// 约束：不改任何沙箱与工作区内容；同路径沙箱里有未提交改动时留给后续提交判冲突。
    ///   `anchor` 是这次同步的转录行位置——回档/留档按它决定这条同步在不在回档点之前。
    fn work_sync(
        &self,
        work: &str,
        paths: &[String],
        time: i64,
        anchor: CommitAnchor,
    ) -> Result<SyncReport, String>;
    /// 目的：把当前 head 树里所有受管路径的外部改动一次性采纳（确认覆盖前的保护动作）。
    /// 约束：无受管路径 = 无操作；不碰未跟踪文件。
    fn work_sync_tracked(
        &self,
        work: &str,
        time: i64,
        anchor: CommitAnchor,
    ) -> Result<SyncReport, String>;
    /// 目的：首次绑定外部工作区时把工作区现有文件导入成初始提交（有上限、忽略 VCS / 依赖目录）。
    /// 约束：已有 head 时无操作；单文件超限 / 总上限截断时只导入可导入的部分。
    fn work_import_root(&self, work: &str, time: i64) -> Result<SyncReport, String>;
    /// **过滤名单**：读写本工作的 `.workignore`（`op` = list / add / remove）。
    /// 约束：规则只决定**哪些路径算工作区内容**（首次导入、状态里的未跟踪清单、提交展开与 grep）；
    ///   显式点名的单个文件照样可读写，已跟踪的文件不会因为新规则消失。只动版本库目录，不碰 `.gitignore`。
    fn work_ignore(
        &self,
        work: &str,
        op: &str,
        patterns: &[String],
    ) -> Result<IgnoreReport, String>;
    /// 目的：把该工作未完成的版本库事务重放到一致状态；没有待恢复事务 = 无操作。
    fn work_recover(&self, work: &str) -> Result<(), String>;
    /// 目的：回收该工作中不再可达的内容对象；apply=false 只统计不删除。
    fn work_gc(&self, work: &str, apply: bool) -> Result<WorkGcReport, String>;
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
    /// **差异**：两个版本的行级差异（`from` 缺省 = 工作线头，`to` 缺省 = 沙箱当前内容）。
    /// 约束：二进制与超过 `DIFF_MAX_BYTES` 的文件只报变更类型，不展开 hunk；只读。
    fn work_diff(&self, work: &str, agent: &str, req: &DiffRequest) -> Result<DiffReport, String>;
    /// **回退**：`paths` = 把沙箱这些路径恢复成工作线头的版本（同 `git restore`）；
    ///   `to` = 把工作线移回那条提交（同 `git reset`），`keep_changes` 时沙箱不动。
    /// 约束：`to` 必须在这条工作线的祖先链上；覆盖 / 删除前把旧内容备份进该 agent 的 lost/。
    fn work_reset(
        &self,
        work: &str,
        agent: &str,
        req: &ResetRequest,
    ) -> Result<ResetReport, String>;
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
    /// **写入门禁**：一批路径的受管状态、当前版本与基线版本；只读，不返回正文。
    /// 约束：只做快照；P0 不在同一次锁内完成"检查 + 写入"（P2 由 `SysIo::mutate` 收口）。
    fn work_gate_info(
        &self,
        work: &str,
        agent: &str,
        paths: &[String],
    ) -> Result<Vec<PathGateInfo>, String>;

    /// **读取相对当前版本的判决**：一批读取事实（工作树、路径、行区间、当时的指纹）逐个判定。
    /// 返回：与 `queries` 同序的 `ReadVerdict`——SameVersion = 指纹相同；Shifted = 版本变了但那段
    ///   内容还在；Changed = 那段内容被改过或删掉了。
    /// 约束：判据是**内容**不是时间——路径没了算 Changed；指纹相同算 SameVersion；指纹不同才比行区间。
    ///   旧版本从内容寻址对象库里取；对象缺失时按 Changed 处理（宁可供模型重读，不供奉可能错的旧内容）。
    ///   消费者策略不同：占位符用 `is_stale()`（只换 Changed），写门禁只认 SameVersion。
    fn judge_reads(
        &self,
        work: &str,
        agent: &str,
        queries: &[ReadQuery],
    ) -> Result<Vec<ReadVerdict>, String>;
    /// 目的：把主副本物化回某个提交点（只改共享区，不动各会话）。
    /// 约束：非破坏性——目标之后的提交仍保留，可再次回滚。
    fn work_restore(&self, work: &str, commit: u64) -> Result<(), String>;
    /// **确认后覆盖**：与 `work_restore` 相同，但允许覆盖工作区里被外部改过的受管路径。
    /// 约束：只给用户经危险确认后的动作；权限与生成中检查仍在调用方。
    fn work_restore_force(&self, work: &str, commit: u64) -> Result<(), String>;
    /// 共享区当前提交点（留档记 after 用）。
    fn work_head(&self, work: &str) -> Result<Option<u64>, String>;
    /// 仅测试用：物化到"各会话都还没越过自己保留行"的最后一个提交点（None = 空共享区）。
    #[cfg(test)]
    fn work_rewind_to(
        &self,
        work: &str,
        keep_by_agent: &std::collections::BTreeMap<String, u64>,
    ) -> Result<Option<u64>, String>;
    /// **留档全局检查点**：算出留档点，把主副本与列出的全部沙箱一起物化过去，
    /// 各沙箱不一致的未提交内容先备份进 lost/，并记 `refs/archive/<mark>`。
    /// 返回：留档前的主副本头与各工作树的备份条数。
    /// 约束：`keep_by_agent` 与 `work_rewind_to` 同义；只动文件与引用，不删提交；
    ///   外部工作区会先做覆盖检查，有外部改动就整次拒绝。
    fn work_archive(
        &self,
        work: &str,
        keep_by_agent: &std::collections::BTreeMap<String, u64>,
        agents: &[String],
        mark: u64,
    ) -> Result<ArchiveReport, String>;
    /// **恢复档案**：主副本与档案里的工作树一起物化回档案头，各 agent 的工作线放回档案记的头，
    ///   再丢弃留档时刻（`since`）及之后产生的全部提交——恢复 = 回到留档操作前，长出来的分叉作废。
    /// 返回：恢复到的提交点（None = 清空）。
    /// 约束：档案缺 `since` 或目标提交已不在时整次拒绝；被丢弃提交的引用（含更晚的档案）一并清掉；
    ///   档案引用用完即移除。
    fn work_restore_archive(&self, work: &str, mark: u64) -> Result<Option<u64>, String>;
    /// **删除的硬回退**：主副本与列出的全部沙箱一起落到删除点，再丢掉该点之后的提交。
    /// 约束：沙箱里与删除点不一致的内容先备份进 lost/；凡是提交记录被丢掉的留档引用
    ///   （`refs/archive/*` 不在 `keep_marks` 里的）一并作废；外部改动按覆盖检查整次拒绝。
    fn work_delete(
        &self,
        work: &str,
        keep_by_agent: &std::collections::BTreeMap<String, u64>,
        agents: &[String],
        keep_marks: &[u64],
    ) -> Result<DeleteReport, String>;
    /// 现存留档号（`refs/archive/*`），供删除前报出会作废几个。
    fn work_archives(&self, work: &str) -> Result<Vec<u64>, String>;
    /// 仅测试用：把主副本物化到某个提交点（None = 清空），不改提交记录；生产走留档/恢复。
    #[cfg(test)]
    fn work_restore_point(&self, work: &str, commit: Option<u64>) -> Result<(), String>;
    /// 仅测试用：丢弃不是 keep 祖先的提交记录并物化到 keep。
    #[cfg(test)]
    fn work_discard_after(&self, work: &str, keep: Option<u64>) -> Result<(), String>;
}
