//! 目的：进程账与守门 的测试：适配器的真实契约（真起进程、真收输出、真判退出码）。
//! 管：只覆盖本模块的公开契约与平台分支；夹具在本文件内。
//! 不管：上层调用方的装配（在 `src/tests/`）、其它适配器。
//! 联动：与 `src/kernel/detail/procs.rs` 同模块，`#[cfg(test)] mod tests;` 指到这里。

use super::*;

/// 目的：账本落盘后可被另一个实例（相当于重启后）读到；注销后两边都清掉。
#[test]
fn the_book_round_trips_through_disk() {
    let dir = crate::tests::scratch("procs-book");
    let home = dir.join(".home");
    let book = ProcessBook::open(&home);
    book.record(Entry {
        pid: 4711,
        since: 7,
        purpose: "guest".to_string(),
        owner: "w-1".to_string(),
        marker: "solomni:w-1".to_string(),
        created: None,
        guard_note: None,
    });
    let reopened = ProcessBook::open(&home);
    assert_eq!(reopened.list(), book.list(), "落了盘就该读得回来");
    reopened.forget(4711);
    assert!(reopened.list().is_empty(), "注销生效");
    assert!(
        ProcessBook::open(&home).list().is_empty(),
        "清掉的那条不该再落盘"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 目的：对账把"账上有、进程已经没了"的条目清掉（重启后最常见的残留）。
#[test]
fn reconcile_drops_entries_whose_process_is_gone() {
    let dir = crate::tests::scratch("procs-reconcile-dead");
    let mut cmd = Command::new(if cfg!(windows) { "cmd" } else { "true" });
    if cfg!(windows) {
        cmd.args(["/C", "exit 0"]);
    }
    let mut child = cmd.spawn().expect("起一个短命进程");
    let pid = child.id();
    let _ = child.wait();
    let book = ProcessBook::open(&dir.join(".home"));
    book.record(Entry {
        pid,
        since: 1,
        purpose: "guest".to_string(),
        owner: "w".to_string(),
        marker: "genuinely-not-in-any-cmdline".to_string(),
        created: None,
        guard_note: None,
    });
    let rep = book.reconcile();
    assert_eq!(rep.dropped, 1, "死条目要清掉");
    assert!(book.list().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 目的：Unix 上确认命令行标记后收掉上次留下的孤儿（Windows 靠作业对象，这里只验 Unix 的确认 + 终止）。
#[cfg(unix)]
#[test]
fn reconcile_reclaims_a_leftover_child_it_can_identify() {
    let dir = crate::tests::scratch("procs-reconcile-alive");
    let mut child = Command::new("sh")
        .args(["-c", "sleep 30"])
        .spawn()
        .expect("起一个长命令");
    let pid = child.id();
    // fork 到 exec 之间 /proc/<pid>/cmdline 还是测试进程自己的：必须等它真 exec 完再对账，
    // 否则并行跑用例时会把活着的自己的进程当认不出的死条目丢掉；等不到就当场红并说清卡在哪一步。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let line = super::unix::cmdline(pid).unwrap_or_default();
        // exec 之后 argv 是 `sleep 30`（有的 sh 会先 fork 再 exec：那时是 `sh -c sleep 30`）。
        if line.ends_with("sleep 30") {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!("子进程 30 秒内没 exec 成 sleep 30（现在读到：{:?}）", line);
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // 谁起的谁收尸：被 TERM 掉的进程在收尸前是僵尸，kill -0 仍算活着，terminate 会判成收不掉；
    // 生产里起进程的那一处也持着 Child 收尸，所以这里照同一条路走。
    let waiter = std::thread::spawn(move || child.wait());
    let book = ProcessBook::open(&dir.join(".home"));
    book.record(Entry {
        pid,
        since: 1,
        purpose: "guest".to_string(),
        owner: "w".to_string(),
        marker: "sleep 30".to_string(),
        created: None,
        guard_note: None,
    });
    let rep = book.reconcile();
    assert_eq!(rep.killed, 1, "认出来就该收掉：{:?}", rep);
    assert!(book.list().is_empty());
    let st = waiter.join().expect("join 收尸线程").expect("收尸");
    assert!(!st.success(), "被收掉的进程不该是正常退出");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 目的：命令行里没有我们的标记就不动手（pid 被复用时，别把别人的进程杀了）。
#[cfg(unix)]
#[test]
fn reconcile_never_kills_a_process_it_cannot_identify() {
    let dir = crate::tests::scratch("procs-reconcile-foreign");
    let mut child = Command::new("sh")
        .args(["-c", "sleep 30"])
        .spawn()
        .expect("起一个长命令");
    let pid = child.id();
    let book = ProcessBook::open(&dir.join(".home"));
    book.record(Entry {
        pid,
        since: 1,
        purpose: "guest".to_string(),
        owner: "w".to_string(),
        marker: "definitely-not-in-this-cmdline".to_string(),
        created: None,
        guard_note: None,
    });
    let rep = book.reconcile();
    assert_eq!(rep.killed, 0, "认不出来就不该动手：{:?}", rep);
    assert!(alive(pid), "那个进程要还活着");
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
}

/// 目的：长驻子进程**按进程**父死子亡，不能按线程——Web 每请求一线程，请求一完线程就退；
///   若在请求线程上直接 spawn，PR_SET_PDEATHSIG 会把 QEMU / 共享守护进程一并 SIGKILL。
/// 约束：只在 Linux 上可判（父死子亡只有它由内核信号给；macOS 走守门进程、Windows 走作业对象）；跑完收掉子进程。
#[cfg(target_os = "linux")]
#[test]
fn a_guarded_child_survives_the_spawning_thread() {
    let mut child = std::thread::spawn(|| {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 30"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        guard(&mut cmd);
        spawn(cmd).expect("在短命线程上起子进程")
    })
    .join()
    .expect("启动线程 join");
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(
        child.try_wait().expect("try_wait").is_none(),
        "发起 spawn 的线程退出后，长驻子进程不该跟着死"
    );
    let _ = terminate(child.id());
    let _ = child.wait();
}

/// 目的：父死子亡的**另一半**——父进程被强杀，受保护的长命令要跟着没（不是线程退出那种）。
/// 约束：测试进程杀不了自己，所以让本测试二进制再起一份当"核心"（见 `guard_helper_spawns_a_guarded_child`），
///   父测试强杀它，再验孙进程没了；只在 Linux 上可判（macOS 走守门进程、Windows 走作业对象）。
#[cfg(target_os = "linux")]
#[test]
fn a_guarded_child_dies_with_the_process_that_started_it() {
    let dir = crate::tests::scratch("procs-parent-death");
    let pidfile = dir.join("child.pid");
    let me = std::env::current_exe().expect("取本测试二进制");
    let mut core = Command::new(me)
        .args([
            "kernel::detail::procs::tests::guard_helper_spawns_a_guarded_child",
            "--exact",
            "--nocapture",
        ])
        .env("SOLOMNI_TEST_GUARD_PIDFILE", &pidfile)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("起当核心的那一份");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let pid = loop {
        if let Some(pid) = std::fs::read_to_string(&pidfile)
            .ok()
            .and_then(|t| t.trim().parse::<u32>().ok())
        {
            break pid;
        }
        if std::time::Instant::now() >= deadline {
            let _ = core.kill();
            let _ = core.wait();
            panic!("当核心的那一份没交出受保护子进程的 pid");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    // 核心被强杀（不是正常退出）：受保护的长命令必须跟着没。
    let _ = core.kill();
    let _ = core.wait();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline && !is_dead(pid) {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    if !is_dead(pid) {
        let _ = terminate(pid);
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(is_dead(pid), "核心被强杀后，受保护的子进程不该还活着");
}

/// 目的：上面那条用例的"核心替身"：起一个受 guard 保护的长命令，把 pid 交给父测试，然后等着被杀。
/// 约束：正常跑（没有 pidfile 环境变量）时只验 guard 的静态前提（独立进程组），不做空过的用例。
#[cfg(target_os = "linux")]
#[test]
fn guard_helper_spawns_a_guarded_child() {
    let mut cmd = Command::new("sh");
    cmd.args(["-c", "exec sleep 30"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    guard(&mut cmd);
    let mut child = spawn(cmd).expect("起受保护的长命令");
    let Some(pidfile) = std::env::var_os("SOLOMNI_TEST_GUARD_PIDFILE") else {
        let pgid = unsafe { libc::getpgid(child.id() as i32) };
        let _ = terminate(child.id());
        let _ = child.wait();
        assert_eq!(pgid, child.id() as i32, "guard 要给子进程独立进程组");
        return;
    };
    std::fs::write(pidfile, child.id().to_string()).expect("交 pid 给父测试");
    // 等父测试强杀本进程：孙进程随 PDEATHSIG 一起没。
    std::thread::sleep(std::time::Duration::from_secs(30));
    let _ = terminate(child.id());
    let _ = child.wait();
}

/// 目的：起进程这一处要把"父死子亡挂上没有"交回来，而不是丢掉——挂不上时账与 `--doctor` 靠它标出会留孤儿的条目。
/// 约束：Linux 由 spawn 之前的内核信号兜（这层本来就成立），Windows 靠作业对象；身份只在 Windows 由创建时间给出。
#[test]
fn attach_reports_the_deadman_state_and_the_identity() {
    let mut child = Command::new(if cfg!(windows) { "cmd" } else { "sh" })
        .args(if cfg!(windows) {
            vec!["/C", "ping -n 20 127.0.0.1"]
        } else {
            vec!["-c", "sleep 20"]
        })
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("起一个长命令");
    let launched = attach(&child, "test");
    assert!(
        launched.guard_note.is_none(),
        "本平台这层该挂得上：{:?}",
        launched.guard_note
    );
    if cfg!(windows) {
        assert!(
            launched.created.is_some(),
            "Windows 要交出创建时间（对账的身份）"
        );
    } else {
        assert!(
            launched.created.is_none(),
            "Unix 的身份来自命令行标记，不是创建时间"
        );
    }
    // 谁起的谁收尸：收尸前是僵尸，判活会失真（与本文件对那一条用例同一条路）。
    let pid = child.id();
    let waiter = std::thread::spawn(move || child.wait());
    let _ = terminate(pid);
    let _ = waiter.join();
}

/// 目的：Windows 上对账按**创建时间**确认身份后直接收掉上次留下的孤儿（作业对象兜底之外的那条路）。
#[cfg(windows)]
#[test]
fn reconcile_reclaims_a_leftover_it_identifies_by_creation_time() {
    let dir = crate::tests::scratch("procs-windows-identity");
    let mut child = Command::new("cmd")
        .args(["/C", "ping -n 30 127.0.0.1"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("起一个长命令");
    let launched = attach(&child, "test");
    let pid = child.id();
    // 谁起的谁收尸：被收掉的进程在收尸前是僵尸，判活会失真。
    let waiter = std::thread::spawn(move || child.wait());
    let book = ProcessBook::open(&dir.join(".home"));
    book.record(Entry {
        pid,
        since: 1,
        purpose: "guest".to_string(),
        owner: "w".to_string(),
        marker: String::new(),
        created: launched.created,
        guard_note: None,
    });
    let rep = book.reconcile();
    assert_eq!(rep.killed, 1, "身份对得上就该收掉：{:?}", rep);
    assert!(book.list().is_empty());
    let _ = waiter.join();
    let _ = std::fs::remove_dir_all(&dir);
}

/// 目的：Windows 上创建时间对不上 = pid 已被复用：作废条目，但**绝不动手**（误杀别人的进程是硬伤）。
#[cfg(windows)]
#[test]
fn reconcile_never_kills_a_pid_whose_creation_time_differs() {
    let dir = crate::tests::scratch("procs-windows-reused");
    let mut child = Command::new("cmd")
        .args(["/C", "ping -n 30 127.0.0.1"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("起一个长命令");
    let launched = attach(&child, "test");
    let pid = child.id();
    let book = ProcessBook::open(&dir.join(".home"));
    book.record(Entry {
        pid,
        since: 1,
        purpose: "guest".to_string(),
        owner: "w".to_string(),
        marker: String::new(),
        created: launched.created.map(|t| t.wrapping_add(1)),
        guard_note: None,
    });
    let rep = book.reconcile();
    assert_eq!(rep.killed, 0, "认出来不是那一个就不该动手：{:?}", rep);
    assert_eq!(rep.dropped, 1, "复用掉的条目要作废");
    assert!(alive(pid), "那个进程要还活着");
    let _ = terminate(pid);
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
}

/// 目的：账上没记身份（更早的账 / 读不到创建时间）时只报不动手——宁可不收，也不误杀。
#[cfg(windows)]
#[test]
fn reconcile_only_reports_an_entry_without_identity() {
    let dir = crate::tests::scratch("procs-windows-no-identity");
    let mut child = Command::new("cmd")
        .args(["/C", "ping -n 30 127.0.0.1"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("起一个长命令");
    let pid = child.id();
    let book = ProcessBook::open(&dir.join(".home"));
    book.record(Entry {
        pid,
        since: 1,
        purpose: "guest".to_string(),
        owner: "w".to_string(),
        marker: String::new(),
        created: None,
        guard_note: None,
    });
    let rep = book.reconcile();
    assert_eq!(rep.killed, 0, "没有身份就不动手：{:?}", rep);
    assert_eq!(rep.alive.len(), 1, "只如实报出来");
    assert!(alive(pid));
    let _ = terminate(pid);
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
}

/// 目的：作业对象绑不上的失败分支能被造出来（只带查询权的句柄，系统会拒绝），
///   起进程处据此如实报"没挂上"而不是静默——不引入测试开关，失败原因就是真实的那一条。
#[cfg(windows)]
#[test]
fn assign_rejects_a_handle_without_the_rights() {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    let mut child = Command::new("cmd")
        .args(["/C", "ping -n 30 127.0.0.1"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("起一个长命令");
    let pid = child.id();
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    assert!(!h.is_null(), "拿得到只带查询权的句柄");
    let r = super::windows::assign(h);
    unsafe { CloseHandle(h) };
    assert!(r.is_err(), "权不够的句柄不该绑得进去：{:?}", r);
    let _ = terminate(pid);
    let _ = child.wait();
}

/// 目的：没有 macOS 守门进程模式的平台，真被调起时如实报错（退出码 2）。
#[test]
fn watchdog_serve_reports_unsupported_off_macos() {
    #[cfg(not(target_os = "macos"))]
    assert_eq!(super::watchdog_serve(0, 0), 2);
}

/// 目的：这个 pid 是不是已经没了（僵尸也算没了：它已经退出，只是还没被收尸）。
#[cfg(target_os = "linux")]
fn is_dead(pid: u32) -> bool {
    match std::fs::read_to_string(format!("/proc/{}/stat", pid)) {
        Err(_) => true,
        Ok(stat) => stat
            .rsplit_once(')')
            .map(|(_, rest)| rest.trim_start().starts_with('Z'))
            .unwrap_or(false),
    }
}
