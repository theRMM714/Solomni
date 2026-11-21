//! 目的：`prep` 的非 Windows 后端——没有围栏授权可准备，只有机器级预授的如实降级。
//! 管：机器级预授的探测 / 授予 / 退役在非 Windows 上的结论（未支持或空操作）。
//! 不管：围栏准备本身（只有 Windows 有）；台账（在 `ledger`）。
//! 联动：由 `src/kernel/detail/confine/prep.rs` 在非 Windows 目标选中。

/// 目的：非 Windows 没有机器级 ACL 预授，如实返回未支持而不是假称就绪。
pub fn machine_missing(_root: &std::path::Path) -> Result<Vec<String>, String> {
    Err("本平台没有机器级 ACL 预授（已登记缺口）".to_string())
}

/// 目的：非 Windows 没有机器级 ACL 预授，如实返回未支持而不是假称就绪。
pub fn machine_ensure(
    _root: &std::path::Path,
    _home: &std::path::Path,
    _out: &mut dyn FnMut(&str),
) -> Result<(), String> {
    Err("本平台没有机器级 ACL 预授（已登记缺口）".to_string())
}

/// 目的：非 Windows 没有旧机器级授权可退役，如实为空操作。
pub fn retire_legacy_machine_grants(
    _home: &std::path::Path,
    _out: &mut dyn FnMut(&str),
) -> Result<(), String> {
    Ok(())
}
