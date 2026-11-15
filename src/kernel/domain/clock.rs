//! 目的：时间的单一来源——把「现在几点」收敛成一处纯机制实现。
//! 管：Unix 纪元秒。
//! 不管：时区、格式化与展示（由呈现层自行换算）；进程计时的毫秒口径（在 `detail/procs.rs`）。
//! 联动：`kernel::api` 重导出给各能力；`conductor` 与 `turn` 的时间戳都取这一份。

/// 目的：取当前时间戳（Unix 纪元起的秒数）。
/// 返回：秒数；系统时钟不可用时回退 0。
/// 约束：全仓唯一一份实现；需要别的精度时在调用点换算，不再复制取时逻辑。
pub fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
