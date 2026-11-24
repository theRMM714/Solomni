//! 目的：本地时间——把"此刻几点"按**本机时区**给出来，进程里只此一份取时实现。
//! 管：`local_now`（年月日时分秒毫秒）、`local_stamp`（到分钟的流水戳 `YYYYMMDD-HHMM`）。
//! 不管：时间怎么排版（调用方按字段自己排）；耗时与单调钟（要时长用 `std::time::Instant`）。
//! 联动：日志文件名与行首用 `local_now`（`src/kernel/detail/file_log.rs`）；会话缺省名用 `local_stamp`（核心建会话，经 `api/clock`）；
//!   取时按平台分派到 `src/kernel/detail/clock/{windows,unix}.rs`。

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as platform;
#[cfg(not(windows))]
mod unix;
#[cfg(not(windows))]
use unix as platform;

/// 目的：本地民用时间的一个瞬间（本机时区；不含偏移与时区名）。
/// 约束：字段是给人看的日历值，不参与时间比较——比先后用 `std::time`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Local {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub millis: u32,
}

/// 目的：此刻的本地时间。
pub fn local_now() -> Local {
    platform::now()
}

/// 目的：此刻的到分钟流水戳（如 `20261124-2053`）——日志与会话缺省名共用的形状只在这里定。
/// 约束：到分钟是刻意的（可读优先）；同一分钟内重名的唯一化由调用方按自己的占用表加尾号。
pub fn local_stamp() -> String {
    let t = local_now();
    format!(
        "{:04}{:02}{:02}-{:02}{:02}",
        t.year, t.month, t.day, t.hour, t.minute
    )
}
