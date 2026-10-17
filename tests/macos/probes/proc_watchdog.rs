//! 目的：长驻子进程"父死子亡"的 macOS 探针（L3）。
//! 管：按真实命令行驱动 `--proc-watchdog`：核心没了收掉子进程组、核心早没了也收、子进程早没了不动核心。
//! 不管：Linux 的 `PR_SET_PDEATHSIG` 与 Windows 的作业对象（各自由本平台验收）；围栏机制。
//! 联动：运行期由 `procs::watch` 拉起（`src/kernel/detail/procs.rs`）；分发在 `src/main.rs` / `src/guard/mod.rs`。

use crate::probe::bin;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// 起一个长命令，自己当进程组组长——运行期 `procs::guard` 干的就是这一步。
fn long_lived() -> Child {
    let mut cmd = Command::new("sh");
    cmd.args(["-c", "exec sleep 30"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn().expect("起一个长命令")
}

/// 按运行期同一条命令行起守门进程（自己的进程组、不继承环境）。
fn watchdog(parent: u32, child: u32) -> Child {
    let mut cmd = Command::new(bin());
    cmd.arg("--proc-watchdog")
        .arg(parent.to_string())
        .arg(child.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn().expect("起守门进程")
}

/// 等到进程退出（顺带收尸）；到点还没退就是 `None`。
fn wait_exit(child: &mut Child, within: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + within;
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return Some(st),
            Ok(None) => {}
            Err(e) => panic!("等进程退出时出错：{}", e),
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// 收掉一个还活着的进程（幂等）：先杀再收尸。
fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// 核心被强杀（不是正常退出）：子进程必须跟着没，守门进程自己正常退出。
#[test]
fn watchdog_kills_the_child_group_after_the_core_is_killed() {
    let mut core = long_lived();
    let mut child = long_lived();
    let mut dog = watchdog(core.id(), child.id());
    // 没有握手协议：给守门进程一点时间把两个 pid 注册进 kqueue（毫秒级）。
    std::thread::sleep(Duration::from_millis(300));
    stop(&mut core);
    let child_st = wait_exit(&mut child, Duration::from_secs(5));
    let dog_st = wait_exit(&mut dog, Duration::from_secs(5));
    // 断言前先把还活着的都收掉，失败时也不留进程。
    if child_st.is_none() {
        stop(&mut child);
    }
    if dog_st.is_none() {
        stop(&mut dog);
    }
    assert!(
        child_st.as_ref().map(|s| !s.success()).unwrap_or(false),
        "核心被强杀后子进程必须被收掉（等到的退出：{:?}）",
        child_st
    );
    assert!(
        dog_st.as_ref().map(|s| s.success()).unwrap_or(false),
        "守门进程要正常退出（等到的退出：{:?}）",
        dog_st
    );
}

/// 守门进程起得比核心的死还晚：注册核心拿到 ESRCH 时要把子进程组立刻收掉。
#[test]
fn watchdog_collects_the_child_when_the_core_is_already_gone() {
    let mut core = long_lived();
    let core_pid = core.id();
    // 已退出并收尸：守门进程注册它时只会拿到 ESRCH。
    stop(&mut core);
    let mut child = long_lived();
    let mut dog = watchdog(core_pid, child.id());
    let child_st = wait_exit(&mut child, Duration::from_secs(5));
    let dog_st = wait_exit(&mut dog, Duration::from_secs(5));
    if child_st.is_none() {
        stop(&mut child);
    }
    if dog_st.is_none() {
        stop(&mut dog);
    }
    assert!(
        child_st.as_ref().map(|s| !s.success()).unwrap_or(false),
        "核心已经没了：子进程必须被收掉（等到的退出：{:?}）",
        child_st
    );
    assert!(
        dog_st.as_ref().map(|s| s.success()).unwrap_or(false),
        "守门进程要正常退出（等到的退出：{:?}）",
        dog_st
    );
}

/// 子进程自己先收场：守门进程跟着退，不去 KILL 一个可能被复用的进程组，也不动核心。
#[test]
fn watchdog_leaves_the_core_alone_when_the_child_is_already_gone() {
    let mut core = long_lived();
    let mut child = long_lived();
    let child_pid = child.id();
    stop(&mut child);
    let mut dog = watchdog(core.id(), child_pid);
    let dog_st = wait_exit(&mut dog, Duration::from_secs(5));
    let core_still_running = core.try_wait().expect("try_wait 核心").is_none();
    stop(&mut core);
    if dog_st.is_none() {
        stop(&mut dog);
    }
    assert!(
        dog_st.as_ref().map(|s| s.success()).unwrap_or(false),
        "子进程早没了：守门进程该直接退出（等到的退出：{:?}）",
        dog_st
    );
    assert!(core_still_running, "没有可收的子进程时，核心不该被动");
}
