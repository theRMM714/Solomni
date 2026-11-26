//! 目的：内容盘的文件系统适配器（实现 `ContentFs` 端口）。
//! 管：目录的确保存在。
//! 不管：路径是不是该建、建完怎么报（在 `service.rs`）。
//! 联动：由组合根构造，只经端口给 `service.rs`。

use crate::units::resource::content::ports::ContentFs;
use std::path::Path;

/// 目的：真实文件系统的目录动作（无状态）。
#[derive(Debug, Clone, Copy, Default)]
pub struct FsContentFs;

impl ContentFs for FsContentFs {
    fn ensure_dir(&self, dir: &Path) -> bool {
        dir.is_dir() || std::fs::create_dir_all(dir).is_ok()
    }
}
