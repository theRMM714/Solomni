//! 工作区与运行包的**出站端口**：目录布局与清单扫描（机制在适配层）。

use crate::capabilities::workspace::domain::module::Roster;
use crate::capabilities::workspace::domain::packages::Library;
use crate::capabilities::workspace::domain::workspace::{WorkFiles, WorkRoots, WorkUsage};
use crate::capabilities::workspace::domain::workstore::{Commit, Index};
use std::path::Path;

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

/// 工作区**目录布局**端口：一次工作的 work 目录与各 agent 沙箱（机制在适配层）。
/// 本能力只说"哪次工作、哪些 agent"，不碰路径拼接细节。**只有 `service.rs` 持有它**（R12）。
pub trait Workdirs {
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
    /// 统计工作区用量（共享区 + 各 agent 沙箱的文件数与总字节）：删除前如实交代用。
    fn usage(&self, session: &str, agents: &[String]) -> Result<WorkUsage, String>;
}

/// 共享区**版本库**的落盘端口：文件原语（主副本与沙箱都经它）+ 内容寻址对象 / 提交记录 / 每个 agent 的拉取基线。
///
/// 策略不在这一层：三方比较、路径校验、冲突判定都在 `domain::workstore`；本端口只保证
/// "按给定根与相对路径读写"，并拒绝一切**符号链接逃逸**（相对路径本身干净不等于落点安全）。
/// **只有 `service.rs` 持有它**（R12）。
pub trait WorkStore: Send + Sync {
    /// 读 root 下的一条干净相对路径；不存在 = None。
    fn read_under(&self, root: &Path, rel: &str) -> Result<Option<Vec<u8>>, String>;
    /// 写 root 下的一条干净相对路径（需要时建父目录）。
    fn write_under(&self, root: &Path, rel: &str, bytes: &[u8]) -> Result<(), String>;
    /// 删 root 下的一条干净相对路径（不存在 = 幂等成功）。
    fn remove_under(&self, root: &Path, rel: &str) -> Result<(), String>;
    /// 递归列 root 下的文件（相对路径、/ 分隔、稳定排序）。
    fn list(&self, root: &Path) -> Result<Vec<String>, String>;
    /// 当前 head 提交号；空仓库 = None。
    fn head(&self, store: &Path) -> Result<Option<u64>, String>;
    fn set_head(&self, store: &Path, id: u64) -> Result<(), String>;
    /// 读一个提交记录；不存在 = None。
    fn read_commit(&self, store: &Path, id: u64) -> Result<Option<Commit>, String>;
    /// 写一个提交记录（调用方已分配 id；只增不改）。
    fn write_commit(&self, store: &Path, commit: &Commit) -> Result<(), String>;
    /// 列出全部提交号（升序）。
    fn list_commits(&self, store: &Path) -> Result<Vec<u64>, String>;
    /// 读某个 agent 的拉取基线；没有 = 空（不是错误）。
    fn read_index(&self, store: &Path, agent: &str) -> Result<Index, String>;
    fn write_index(&self, store: &Path, agent: &str, index: &Index) -> Result<(), String>;
    /// 内容寻址对象：同指纹只写一次。
    fn write_object(&self, store: &Path, hash: &str, bytes: &[u8]) -> Result<(), String>;
    fn read_object(&self, store: &Path, hash: &str) -> Result<Vec<u8>, String>;
}
