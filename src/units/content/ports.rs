//! 目的：内容盘的出站端口（只有本单元 `service.rs` 持有，R12）。
//! 管：目录的确保存在这一件事。
//! 不管：路径怎么派生（在 `domain/paths.rs`）；上层语义。
//! 联动：适配器在 `detail/fs_content.rs`，只由组合根构造。

use std::path::Path;

/// 目的：目录落点的文件系统动作。
pub trait ContentFs: Send + Sync {
    /// 目的：确保目录存在（幂等）。
    /// 参数：`dir` 目录路径。
    /// 返回：`true` = 现在可用；`false` = 建不出来。
    fn ensure_dir(&self, dir: &Path) -> bool;
}
