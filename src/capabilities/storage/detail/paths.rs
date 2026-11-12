//! 目的：回收各处的落点与构建卷占用判定——survey 与 reclaim 共用同一套路径。
//! 管：构建缓存、下载缓存、可写层、日志的路径，以及"构建锁还在不在"。
//! 不管：怎么扫（survey）、怎么删（reclaim）、回收锁（lock）。
//! 联动：detail::survey 与 detail::reclaim 都经它取路径，扫与删不会各写一份。

use std::path::{Path, PathBuf};

/// 目的：构建缓存卷的目录。
pub fn build_dir(root: &Path) -> PathBuf {
    root.join(".vm").join("build-cache")
}

/// 目的：一个虚拟根的构建缓存卷。
pub fn build_volume(root: &Path, guest: &str) -> PathBuf {
    build_dir(root).join(format!("{}.qcow2", guest))
}

/// 目的：构建缓存卷旁的元数据（builds 写，盘点读）。
pub fn build_meta(root: &Path, guest: &str) -> PathBuf {
    build_dir(root).join(format!("{}.json", guest))
}

/// 目的：一个虚拟根的构建锁（builds 持，盘点与回收看）。
pub fn build_lock(root: &Path, guest: &str) -> PathBuf {
    build_dir(root).join(format!("{}.lock", guest))
}

/// 目的：按模块清理的排队标记（下次构建前消费）。
pub fn build_purge(root: &Path, guest: &str) -> PathBuf {
    build_dir(root).join(format!("{}.purge", guest))
}

/// 目的：依赖补齐的下载缓存目录。
pub fn download_dir(root: &Path) -> PathBuf {
    root.join(".vm").join("cache")
}

/// 目的：会话可写层目录。
pub fn overlay_dir(root: &Path) -> PathBuf {
    root.join(".vm").join("overlays")
}

/// 目的：运行日志目录。
pub fn log_dir(root: &Path) -> PathBuf {
    root.join("logs")
}

/// 目的：这个虚拟根是不是有构建在跑（构建锁仍有效）。
pub fn build_locked(root: &Path, guest: &str) -> bool {
    lock_held(&build_lock(root, guest))
}

/// 目的：构建锁是否仍有效——pid 还活着，或是三小时内建的（读不到按有效处理，不抢）。
pub fn lock_held(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let text = std::fs::read_to_string(path).unwrap_or_default();
    if let Some(pid) = text
        .split_whitespace()
        .skip_while(|w| *w != "pid")
        .nth(1)
        .and_then(|s| s.parse::<u32>().ok())
        && crate::kernel::detail::procs::alive(pid)
    {
        return true;
    }
    meta.modified()
        .ok()
        .and_then(|t| std::time::SystemTime::now().duration_since(t).ok())
        .map(|d| d < std::time::Duration::from_secs(3 * 3600))
        .unwrap_or(false)
}
