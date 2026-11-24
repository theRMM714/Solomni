//! 目的：钉本地时间机制的形状契约——缺省命名的流水戳与日志排版都靠它。
//! 管：`local_stamp` 的形状（到分钟、年-月-日与时-分两段）、`local_now` 各字段落在合法范围。
//! 不管：与系统时钟是否一致（那是平台的活，用例只认形状）；时区换算的正确性（交给平台）。
//! 联动：`src/kernel/detail/clock.rs`；会话缺省名在 `runtime` 建会话处消费它。

use crate::kernel::detail::clock;

#[test]
fn local_stamp_is_a_minute_precision_stamp() {
    let stamp = clock::local_stamp();
    assert_eq!(stamp.chars().count(), 13, "YYYYMMDD-HHMM：{stamp}");
    assert_eq!(&stamp[8..9], "-", "日与时之间用 - 隔开：{stamp}");
    for (i, c) in stamp.chars().enumerate() {
        if i == 8 {
            continue;
        }
        assert!(c.is_ascii_digit(), "第 {} 位该是数字：{stamp}", i + 1);
    }
    let year: u32 = stamp[..4].parse().expect("年份该是四位数字");
    assert!(year >= 2024, "年份取四位：{stamp}");
}

#[test]
fn local_now_fields_are_in_range() {
    let t = clock::local_now();
    assert!(t.year >= 2024, "年：{}", t.year);
    assert!((1..=12).contains(&t.month), "月：{}", t.month);
    assert!((1..=31).contains(&t.day), "日：{}", t.day);
    assert!(t.hour < 24, "时：{}", t.hour);
    assert!(t.minute < 60, "分：{}", t.minute);
    assert!(t.second < 60, "秒：{}", t.second);
    assert!(t.millis < 1000, "毫秒：{}", t.millis);
}
