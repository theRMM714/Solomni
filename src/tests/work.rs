//! 目的：`work` 的纯函数用例——工作名校验与 guest 运行包重建判据。
//! 管：名字挡掉路径与保留名、重建只在缺包时发生。
//! 不管：建工作、改配与视图（见 `src/tests/conductor.rs`、`src/tests/api/session.rs`）。
//! 联动：`src/capabilities/work/domain.rs`、`src/capabilities/work/service.rs`。

use crate::capabilities::work::domain::validate_work_name;
use crate::capabilities::work::service::needs_guest_packages_rebuild;

/// 目的：工作名挡掉路径分隔符、保留设备名与首尾空白，正常名字放行。
#[test]
pub(crate) fn a_work_name_rejects_paths_and_device_names() {
    assert!(validate_work_name("修登录").is_ok());
    assert!(validate_work_name("a/b").is_err(), "斜杠会被当成目录");
    assert!(validate_work_name("a\\b").is_err(), "反斜杠会被当成目录");
    assert!(validate_work_name("CON").is_err(), "系统保留名不能当工作名");
    assert!(
        validate_work_name("com1").is_err(),
        "保留的设备名不分大小写"
    );
    assert!(validate_work_name(" 先").is_err(), "首尾空白会落到目录名上");
    assert!(validate_work_name("").is_err(), "空名没有落盘目录");
}

/// 目的：只有 guest 已经起来且运行包集合变了才重建，其余情况不动它。
#[test]
pub(crate) fn guest_packages_are_rebuilt_only_when_the_guest_is_up_and_the_set_changed() {
    let want = vec!["node".to_string(), "python".to_string()];
    assert!(
        !needs_guest_packages_rebuild(false, &want, Some(&want)),
        "guest 没起来时不需要重建"
    );
    assert!(
        !needs_guest_packages_rebuild(true, &want, Some(&want)),
        "集合没变就不重建"
    );
    assert!(
        needs_guest_packages_rebuild(true, &want, Some(&["node".to_string()])),
        "guest 已起且集合变大要补装"
    );
    assert!(
        !needs_guest_packages_rebuild(true, &[], Some(&want)),
        "集合变小不是缺包，不重建"
    );
    assert!(
        needs_guest_packages_rebuild(true, &want, None),
        "guest 已起但没有安装记录时按需重建"
    );
}
