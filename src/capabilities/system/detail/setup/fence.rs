//! 目的：围栏的机器级一次性预授（--setup fence 与首次授权后的自动那一趟共用）。
//! 管：把落点清单交给 kernel 的 confine::machine_ensure 并逐条交出进度；预授后退役旧的全应用包授权；
//!   fence-calibrate 用合成树标定本卷授权速率并写成本样本（record_grant_cost）。
//! 不管：落点怎么选（kernel 的 confine/windows/machine.rs）、ACE 怎么写（acl.rs）。
//! 联动：setup 的模式分派；entry 的启动对账调 missing / ensure。

use std::path::Path;

/// 目的：一次性预授：没能力就如实说缺口（不算失败），有能力就逐条写并退役旧授权。
pub(crate) fn grant(root: &Path, on_line: &mut dyn FnMut(&str)) -> Result<(), String> {
    if !crate::kernel::api::HAS_EXTERNAL_ACL_STEP {
        on_line("本平台没有机器级 ACL 预授（已登记缺口）：围栏的权限模型不写 ACL");
        return Ok(());
    }
    let home = root.join(".home");
    on_line("围栏机器级预授：只需成功一次；之后新会话不再重写这批 ACL");
    ensure(root, &home, on_line)?;
    on_line("围栏机器级预授完成");
    // 能力 SID 已就位才撤旧授权：不留读不到的窗口。
    crate::kernel::detail::confine::retire_legacy_machine_grants(&home, &mut |line| on_line(line))
}

/// 目的：标定本卷的机器级授权速率：自建合成树真写一次，测完把样本写进成本簿并清理合成树。
pub(crate) fn calibrate(root: &Path, on_line: &mut dyn FnMut(&str)) -> Result<(), String> {
    if !crate::kernel::api::HAS_EXTERNAL_ACL_STEP {
        return Err("本平台没有机器级 ACL 标定（已登记缺口）：围栏的权限模型不写 ACL".to_string());
    }
    let home = root.join(".home");
    on_line("围栏授权成本标定：自建合成树，真写一次机器级只读 ACL（用后清理）");
    let cal = crate::kernel::detail::confine::calibrate_machine_grant(&home)?;
    crate::capabilities::workspace::api::record_grant_cost(&home, &cal.path, cal.objects, cal.ms);
    on_line(&format!(
        "标定完成：{} 个对象、{} ms；样本已写入 .home/fence-costs.json",
        cal.objects, cal.ms
    ));
    crate::kernel::detail::confine::remove_calibration_tree(&cal.path)
}

/// 目的：把机器级落点一次授好（进度逐条经 on_line）。
pub(crate) fn ensure(
    root: &Path,
    home: &Path,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), String> {
    crate::kernel::detail::confine::machine_ensure(root, home, on_line)
}

/// 目的：还缺哪些机器级落点（只读；空 = 已就绪）。
pub(crate) fn missing(root: &Path) -> Result<Vec<String>, String> {
    crate::kernel::detail::confine::machine_missing(root)
}
