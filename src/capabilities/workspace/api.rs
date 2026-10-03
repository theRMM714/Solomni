//! 入站能力面：**其它能力与呈现层只准用这里**（不许碰 `domain` / `ports`）。

pub use crate::capabilities::workspace::domain::exec::{
    absent, declared, diagnose_text, plan, plan_summary, tier_readiness, tier_refusal, unavailable,
    vm_diagnoses, vm_requirements, Diagnosis, ExecSpec, VmInputs, VmRequirement,
};
pub use crate::capabilities::workspace::domain::module::{
    agent_system, check_runtimes, check_tools, listing, role_system, Module, ModuleManifest, Param,
    ParamType, Roster, ToolDecl,
};
pub use crate::capabilities::workspace::domain::packages::{Library, PackageManifest};
pub use crate::capabilities::workspace::domain::workspace::{
    safe_file_name, AreaUsage, Place, Sandbox, Sandboxes, WorkFiles, WorkRoots, WorkUsage,
};

/// 工作区的**队列面**：呈现层要的清单事实（模块公地 + 拒收原因）。
/// 只报事实、不做选择——"挑哪些模块组合成一个 agent"是用户与核心的事。
pub trait WorkspaceOps: Send + Sync {
    fn roster(&self) -> Result<Roster, String>;
}

/// 工作区的**用例面**（`service.rs` 实现）：别的能力要清单事实、要工作区目录，走这里；
/// 三个出站端口（`ModuleSource` / `PackageSource` / `Workdirs`）**只由它持有**（R12）。
pub trait Workspace: Send + Sync {
    /// 模块公地：合法模块 + 拒收原因（清单即事实，每次重扫）。
    fn roster(&self) -> Roster;
    /// 运行包库：可用的包清单（每次重扫）。
    fn library(&self) -> Library;
    /// 包库所在目录（界面上要如实告诉用户"把包放哪儿"）。
    fn runtimes_dir(&self) -> std::path::PathBuf;
    /// 准备工作区：建 session/<工作名>/work 与每个 agent 的沙箱目录。
    fn prepare(&self, session: &str, agents: &[String]) -> Result<(), String>;
    /// 界面投喂：把文件写进本工作的 work/（文件名由调用方净化）。
    fn write_work(&self, session: &str, name: &str, bytes: &[u8]) -> Result<(), String>;
    /// work/ 下是否已有同名文件（上传同名冲突判定）。
    fn work_has(&self, session: &str, name: &str) -> bool;
    /// 列出本工作可引用的文件（work/ 与各 agent 沙箱；相对路径、/ 分隔、排序稳定）。
    fn files(&self, session: &str, agents: &[String]) -> Result<WorkFiles, String>;
    /// 统计工作区用量（共享区 + 各 agent 沙箱的文件数与总字节）：删除前如实交代用。
    fn usage(&self, session: &str, agents: &[String]) -> Result<WorkUsage, String>;
    /// 沙箱寻址根（work 与各 agent 私有区）：布局机制在适配层，拼接与越界校验在本能力。
    fn roots(&self, session: &str, agents: &[String]) -> Result<WorkRoots, String>;
}
