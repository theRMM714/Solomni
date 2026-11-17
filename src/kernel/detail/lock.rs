//! 目的：跨进程文件锁的获取协议——打开锁文件、有界等待、拿不到就如实报错。
//! 管：`lock_within`（25ms 轮询 `File::try_lock`，超时与打开失败都返回原因）。
//! 不管：锁文件放哪、等多久、锁保护的是什么、进程内互斥（各调用方按需自己持有）。
//! 联动：workspace 的共享区锁与版本库锁、内核的围栏台账锁与视图盘记录锁都经它。

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

#[cfg(test)]
mod tests;
