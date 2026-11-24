//! 目的：`clock` 的 Unix 后端——系统本地时间（`localtime_r`），时区坏掉时退到 UTC。
//! 管：`now`（秒经 `localtime_r` 转本地民用时间，毫秒取同一瞬间的余数）。
//! 不管：时区数据库从哪来（系统提供）；夏令时的规则。
//! 联动：机制门面在 `src/kernel/detail/clock.rs`。

use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn now() -> super::Local {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() as libc::time_t;
    let millis = now.subsec_millis();
    let mut tm = std::mem::MaybeUninit::<libc::tm>::zeroed();
    // 安全：localtime_r / gmtime_r 只写这块本栈上的 tm；secs 是同一栈帧里的局部变量，
    // 调用期间一直有效。返回空指针时 tm 可能没被写，故失败后才补一次 gmtime_r，再统一取已初始化的值。
    let local = unsafe { libc::localtime_r(&secs, tm.as_mut_ptr()) };
    let filled = unsafe {
        if local.is_null() && libc::gmtime_r(&secs, tm.as_mut_ptr()).is_null() {
            return super::Local {
                year: 1970,
                month: 1,
                day: 1,
                hour: 0,
                minute: 0,
                second: 0,
                millis,
            };
        }
        tm.assume_init()
    };
    super::Local {
        year: filled.tm_year + 1900,
        month: (filled.tm_mon + 1) as u32,
        day: filled.tm_mday as u32,
        hour: filled.tm_hour as u32,
        minute: filled.tm_min as u32,
        second: filled.tm_sec as u32,
        millis,
    }
}
