//! 目的：`procs` 的非 Windows 后端——进程组、PDEATHSIG、macOS 守门进程与信号终止。
//! 管：父死子亡、长命线程 spawn、命令行身份、存活探测与先 TERM 后 KILL。
//! 不管：账本读写与对账流程（在门面）；Windows 的作业对象与创建时间。
//! 联动：由 `src/kernel/detail/procs.rs` 在非 Windows 目标选中。

use super::*;
use std::process::Command;

/// 目的：Unix 起独立进程组；Linux 另挂内核级父死子亡信号。
pub(super) fn guard(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
        // 父进程没了就收信号：核心被强杀时，QEMU / 共享进程不该继续攥着镜像。
        #[cfg(target_os = "linux")]
        {
            unsafe {
                cmd.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                    Ok(())
                });
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = cmd;
    }
}

/// 目的：Unix 在专门的长命线程上 spawn（PDEATHSIG 绑的是发起线程）。
pub(super) fn spawn(cmd: Command) -> std::io::Result<Child> {
    #[cfg(unix)]
    {
        let (reply, wait) = std::sync::mpsc::channel();
        launcher()
            .send(Launch { cmd, reply })
            .map_err(|_| std::io::Error::other("进程启动线程没了"))?;
        wait.recv()
            .unwrap_or_else(|_| Err(std::io::Error::other("进程启动线程没了")))
    }
    #[cfg(not(unix))]
    {
        let mut cmd = cmd;
        cmd.spawn()
    }
}

/// 目的：一次移到长命线程上 spawn 的投递。
#[cfg(unix)]
struct Launch {
    cmd: Command,
    reply: std::sync::mpsc::Sender<std::io::Result<Child>>,
}

/// 目的：那个长命线程——一次起好，直到进程退出；受 guard 保护的长驻子进程都在它上面 spawn。
#[cfg(unix)]
fn launcher() -> &'static std::sync::mpsc::Sender<Launch> {
    static LAUNCHER: std::sync::OnceLock<std::sync::mpsc::Sender<Launch>> =
        std::sync::OnceLock::new();
    LAUNCHER.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<Launch>();
        std::thread::Builder::new()
            .name("solomni-proc-launcher".to_string())
            .spawn(move || {
                while let Ok(mut job) = rx.recv() {
                    let r = job.cmd.spawn();
                    let _ = job.reply.send(r);
                }
            })
            .expect("起进程启动线程");
        tx
    })
}

/// 目的：非 Windows 没有作业对象，绑定为空操作。
pub(super) fn bind(_child: &Child) -> Result<(), String> {
    Ok(())
}

/// 目的：Unix 没有便宜可信的创建时间读法，身份改由命令行标记证明。
pub(super) fn created(_pid: u32) -> Option<u64> {
    None
}

/// 目的：macOS 另起守门进程盯住核心；其余 Unix 由内核信号兜。
pub(super) fn watch(child: &Child) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::process::CommandExt;
        let started = std::env::current_exe()
            .map_err(|e| format!("取不到本程序路径（{}）", e))
            .and_then(|exe| {
                let mut cmd = Command::new(exe);
                cmd.arg(WATCHDOG_FLAG)
                    .arg(std::process::id().to_string())
                    .arg(child.id().to_string())
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .env_clear()
                    .process_group(0);
                cmd.spawn().map_err(|e| format!("起不了守门进程（{}）", e))
            });
        // 收尸：子进程或核心收场时守门进程就退出，别让它变成僵尸。
        let mut dog = started?;
        std::thread::spawn(move || {
            let _ = dog.wait();
        });
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = child;
        Ok(())
    }
}

/// 目的：macOS 的守门进程本体；其余平台真被调起时如实报错。
pub(super) fn watchdog_serve(parent: u32, child: u32) -> i32 {
    #[cfg(target_os = "macos")]
    {
        let kq = unsafe { libc::kqueue() };
        if kq < 0 {
            eprintln!(
                "[进程] 守门进程建不起 kqueue（{}）",
                std::io::Error::last_os_error()
            );
            return 2;
        }
        // 先盯核心：注册不上只可能是它已经没了。
        if let Err(e) = watch_exit(kq, parent) {
            unsafe { libc::close(kq) };
            if e.raw_os_error() == Some(libc::ESRCH) {
                kill_group(child);
                return 0;
            }
            eprintln!("[进程] 守门进程盯不住核心（{}）", e);
            return 2;
        }
        // 再盯子进程：它已经退出就没什么可收的，也不去 KILL 一个可能被复用的组。
        if let Err(e) = watch_exit(kq, child) {
            unsafe { libc::close(kq) };
            if e.raw_os_error() == Some(libc::ESRCH) {
                return 0;
            }
            eprintln!("[进程] 守门进程盯不住子进程（{}）", e);
            return 2;
        }
        let mut event: libc::kevent = unsafe { std::mem::zeroed() };
        loop {
            let n =
                unsafe { libc::kevent(kq, std::ptr::null(), 0, &mut event, 1, std::ptr::null()) };
            if n < 0 {
                let e = std::io::Error::last_os_error();
                if e.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                eprintln!("[进程] 守门进程等不到事件（{}）", e);
                unsafe { libc::close(kq) };
                return 2;
            }
            if n == 0 {
                continue;
            }
            // packed 结构：字段按值读。
            let who = event.ident as u32;
            unsafe { libc::close(kq) };
            if who == parent {
                kill_group(child);
            }
            return 0;
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (parent, child);
        eprintln!("[进程] 守门进程只在 macOS 上存在：Linux 走 PR_SET_PDEATHSIG");
        2
    }
}

/// 目的：把一个 pid 的「退出」注册进 kqueue；它已经不在了就是 `ESRCH`。
#[cfg(target_os = "macos")]
fn watch_exit(kq: std::os::raw::c_int, pid: u32) -> std::io::Result<()> {
    let change = libc::kevent {
        ident: pid as usize,
        filter: libc::EVFILT_PROC,
        flags: libc::EV_ADD | libc::EV_ENABLE,
        fflags: libc::NOTE_EXIT,
        data: 0,
        udata: std::ptr::null_mut(),
    };
    let n = unsafe { libc::kevent(kq, &change, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
    if n < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// 目的：按进程组收掉子进程（它起的时候就是组长）。
#[cfg(target_os = "macos")]
fn kill_group(pid: u32) {
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

/// 目的：用 `kill -0` 判活。
pub(super) fn alive(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// 目的：先按进程组 TERM，再 KILL；不是组长就退回单 pid。
pub(super) fn terminate(pid: u32) -> Result<(), String> {
    // 组号 = 组长 pid：组长自己也在这一组里，按组发就等于连它一起发。
    let group = |sig: libc::c_int| unsafe { libc::kill(-(pid as i32), sig) == 0 };
    let one = |sig: libc::c_int| {
        unsafe { libc::kill(pid as i32, sig) };
    };
    if !group(libc::SIGTERM) {
        one(libc::SIGTERM);
    }
    // 给它几秒落盘；组长还在、或组里还有不理会 TERM 的成员，就继续等。
    for _ in 0..30 {
        if !alive(pid) && unsafe { libc::kill(-(pid as i32), 0) } != 0 {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    if !group(libc::SIGKILL) {
        one(libc::SIGKILL);
    }
    if alive(pid) || unsafe { libc::kill(-(pid as i32), 0) } == 0 {
        return Err(format!("收不掉进程 {}", pid));
    }
    Ok(())
}

/// 目的：对账的身份结论——命令行里带我们的标记才是我们的；读不到或没有标记就算别人的。
pub(super) fn ownership(entry: &Entry) -> Ownership {
    match cmdline(entry.pid) {
        Some(line) if !entry.marker.is_empty() && line.contains(&entry.marker) => Ownership::Ours,
        _ => Ownership::Foreign,
    }
}

/// 目的：读一个 pid 的命令行（Linux 走 `/proc`，其它 Unix 走 `ps`，无该机制的平台给 None）。
pub(super) fn cmdline(pid: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let raw = std::fs::read(format!("/proc/{}/cmdline", pid)).ok()?;
        let line: Vec<String> = raw
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).to_string())
            .collect();
        Some(line.join(" "))
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        let out = Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "command="])
            .output()
            .ok()?;
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        None
    }
}
