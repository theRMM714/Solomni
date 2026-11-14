//! 目的：围栏的机器级一次性预授（--setup=fence 与首次授权后的自动那一趟共用）。
//! 管：把落点清单交给 kernel 的 confine::machine_ensure，逐条打印进度与耗时，映射退出码；
//!   命令入口另把旧的全应用包机器级授权交给 confine::retire_legacy_machine_grants 退役。
//! 不管：落点怎么选（kernel 的 confine/windows/machine.rs）、ACE 怎么写（acl.rs）。
//! 联动：entry::setup 分派；main.rs 首次授权后调 ensure；启动只读对账调 missing。

use std::path::Path;

/// 目的：--setup=fence 命令入口：逐条打印进度，成功 0 / 失败 1。
/// 约束：预授成功后顺带退役旧的全应用包机器级授权（能力 SID 已就位才撤，不留读不到的窗口）；
///   首次授权后的自动那一趟只走 ensure，不做这件慢清理。
#[cfg(windows)]
pub(crate) fn command(root: &Path) -> i32 {
    let home = root.join(".home");
    println!("[便携] 围栏机器级预授：只需成功一次；之后新会话不再重写这批 ACL");
    if let Err(e) = ensure(root, &home) {
        eprintln!("[便携] 围栏机器级预授失败：{}", e);
        return 1;
    }
    println!("[便携] 围栏机器级预授完成");
    match crate::kernel::detail::confine::retire_legacy_machine_grants(&home, &mut |line| {
        println!("[便携] {}", line)
    }) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("[便携] 旧全应用包授权退役未完成：{}", e);
            1
        }
    }
}

#[cfg(windows)]
pub(crate) fn ensure(root: &Path, home: &Path) -> Result<(), String> {
    crate::kernel::detail::confine::machine_ensure(root, home, &mut |line| {
        println!("[便携] {}", line)
    })
}

/// 目的：还缺哪些机器级落点（只读；空 = 已就绪）。
#[cfg(windows)]
pub(crate) fn missing(root: &Path) -> Result<Vec<String>, String> {
    crate::kernel::detail::confine::machine_missing(root)
}

#[cfg(not(windows))]
pub(crate) fn command(_root: &Path) -> i32 {
    println!("[便携] 本平台没有机器级 ACL 预授（已登记缺口）：围栏的权限模型不写 ACL");
    0
}

#[cfg(not(windows))]
pub(crate) fn ensure(root: &Path, home: &Path) -> Result<(), String> {
    crate::kernel::detail::confine::machine_ensure(root, home, &mut |_line| {})
}

#[cfg(not(windows))]
pub(crate) fn missing(root: &Path) -> Result<Vec<String>, String> {
    crate::kernel::detail::confine::machine_missing(root)
}
