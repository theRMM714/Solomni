//! 目的：回收的跨进程互斥——data/.vm/storage.lock，进程退出即释放。
//! 管：try_acquire（非阻塞拿锁）与锁落点；不持业务状态。
//! 不管：构建卷自己的锁（在 builds）；陈旧判定（内核文件锁由操作系统在进程退出时释放）。
//! 联动：storage::service 的 gc 与显式清理共用这一把闸。

use std::path::{Path, PathBuf};

/// 目的：锁文件路径。
fn path(root: &Path) -> PathBuf {
    crate::units::resource::vmstore::api::paths(root).storage_lock()
}

/// 目的：尝试拿回收锁；已有另一个回收在跑给 None。
/// 约束：非阻塞——守卫只包一次回收，等锁没有意义，拿不到就如实拒绝。
/// 错误：建目录或拿锁失败。
pub fn try_acquire(root: &Path) -> Result<Option<std::fs::File>, String> {
    let p = path(root);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("建存储目录失败：{}", e))?;
    }
    crate::kernel::api::lock::try_lock(&p, "回收")
}
