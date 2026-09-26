//! 工作区与运行包的**出站端口**：目录布局与清单扫描（机制在适配层）。

use crate::capabilities::workspace::domain::module::Roster;
use crate::capabilities::workspace::domain::packages::Library;
use crate::capabilities::workspace::domain::workspace::{WorkFiles, WorkRoots};

/// 模块清单来源端口。
pub trait ModuleSource {
    fn scan(&self) -> Roster;
}

/// 运行包库来源端口：扫描依赖文件夹（runtimes/）里的包清单。
/// 「清单即事实」：每次调用重扫，放入即出现；清单校验、去重与冲突预检在本能力（packages::Library::build），
/// 目录遍历与 yaml 解析在适配层。
pub trait PackageSource {
    fn scan(&self) -> Library;
    /// 包库所在目录（配置界面要把"把包放哪儿"如实告诉用户）。
    fn dir(&self) -> std::path::PathBuf;
}

/// 工作区端口：一次工作的 work 目录与各 agent 沙箱（目录布局机制在适配层）。
/// 本能力只说"哪次工作、哪些 agent"，不碰路径拼接细节。
pub trait Workspace {
    /// 准备工作区：建 session/<工作名>/work 与每个 agent 的沙箱目录。
    fn prepare(&self, session: &str, agents: &[String]) -> Result<(), String>;
    /// 界面投喂：把文件写进本工作的 work/（文件名由调用方净化）。
    fn write_work(&self, session: &str, name: &str, bytes: &[u8]) -> Result<(), String>;
    /// work/ 下是否已有同名文件（上传同名冲突判定）。
    fn work_has(&self, session: &str, name: &str) -> bool;
    /// 沙箱寻址根（work 与各 agent 私有区）：布局机制在适配层，拼接与越界校验在本能力。
    fn roots(&self, session: &str, agents: &[String]) -> Result<WorkRoots, String>;
    /// 列出本工作可引用的文件（work/ 与各 agent 沙箱；相对路径、/ 分隔、排序稳定）。
    fn list(&self, session: &str, agents: &[String]) -> Result<WorkFiles, String>;
}
