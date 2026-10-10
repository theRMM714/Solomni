//! 入站能力面：**其它能力与呈现层只准用这里**（不许碰 `domain` / `ports`）。

pub use crate::capabilities::workspace::domain::exec::{
    absent, declared, diagnose_text, plan, plan_summary, tier_readiness, tier_refusal, unavailable,
    vm_diagnoses, vm_requirements, Diagnosis, ExecSpec, VmInputs, VmRequirement,
};
pub use crate::capabilities::workspace::domain::module::{
    agent_system, check_runtimes, check_secrets, check_services, check_tools, listing, role_system,
    Module, ModuleManifest, Param, ParamType, Roster, ServiceDecl, ToolDecl,
};
pub use crate::capabilities::workspace::domain::packages::{Library, PackageManifest};
pub use crate::capabilities::workspace::domain::workspace::{
    safe_file_name, AreaUsage, Place, Sandbox, Sandboxes, WorkFiles, WorkRoots, WorkUsage,
};
pub use crate::capabilities::workspace::domain::workstore::{
    Change, ChangeKind, CommitReport, CommitRequest, CommitSummary, PullReport, StatusEntry,
    StatusReport,
};

/// 工作区的**队列面**：呈现层要的清单事实（模块公地 + 拒收原因）。
/// 只报事实、不做选择——"挑哪些模块组合成一个 agent"是用户与核心的事。
pub trait WorkspaceOps: Send + Sync {
    fn roster(&self) -> Result<Roster, String>;
}

/// 工作区的**用例面**（`service.rs` 实现）：别的能力要清单事实、要工作区目录，走这里；
/// 出站端口（`ModuleSource` / `PackageSource` / `Workdirs` / `WorkStore`）**只由它持有**（R12）。
pub trait Workspace: Send + Sync {
    /// 模块公地：合法模块 + 拒收原因（清单即事实，每次重扫）。
    fn roster(&self) -> Roster;
    /// 运行包库：可用的包清单（每次重扫）。
    fn library(&self) -> Library;
    /// 包库所在目录（界面上要如实告诉用户"把包放哪儿"）。
    fn runtimes_dir(&self) -> std::path::PathBuf;
    /// 准备工作区：建 session/<工作名>/work 与每个 agent 的沙箱目录。
    fn prepare(&self, session: &str, agents: &[String]) -> Result<(), String>;
    /// work/ 下是否已有同名文件（上传同名冲突判定）。
    fn work_has(&self, session: &str, name: &str) -> bool;
    /// 列出本工作可引用的文件（work/ 与各 agent 沙箱；相对路径、/ 分隔、排序稳定）。
    fn files(&self, session: &str, agents: &[String]) -> Result<WorkFiles, String>;
    /// 统计工作区用量（共享区 + 各 agent 沙箱的文件数与总字节）：删除前如实交代用。
    fn usage(&self, session: &str, agents: &[String]) -> Result<WorkUsage, String>;
    /// 沙箱寻址根（work 与各 agent 私有区）：布局机制在适配层，拼接与越界校验在本能力。
    fn roots(&self, session: &str, agents: &[String]) -> Result<WorkRoots, String>;

    /// **拉取**：把主副本（head）的路径写进该 agent 的沙箱同相对路径，并更新它的拉取基线。
    /// 本地已改且上游也改的路径**不覆盖**，只报冲突。
    fn work_pull(&self, work: &str, agent: &str, paths: &[String]) -> Result<PullReport, String>;
    /// **提交**：文件级三方比较（基线 / 沙箱 / head）。任一条冲突 → 整个提交拒绝、逐条点名。
    fn work_commit(
        &self,
        work: &str,
        agent: &str,
        session: &str,
        req: &CommitRequest,
    ) -> Result<CommitReport, String>;
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
    /// 把主副本物化回某个提交点（只改共享区，不动各会话）。
    /// **消费方是回档对齐与提交历史（缺口 `work.rewind-and-file-state`）**：机制先就位、进测试，
    /// 生产调用点随回档重做接入——二进制 crate 里暂时没有生产调用点的接口方法会被 dead_code 误报。
    #[allow(dead_code)]
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
