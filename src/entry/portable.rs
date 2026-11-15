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
    if !crate::kernel::api::HAS_EXTERNAL_ACL_STEP {
        return true;
    }
    match SetupHost::new(root).fence_missing() {
        Ok(missing) if missing.is_empty() => true,
        Ok(missing) => {
            println!(
                "[便携] 围栏机器级授权还缺 {} 处（{}）：跑一次 solomni --setup=fence 可免掉新会话的重复 ACL 写",
                missing.len(),
                missing.join("；")
            );
            false
        }
        Err(e) => {
            eprintln!("[便携] 围栏预授对账失败：{}", e);
            true
        }
    }
}

/// 目的：首次授权写盘后的那一趟：把机器级落点一次授好并显示进度（失败只告警，不拦重启）。
pub(crate) fn ensure_after_authorize(root: &Path) {
    let host = SetupHost::new(root);
    if crate::kernel::api::HAS_EXTERNAL_ACL_STEP {
        println!("[便携] 首次授权：预授围栏的机器级授权（只需一次，之后新会话不再重写这批 ACL）");
        if let Err(e) = host.fence_ensure(&mut |line| println!("[便携] {}", line)) {
            eprintln!(
                "[便携] 机器级预授未完成：{}（稍后可用 solomni --setup=fence 重跑）",
                e
            );
        }
    } else {
        let _ = host.fence_ensure(&mut |_| {});
    }
}
