//! 目的：文件锁协议的测试：真文件、真句柄、真超时（内核用真实实例）。
//! 管：只覆盖本模块的公开契约与报错文案；夹具在本文件内。
//! 不管：调用方怎么用这把锁（在各自的服务里）。
//! 联动：与 `src/kernel/detail/lock.rs` 同模块，`#[cfg(test)] mod tests;` 指到这里。

use super::*;

/// 目的：拿到锁后第二次获取在等待上限内如实超时；放掉第一把后又能拿到。
#[test]
fn a_held_lock_times_out_and_is_free_after_release() {
    let dir = crate::tests::scratch("lock-within");
    let path = dir.join("guard.lock");
    let first = lock_within(&path, Duration::from_millis(200), "台账").expect("首把锁");
    let err = lock_within(&path, Duration::from_millis(120), "台账").expect_err("第二把应当是超时");
    assert!(err.contains("台账"), "报错带主语：{}", err);
    assert!(err.contains("等待超时"), "报错说明等不到：{}", err);
    drop(first);
    let again = lock_within(&path, Duration::from_millis(200), "台账").expect("放掉后可再拿");
    drop(again);
    let _ = std::fs::remove_dir_all(&dir);
}
