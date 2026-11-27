//! 目的：工作区的**出站端口**——目录布局（机制在适配层）。
//! 管：`Workdirs`（一次工作的 work 目录与各 agent 沙箱）。
//! 不管：路径拼接与越界判定的纯规则（在 `domain/workspace.rs`）；版本化落盘（在 `versions` 单元）。
//! 联动：本端口只由 `service.rs` 持有（R12）；实现见 `detail/fs_workspace.rs`。

use crate::units::business::workspace::domain::workspace::{WorkFiles, WorkRoots, WorkUsage};
use std::path::Path;

/// 目的：工作区**目录布局**端口——一次工作的 work 目录与各 agent 沙箱（机制在适配层）。
/// 约束：本单元只说"哪次工作、哪些 agent"，不碰路径拼接细节；**只有 `service.rs` 持有它**（R12）。
pub trait Workdirs {
    /// 目的：把工作区绑定落成布局投影（`Some` = 外部工作区根；`None` = 产品内布局）。
    /// 约束：只写会话目录里的布局文件，不创建 / 不校验外部根，也不碰用户文件。
    fn bind(&self, session: &str, root: Option<&Path>) -> Result<(), String>;
    /// 目的：该会话是否绑定了外部工作区（只读布局投影）。
    fn is_external(&self, session: &str) -> bool;
    /// 目的：拿住该工作区根的**跨会话写闸**；`None` = 产品内布局（已有按工作 / 版本库的锁）。
    /// 约束：多个会话绑定同一个外部目录时共用同一把锁；guard 要活到这次操作结束。
    fn shared_lock_path(&self, session: &str) -> Result<Option<std::path::PathBuf>, String>;
    /// 按会话铺布局：建 session/<工作名>/work 与每个 agent 的沙箱目录。
    fn ensure_layout(&self, session: &str, agents: &[String]) -> Result<(), String>;
    /// 撤销一次失败的铺布局：只删本产品建出的空沙箱 / 空 work/、布局投影与版本库 `.work`。
    /// 约束：调用方只在会话**从未落盘**时调用；用户文件绝不因为一次失败的创建被删。
    fn discard_layout(&self, session: &str, agents: &[String]) -> Result<(), String>;
    /// 界面投喂：把文件写进本工作的 work/（文件名由调用方净化）。
    fn write_work(&self, session: &str, name: &str, bytes: &[u8]) -> Result<(), String>;
    /// work/ 下是否已有同名文件（上传同名冲突判定）。
    fn work_has(&self, session: &str, name: &str) -> bool;
    /// 沙箱寻址根（work 与各 agent 私有区）：布局机制在适配层，拼接与越界校验在本单元。
    fn roots(&self, session: &str, agents: &[String]) -> Result<WorkRoots, String>;
    /// 列出本工作可引用的文件（work/ 与各 agent 沙箱；相对路径、/ 分隔、排序稳定）。
    fn list(&self, session: &str, agents: &[String]) -> Result<WorkFiles, String>;
    /// 统计工作区用量（共享区 + 各 agent 沙箱的文件数与总字节）：删除前如实交代用。
    fn usage(&self, session: &str, agents: &[String]) -> Result<WorkUsage, String>;
}
