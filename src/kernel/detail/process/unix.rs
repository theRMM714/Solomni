//! 目的：`process` 的非 Windows 后端——没有容器围栏准备，用进程组收树，shell 走 sh。
//! 管：进程组创建与按组杀树；把 shell 脚本经 stdin 交给 `sh -c`。
//! 不管：一次执行的公共轮询与回执（在门面）；围栏策略与文案。
//! 联动：由 `src/kernel/detail/process.rs` 在非 Windows 目标选中。

use super::*;
use crate::kernel::ports::ShellRun;

/// 目的：非 Windows 没有容器围栏，直接返回「未准备、无家目录」。
pub(super) fn prepare(_ctx: PrepareCtx<'_>) -> Prepared {
    Prepared::Ready {
        prepared: false,
        unfenced_note: None,
        home: None,
    }
}

/// 目的：Unix 起独立进程组；其它非 Windows 目标不做额外配置。
pub(super) fn configure_spawn(cmd: &mut std::process::Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(not(unix))]
    {
        let _ = cmd;
    }
}

/// 目的：按进程组杀整棵树；没有进程组语义的平台退回单进程终止。
pub(super) fn kill_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        // 负 pid = 整个进程组（守门进程是组长，组员含 shell 与工具本身）。
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
        let _ = child.wait();
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// 目的：非 Windows 的 shell——外层守门进程本身就是 `sh -c`，这里再起一个 sh 读 stdin。
/// 约束：脚本不会与命令的 stdin 抢同一条流。
pub(super) fn run_shell(tools: &ProcTools, run: &ShellRun<'_>) -> RunOutcome {
    let command = "sh -c \"$(cat)\"";
    tools.run_inner(
        run.fence,
        command,
        run.script,
        &[],
        run.path,
        run.ask,
        run.gate,
        RunPlan {
            timeout: run.timeout,
            strict: true,
            shell: true,
            cancel: run.cancel,
        },
    )
}
