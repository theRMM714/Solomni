//! 目的：`view_drive_lifecycle` 的 Windows 后端——真分配盘符、清扫与释放。
//! 管：逃生口开关、分配失败如实提示、调用视图盘模块。
//! 不管：盘符分配与登记（在 `view_drive/windows.rs`）。
//! 联动：由 `src/kernel/detail/view_drive_lifecycle.rs` 按平台选中。

use super::super::view_drive;
use std::path::Path;
use std::sync::Arc;

use crate::kernel::ports::PathCodec;

/// 目的：视图盘开关（`SOLOMNI_VIEW_DRIVE=0` 关闭；未设、空值或其它值都开）。
fn enabled(raw: Option<&str>) -> bool {
    !raw.is_some_and(|v| v.trim() == "0")
}

/// 目的：分配本实例的视图盘；返回是否真的分配了。
pub(super) fn begin(root: &Path, home: &Path) -> bool {
    if !enabled(std::env::var("SOLOMNI_VIEW_DRIVE").ok().as_deref()) {
        return false;
    }
    match view_drive::ensure_view_drive(root, home) {
        Ok(_) => true,
        Err(e) => {
            eprintln!("[围栏] 视图盘不可用：{}；回退真实路径", e);
            false
        }
    }
}

/// 目的：只清扫视图盘的崩溃残留（不分配盘符）。
pub(super) fn sweep(home: &Path) {
    view_drive::sweep_view_drive(home);
}

/// 目的：退出时注销本进程的视图盘登记。
pub(super) fn release(home: &Path) {
    view_drive::release_view_drive(home);
}

/// 目的：沙箱用的路径编解码器。
pub(super) fn codec() -> Option<Arc<dyn PathCodec>> {
    view_drive::view_codec()
}

#[cfg(test)]
mod tests {
    use super::enabled;

    #[test]
    fn switch_only_off_for_zero() {
        assert!(!enabled(Some("0")));
        assert!(!enabled(Some(" 0 ")));
        assert!(enabled(None));
        assert!(enabled(Some("1")));
        assert!(enabled(Some("")));
    }
}
