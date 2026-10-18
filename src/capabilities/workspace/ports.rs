//! 工作区的**出站端口**：目录布局与共享区版本库（机制在适配层）。

use crate::capabilities::workspace::domain::asset::FileVersion;
use crate::capabilities::workspace::domain::workspace::{WorkFiles, WorkRoots, WorkUsage};
use crate::capabilities::workspace::domain::workstore::{Commit, Index, ObjectInfo, Pending};
use std::collections::BTreeMap;
use std::fs::File;
use std::path::Path;

/// 工作区**目录布局**端口：一次工作的 work 目录与各 agent 沙箱（机制在适配层）。
/// 本能力只说"哪次工作、哪些 agent"，不碰路径拼接细节。**只有 `service.rs` 持有它**（R12）。
pub trait Workdirs {
    /// 准备工作区：建 session/<工作名>/work 与每个 agent 的沙箱目录。
    fn prepare(&self, session: &str, agents: &[String]) -> Result<(), String>;
    /// 撤销一次准备：只删**空**目录（有什么都不删），建会话失败时用。
    fn discard(&self, session: &str, agents: &[String]) -> Result<(), String>;
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

/// 共享区**版本库**的落盘端口：流式资产原语（主副本与沙箱都经它）+ 内容寻址对象 / 提交记录 / 每个 agent 的拉取基线。
///
/// 策略不在这一层：三方比较、路径校验、冲突判定都在 `domain::workstore`；本端口只保证
/// "按给定根与相对路径读写"，并拒绝一切**符号链接逃逸**（相对路径本身干净不等于落点安全）。
/// **只有 `service.rs` 持有它**（R12）。
pub trait WorkStore: Send + Sync {
    /// 删 root 下的一条干净相对路径（不存在 = 幂等成功）。
    fn remove_under(&self, root: &Path, rel: &str) -> Result<(), String>;
    /// 递归列 root 下的文件（相对路径、/ 分隔、稳定排序）。
    fn list(&self, root: &Path) -> Result<Vec<String>, String>;
    /// 目的：读 root 下文件的大小；不存在 = None。
    /// 约束：只读元数据，不读内容；上限预检用它。
    fn stat_under(&self, root: &Path, rel: &str) -> Result<Option<u64>, String>;
    /// 目的：流式扫描 root 下文件：指纹、大小、可选文本预览；不存在 = None。
    /// 约束：`preview = None` 只算指纹与大小；`Some(n)` 才做 NUL/UTF-8 判定并保留最多 n 字节预览。
    fn scan_under(
        &self,
        root: &Path,
        rel: &str,
        preview: Option<usize>,
    ) -> Result<Option<FileVersion>, String>;
    /// 目的：读一个内容对象的大小；对象不存在 = Err。
    fn stat_object(&self, store: &Path, hash: &str) -> Result<u64, String>;
    /// 目的：流式扫描一个内容对象；`preview = None` 只算指纹与大小。
    fn scan_object(
        &self,
        store: &Path,
        hash: &str,
        preview: Option<usize>,
    ) -> Result<FileVersion, String>;
    /// 目的：把 root 下文件流式写成内容对象（对象名 = `expected_hash`），并校验实际指纹与上限。
    /// 约束：超上限或指纹不符时删临时文件并报错；对象已存在则直接返回其大小。
    fn ingest_under(
        &self,
        store: &Path,
        root: &Path,
        rel: &str,
        expected_hash: &str,
        max_bytes: u64,
    ) -> Result<u64, String>;
    /// 目的：把一个内容对象流式复制到 root 下同相对路径（需要时建父目录）。
    fn copy_object_to_under(
        &self,
        store: &Path,
        hash: &str,
        root: &Path,
        rel: &str,
    ) -> Result<(), String>;
    /// 当前 head 提交号；空仓库 = None。
    fn head(&self, store: &Path) -> Result<Option<u64>, String>;
    fn set_head(&self, store: &Path, id: u64) -> Result<(), String>;
    /// 清空 head（恢复/删除到"空共享区"时用）。
    fn clear_head(&self, store: &Path) -> Result<(), String>;
    /// 读一个提交记录；不存在 = None。
    fn read_commit(&self, store: &Path, id: u64) -> Result<Option<Commit>, String>;
    /// 写一个提交记录（调用方已分配 id；只增不改）。
    fn write_commit(&self, store: &Path, commit: &Commit) -> Result<(), String>;
    /// 列出全部提交号（升序）。
    fn list_commits(&self, store: &Path) -> Result<Vec<u64>, String>;
    /// 删除一个提交记录（删除 / 恢复时丢弃不再可达的提交）。
    fn remove_commit(&self, store: &Path, id: u64) -> Result<(), String>;
    /// 读某个 agent 的拉取基线；没有 = 空（不是错误）。
    fn read_index(&self, store: &Path, agent: &str) -> Result<Index, String>;
    fn write_index(&self, store: &Path, agent: &str, index: &Index) -> Result<(), String>;
    /// 内容寻址对象：同指纹只写一次。
    fn write_object(&self, store: &Path, hash: &str, bytes: &[u8]) -> Result<(), String>;
    fn read_object(&self, store: &Path, hash: &str) -> Result<Vec<u8>, String>;
    /// 目的：拿住该工作的跨进程写闸；返回 None = 内存替身没有文件锁。
    /// 约束：guard 要活到这次操作结束，drop 即释放。
    fn lock(&self, store: &Path) -> Result<Option<File>, String>;
    /// 目的：读待恢复事务；没有 = None。
    fn read_pending(&self, store: &Path) -> Result<Option<Pending>, String>;
    /// 目的：写待恢复事务（同目录临时文件 + rename）。
    fn write_pending(&self, store: &Path, pending: &Pending) -> Result<(), String>;
    /// 目的：清掉待恢复事务；不存在 = 幂等成功。
    fn clear_pending(&self, store: &Path) -> Result<(), String>;
    /// 目的：读下一个提交号计数器；没有 = None。
    fn read_next_id(&self, store: &Path) -> Result<Option<u64>, String>;
    /// 目的：写下一个提交号计数器（同目录临时文件 + rename）。
    fn write_next_id(&self, store: &Path, id: u64) -> Result<(), String>;
    /// 目的：读出全部 agent 基线；没有 index 目录 = 空。
    fn list_indexes(&self, store: &Path) -> Result<BTreeMap<String, Index>, String>;
    /// 目的：列出对象目录里的文件（含非指纹杂项；回收方自己过滤）。
    fn list_objects(&self, store: &Path) -> Result<Vec<ObjectInfo>, String>;
    /// 目的：删一个对象目录里的文件；不存在 = 幂等成功。
    fn remove_object(&self, store: &Path, hash: &str) -> Result<(), String>;
}
