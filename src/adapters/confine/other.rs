//! 其他平台后端：没有原生围栏机制可用——如实降级（只保证进程树与超时），绝不假装有文件系统围栏。

use super::{shell_command, Capability, FenceVerdict, FENCE_FAILED};
use crate::core::fence::FenceSpec;

pub fn capability() -> Capability {
    Capability {
        fs: false,
        net: false,
        tree: true,
        note: "本平台没有接入文件系统围栏（只有超时与整棵树终止）".to_string(),
    }
}

/// 本平台**没有**文件系统围栏机制可装：这是环境结论（"本平台没接入"），不是我们写错了，
/// 所以恒为 EnvUnavailable——未授权时段据此如实降级照跑，而不是拒绝执行。
pub fn verify(_spec: &FenceSpec, _command: &str) -> FenceVerdict {
    FenceVerdict::EnvUnavailable("本平台没有接入文件系统围栏（只有超时与整棵树终止）".to_string())
}

/// `_prepared`（外层是否已完成本机授权）与 `_home`（容器 profile 的台账落点）只有 Windows 的容器围栏用得上：
/// 本平台没有容器这一步。
pub fn run_fenced(
    spec: &FenceSpec,
    _prepared: bool,
    _home: Option<&std::path::Path>,
    command: &str,
) -> i32 {
    match shell_command(command).current_dir(&spec.cwd).status() {
        Ok(s) => s.code().unwrap_or(FENCE_FAILED),
        Err(e) => {
            eprintln!("[围栏] 工具进程启动失败：{}", e);
            FENCE_FAILED
        }
    }
}
