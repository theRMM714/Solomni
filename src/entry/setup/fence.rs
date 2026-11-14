//! 目的：围栏的机器级一次性预授（--setup=fence 与首次授权后的自动那一趟共用）。
//! 管：把落点清单交给 kernel 的 confine::machine_ensure，逐条打印进度与耗时，映射退出码；
//!   命令入口另把旧的全应用包机器级授权交给 confine::retire_legacy_machine_grants 退役；
//!   `--setup=fence-calibrate` 用合成树标定本卷授权速率并写成本样本（`record_grant_cost`）。
//! 不管：落点怎么选（kernel 的 confine/windows/machine.rs）、ACE 怎么写（acl.rs）。
//! 联动：entry::setup 分派；main.rs 首次授权后调 ensure；启动只读对账调 missing。

use std::path::Path;

/// 目的：--setup=fence 命令入口：逐条打印进度，成功 0 / 失败 1。
/// 约束：预授成功后顺带退役旧的全应用包机器级授权（能力 SID 已就位才撤，不留读不到的窗口）；
///   首次授权后的自动那一趟只走 ensure，不做这件慢清理。
pub(crate) fn command(root: &Path) -> i32 {
    if !crate::kernel::api::HAS_EXTERNAL_ACL_STEP {
        println!("[便携] 本平台没有机器级 ACL 预授（已登记缺口）：围栏的权限模型不写 ACL");
        return 0;
    }
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

/// 目的：--setup=fence-calibrate 命令入口：自建合成树真写一次机器级只读 ACL，测完后把本卷速率写进成本簿。
/// 约束：只动私有区里的自建目录；用后清理（撤权由 kernel 完成，这里只删树）。
pub(crate) fn calibrate_command(root: &Path) -> i32 {
    if !crate::kernel::api::HAS_EXTERNAL_ACL_STEP {
        eprintln!("[便携] 本平台没有机器级 ACL 标定（已登记缺口）：围栏的权限模型不写 ACL");
        return 1;
    }
    let home = root.join(".home");
    println!("[便携] 围栏授权成本标定：自建合成树，真写一次机器级只读 ACL（用后清理）");
    match crate::kernel::detail::confine::calibrate_machine_grant(&home) {
        Ok(cal) => {
            crate::capabilities::workspace::api::record_grant_cost(
                &home,
                &cal.path,
                cal.objects,
                cal.ms,
            );
            println!(
                "[便携] 标定完成：{} 个对象、{} ms；样本已写入 .home/fence-costs.json",
                cal.objects, cal.ms
            );
            match crate::kernel::detail::confine::remove_calibration_tree(&cal.path) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("[便携] 标定目录清理未完成：{}", e);
                    1
                }
            }
        }
        Err(e) => {
            eprintln!("[便携] 围栏授权成本标定失败：{}", e);
            1
        }
    }
}

pub(crate) fn ensure(root: &Path, home: &Path) -> Result<(), String> {
    crate::kernel::detail::confine::machine_ensure(root, home, &mut |line| {
        println!("[便携] {}", line)
    })
}

/// 目的：还缺哪些机器级落点（只读；空 = 已就绪）。
pub(crate) fn missing(root: &Path) -> Result<Vec<String>, String> {
    crate::kernel::detail::confine::machine_missing(root)
}
