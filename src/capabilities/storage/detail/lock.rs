//! 目的：回收的跨进程互斥——.vm/storage.lock，进程死掉或超过时限视为陈旧。
//! 管：try_acquire / 陈旧判定 / 释放；不持业务状态。
//! 不管：构建卷自己的锁（在 builds）。
//! 联动：storage::service 的 gc 与显式清理共用这一把闸。

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// 目的：回收锁；Drop 时删除锁文件。
pub struct GcLock {
    path: PathBuf,
}

/// 目的：锁文件路径。
pub fn path(root: &Path) -> PathBuf {
    root.join(".vm").join("storage.lock")
}

/// 目的：尝试拿锁；已有另一个回收在跑给 None。
/// 错误：建目录或写锁失败。
pub fn try_acquire(root: &Path) -> Result<Option<GcLock>, String> {
    let p = path(root);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("建存储目录失败：{}", e))?;
    }
    let stale = std::fs::metadata(&p)
        .map(|m| {
            let old = m
                .modified()
                .ok()
                .and_then(|t| SystemTime::now().duration_since(t).ok())
                .map(|d| d > Duration::from_secs(30 * 60))
                .unwrap_or(false);
            old || !pid_alive(&p)
        })
        .unwrap_or(false);
    if stale {
        let _ = std::fs::remove_file(&p);
    }
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&p)
    {
        Ok(mut f) => {
            use std::io::Write;
            let _ = write!(f, "pid {} at {:?}", std::process::id(), SystemTime::now());
            Ok(Some(GcLock { path: p }))
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
        Err(e) => Err(format!("建回收锁失败（{}）：{}", p.display(), e)),
    }
}

/// 目的：锁内容里的 pid 还活着吗；读不到按“可能活着”处理（不抢）。
fn pid_alive(path: &Path) -> bool {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let Some(pid) = text
        .split_whitespace()
        .skip_while(|w| *w != "pid")
        .nth(1)
        .and_then(|s| s.parse::<u32>().ok())
    else {
        return true;
    };
    crate::kernel::detail::procs::alive(pid)
}

impl Drop for GcLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
