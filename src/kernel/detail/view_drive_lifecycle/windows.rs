//! 目的：`view_drive_lifecycle` 的 Windows 后端——真分配盘符、清扫、释放与退出清理。
//! 管：逃生口开关、分配失败如实提示、调用视图盘模块、控制台终止时撤掉本实例的盘符。
//! 不管：盘符分配与登记（在 `view_drive/windows.rs`）。
//! 联动：由 `src/kernel/detail/view_drive_lifecycle.rs` 按平台选中。

use super::super::view_drive;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use crate::kernel::ports::PathCodec;
use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;

/// 目的：视图盘开关（`SOLOMNI_VIEW_DRIVE=0` 关闭；未设、空值或其它值都开）。
fn enabled(raw: Option<&str>) -> bool {
    !raw.is_some_and(|v| v.trim() == "0")
}

/// 目的：控制台终止时留给清理的锁等待上限：控制台事件线程上不能久等。
const EXIT_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// 目的：本实例的私有区（退出清理要知道撤哪份登记）；只在钩子装上之后才有值。
static HOME: OnceLock<PathBuf> = OnceLock::new();

/// 目的：控制台事件处理——先把本实例的视图盘映射与登记撤掉，再把事件交回默认处理（继续退出）。
/// 约束：跑在系统另起的线程上，只做撤登记这一件事。返回 FALSE 不吞事件，Ctrl+C / 关窗 / 注销 / 关机
///   都由默认处理终止进程；原始模式下 Ctrl+C 是按键（`src/presentation/cli/term.rs`），走 CLI 正常退出、
///   已在组合根释放，不经过这里。
/// 安全前提：处理函数只读进程内静态与 `.home` 记录，不碰其它共享状态；注册只发生一次。
unsafe extern "system" fn on_console_event(_event: u32) -> i32 {
    if let Some(home) = HOME.get() {
        cleanup(home);
    }
    0
}

/// 目的：退出清理本体——撤本进程的视图盘登记与映射。
fn cleanup(home: &Path) {
    view_drive::release_view_drive_within(home, EXIT_WAIT);
}

/// 目的：装上退出清理钩子：此后的控制台终止都会先撤掉视图盘，不留可见盘符。
/// 约束：每进程只装一次；没有控制台时安装失败只记一行，残留仍由下次启动的清扫兜底。
fn install_exit_hook(home: &Path) {
    if HOME.set(home.to_path_buf()).is_err() {
        return;
    }
    let ok = unsafe { SetConsoleCtrlHandler(Some(on_console_event), 1) };
    if ok == 0 {
        eprintln!("[视图盘] 装不上退出清理钩子；盘符残留留给下次启动清扫");
    }
}

/// 目的：分配本实例的视图盘；返回是否真的分配了。
pub(super) fn begin(root: &Path, home: &Path) -> bool {
    if !enabled(std::env::var("SOLOMNI_VIEW_DRIVE").ok().as_deref()) {
        return false;
    }
    match view_drive::ensure_view_drive(root, home) {
        Ok(_) => {
            install_exit_hook(home);
            true
        }
        Err(e) => {
            eprintln!("[围栏] 视图盘不可用：{}；回退真实路径", e);
            false
        }
    }
}

/// 目的：只清扫视图盘的崩溃残留（不分配盘符）；返回撤掉的盘符个数。
pub(super) fn sweep(home: &Path) -> usize {
    view_drive::sweep_view_drive(home)
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
    use super::{cleanup, enabled};

    #[test]
    fn switch_only_off_for_zero() {
        assert!(!enabled(Some("0")));
        assert!(!enabled(Some(" 0 ")));
        assert!(enabled(None));
        assert!(enabled(Some("1")));
        assert!(enabled(Some("")));
    }

    /// 目的：控制台终止走的就是这条清理——记录里本进程的条目要撤掉，映射不在时记录文件整个删掉。
    #[test]
    fn exit_cleanup_drops_the_record() {
        let home = std::env::temp_dir().join(format!("solomni-view-exit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).expect("建临时私有区");
        let root = home.join("product-root");
        std::fs::create_dir_all(&root).expect("建假产品根");
        let pid = std::process::id();
        let record = serde_json::json!([{
            "letter": "Q:",
            "root": root.to_string_lossy(),
            "users": [{
                "pid": pid,
                "created": crate::kernel::detail::procs::created(pid).unwrap_or(0),
            }],
        }]);
        std::fs::write(home.join("view-drive.json"), record.to_string()).expect("写记录");
        cleanup(&home);
        assert!(
            !home.join("view-drive.json").exists(),
            "退出清理后不该留下盘符登记"
        );
        let _ = std::fs::remove_dir_all(&home);
    }
}
