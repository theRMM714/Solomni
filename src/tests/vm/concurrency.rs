//! 目的：租约的并发准入——到上限拒绝、跨租约真并行。
//! 管：准入计数、表锁范围。
//! 不管：执行本身。
//! 联动：`src/units/business/vm/api.rs`。

use super::super::prelude::*;

/// 目的：并发准入——到上限拒绝、同一租约重启放行、0 = 不限制。
#[test]
pub(crate) fn guest_admission_caps_concurrency() {
    let two = vec!["a".to_string(), "b".to_string()];
    assert!(
        exec::admit_guest(&two, "a", 2).is_ok(),
        "同一租约重启不占新名额"
    );
    assert!(exec::admit_guest(&two, "c", 2).is_err(), "到上限要拒");
    assert!(exec::admit_guest(&two, "c", 3).is_ok(), "没到上限放行");
    assert!(exec::admit_guest(&two, "c", 0).is_ok(), "0 = 不限制");
    assert!(exec::admit_guest(&[], "c", 2).is_ok());
}
