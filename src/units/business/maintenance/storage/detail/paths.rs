//! 目的：回收各处的落点与构建卷占用判定——survey 与 reclaim 共用同一套路径。
//! 管：把盘点用的名字映射到 `vmstore` 的布局上，并问内核"构建锁此刻有没有人持"。
//! 不管：怎么扫（survey）、怎么删（reclaim）、回收锁（lock）；落点本身归 `vmstore`。
//! 联动：detail::survey 与 detail::reclaim 都经它取路径，扫与删不会各写一份；布局见 `units::resource::vmstore`。

use crate::units::resource::vmstore::api;
use std::path::{Path, PathBuf};

/// 目的：构建缓存卷的目录。
pub fn build_dir(root: &Path) -> PathBuf {
    api::paths(root).build_cache()
}

/// 目的：一个虚拟根的构建缓存卷。
pub fn build_volume(root: &Path, guest: &str) -> PathBuf {
    api::paths(root).build_volume(guest)
}

/// 目的：构建缓存卷旁的元数据（builds 写，盘点读）。
pub fn build_meta(root: &Path, guest: &str) -> PathBuf {
    api::paths(root).build_meta(guest)
}

/// 目的：一个虚拟根的构建锁（builds 持，盘点与回收看）。
pub fn build_lock(root: &Path, guest: &str) -> PathBuf {
    api::paths(root).build_lock(guest)
}

/// 目的：按模块清理的排队标记（下次构建前消费）。
pub fn build_purge(root: &Path, guest: &str) -> PathBuf {
    api::paths(root).build_purge(guest)
}

/// 目的：依赖补齐的下载缓存目录。
pub fn download_dir(root: &Path) -> PathBuf {
    api::paths(root).downloads()
}

/// 目的：会话可写层目录。
pub fn overlay_dir(root: &Path) -> PathBuf {
    api::paths(root).overlays()
}

/// 目的：运行日志目录。
pub fn log_dir(root: &Path) -> PathBuf {
    crate::kernel::api::layout::logs(root)
}

/// 目的：这个虚拟根是不是有构建在跑（有人此刻持着构建锁）。
pub fn build_locked(root: &Path, guest: &str) -> bool {
    lock_held(&build_lock(root, guest))
}

/// 目的：构建锁此刻有没有人持——问内核文件锁；问不清时按"有人持"处理（宁可不删）。
pub fn lock_held(path: &Path) -> bool {
    crate::kernel::api::lock::held(path, "构建缓存卷").unwrap_or(true)
}
