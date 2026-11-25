//! 目的：跨进程文件锁的协议——打开锁文件、有界等待或一问即答、拿不到就如实报错。
//! 管：`lock_within`（25ms 轮询 `File::try_lock`，超时与打开失败都返回原因）、`try_lock`（非阻塞拿锁）、`held`（只问不拿）。
//! 不管：锁文件放哪、等多久、锁保护的是什么、进程内互斥（各调用方按需自己持有）。
//! 联动：workspace 的共享区锁与版本库锁、内核的围栏台账锁与视图盘记录锁、构建卷锁与回收锁都经它。

use std::path::Path;
use std::time::{Duration, Instant};

/// 目的：在 `wait` 内拿到 `path` 上的独占文件锁；返回的句柄 drop 即释放。
/// 参数：`what` 只进报错文案（如「工作区」「台账」），不参与判断。
/// 约束：有界等待，不无限阻塞；拿不到就如实报错，不静默跳过保护。
pub fn lock_within(path: &Path, wait: Duration, what: &str) -> Result<std::fs::File, String> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| format!("打开{}锁失败（{}）：{}", what, path.display(), e))?;
    let deadline = Instant::now() + wait;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) => {
                if Instant::now() >= deadline {
                    return Err(format!("{}被另一个进程占用，等待超时（稍后重试）", what));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(std::fs::TryLockError::Error(e)) => return Err(format!("{}加锁失败：{}", what, e)),
        }
    }
}

/// 目的：非阻塞拿 `path` 上的独占文件锁；别人持着就给 `None`，从不等待。
/// 参数：`what` 只进报错文案（如「构建缓存卷」），不参与判断。
/// 约束：判断与取得是同一个原子操作，不会出现"先看再抢"的空档；锁文件不存在时会建。
pub fn try_lock(path: &Path, what: &str) -> Result<Option<std::fs::File>, String> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| format!("打开{}锁失败（{}）：{}", what, path.display(), e))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(format!("{}加锁失败：{}", what, e)),
    }
}

/// 目的：只问不拿——此刻有别的进程持着 `path` 就 true；锁文件不存在或没人持都给 false。
/// 参数：`what` 只进报错文案，不参与判断。
/// 约束：不建锁文件、不改锁文件；问到的只是这一瞬的状态，期间别人可以来拿。
pub fn held(path: &Path, what: &str) -> Result<bool, String> {
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("打开{}锁失败（{}）：{}", what, path.display(), e)),
    };
    match file.try_lock() {
        Ok(()) => Ok(false),
        Err(std::fs::TryLockError::WouldBlock) => Ok(true),
        Err(std::fs::TryLockError::Error(e)) => Err(format!("{}加锁失败：{}", what, e)),
    }
}

#[cfg(test)]
mod tests;
