//! 目的：启动期的便携对账——围栏机器级预授的缺项提示与首次授权后的一次性预授。
//! 管：两个启动钩子（缺项提示、授权后预授）与它们的输出前缀；具体写入在 system 能力的便携维护实现里。
//! 不管：`--setup` 的模式分派与状态回执（在 capabilities::system 的 SetupOps 与 presentation/cli/boot.rs）。
//! 联动：main.rs 在启动授权后调 ensure_after_authorize、在启动报告里调 fence_hint。

use std::path::Path;

use crate::capabilities::system::api::SetupOps;
use crate::capabilities::system::detail::setup::SetupHost;

/// 目的：启动期只读对账：机器级预授缺项时打一条提示（不静默慢写、不阻断启动）。
/// 返回：true = 已就绪；false = 有缺项（调用方只提示一次）。
pub(crate) fn fence_hint(root: &Path) -> bool {
    fence_report(
        &SetupHost::new(root),
        crate::kernel::api::HAS_EXTERNAL_ACL_STEP,
        &|line| println!("{line}"),
        &|line| eprintln!("{line}"),
    )
}

/// 目的：首次授权写盘后的那一趟：把机器级落点一次授好并显示进度（失败只告警，不拦重启）。
pub(crate) fn ensure_after_authorize(root: &Path) {
    ensure_grants(
        &SetupHost::new(root),
        crate::kernel::api::HAS_EXTERNAL_ACL_STEP,
        &|line| println!("{line}"),
        &|line| eprintln!("{line}"),
    );
}

/// 目的：围栏对账的判定与文案：只有本平台有外部 ACL 那一步才需要对账。
/// 参数：`host` 是便携维护端口；`acl_step` 是本平台的 ACL 能力（生产取 kernel 的 HAS_EXTERNAL_ACL_STEP，用例两档都走）；`say`/`warn` 是提示与告警的出口。
/// 返回：true = 已就绪或无需对账；false = 有缺项。
pub(crate) fn fence_report(
    host: &dyn SetupOps,
    acl_step: bool,
    say: &dyn Fn(&str),
    warn: &dyn Fn(&str),
) -> bool {
    if !acl_step {
        return true;
    }
    match host.fence_missing() {
        Ok(missing) if missing.is_empty() => true,
        Ok(missing) => {
            say(&format!(
                "[便携] 围栏机器级授权还缺 {} 处（{}）：跑一次 solomni --setup=fence 可免掉新会话的重复 ACL 写",
                missing.len(),
                missing.join("；")
            ));
            false
        }
        Err(e) => {
            warn(&format!("[便携] 围栏预授对账失败：{e}"));
            true
        }
    }
}

/// 目的：首次授权后的一次性预授：有 ACL 一步的平台上先声明再逐条打进度，失败只告警。
/// 参数：`host` 是便携维护端口；`acl_step` 是本平台的 ACL 能力；`say`/`warn` 是提示与告警的出口。
pub(crate) fn ensure_grants(
    host: &dyn SetupOps,
    acl_step: bool,
    say: &dyn Fn(&str),
    warn: &dyn Fn(&str),
) {
    if !acl_step {
        let _ = host.fence_ensure(&mut |_| {});
        return;
    }
    say("[便携] 首次授权：预授围栏的机器级授权（只需一次，之后新会话不再重写这批 ACL）");
    if let Err(e) = host.fence_ensure(&mut |line| say(&format!("[便携] {line}"))) {
        warn(&format!(
            "[便携] 机器级预授未完成：{e}（稍后可用 solomni --setup=fence 重跑）"
        ));
    }
}
