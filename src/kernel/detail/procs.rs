//! 目的：长驻子进程的账与回收——谁在跑、属于谁、什么时候起的；核心没了也不留孤儿。
//! 管：登记表（`.home/processes.json`）、Windows 作业对象绑定与 Unix 父死子亡、存活探测与终止、启动时对账。
//! 不管：进程怎么起（各适配器自己 new Command）、业务语义（purpose / owner 只是标签，不参与决策）。
//! 联动：vm 的引擎与共享适配器在起进程处登记；组合根启动时对一次账，`--doctor` 与 `/api/processes` 把它读出来。

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};

/// 目的：一条登记——一个我们亲手起的长驻子进程。
/// 约束：`marker` 要能在它的命令行里找到（Unix 对账时靠它确认"这个 pid 还是我们那一个"，避免误杀别人的进程）。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    pub pid: u32,
    /// 目的：起的时间（unix 秒）——只作呈现与"新旧"判断，不参与身份确认。
    pub since: u64,
    /// 目的：用途标签（guest / share:virtiofs / share:smb）。
    pub purpose: String,
    /// 目的：归属（租约 = 会话名；共享则记落点）。
    pub owner: String,
    /// 目的：命令行里应出现的标记。
    pub marker: String,
}

/// 目的：启动对账的结果——死条目删掉几条、收掉几个孤儿、还有哪些"账上有且活着"的（如实报，不假装）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub dropped: usize,
    pub killed: usize,
    pub alive: Vec<Entry>,
}

impl Report {
    /// 目的：对账有没有动过什么（组合根据此决定要不要打印一行）。
    pub fn touched(&self) -> bool {
        self.dropped > 0 || self.killed > 0 || !self.alive.is_empty()
    }
}

/// 目的：长驻子进程的账——落 `.home/processes.json`，进程内一份内存表（跨线程共享）。
pub struct ProcessBook {
    path: PathBuf,
    entries: Mutex<Vec<Entry>>,
}

impl ProcessBook {
    /// 目的：打开账本（读不懂就当空账：账坏了不该挡住启动，但会如实少一份记录）。
    pub fn open(home: &Path) -> Arc<ProcessBook> {
        let path = home.join("processes.json");
        let entries = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<Vec<Entry>>(&t).ok())
            .unwrap_or_default();
        let book = ProcessBook {
            path,
            entries: Mutex::new(entries),
        };
        // 账本只在该目录存在时才有意义；首次启动时补建它。
        if let Some(dir) = book.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Arc::new(book)
    }

    /// 目的：记一条（同一 pid 只留最新一条）。
    pub fn record(&self, entry: Entry) {
        let mut book = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        book.retain(|e| e.pid != entry.pid);
        book.push(entry);
        self.save(&book);
    }

    /// 目的：注销一个 pid（进程收掉了；幂等）。
    pub fn forget(&self, pid: u32) {
        let mut book = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let before = book.len();
        book.retain(|e| e.pid != pid);
        if book.len() != before {
            self.save(&book);
        }
    }

    /// 目的：当前账上有什么（按起始时间升序：先起的先列）。
    pub fn list(&self) -> Vec<Entry> {
        let mut out = self
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        out.sort_by_key(|e| (e.since, e.pid));
        out
    }

    /// 目的：启动时对账——已经不在的删掉；还在的按平台收掉或如实报出来。
    /// 约束：Unix 上先确认命令行里有我们的标记才动手（pid 会被复用，误杀别人的进程是硬伤）；
    ///   Windows 上有作业对象兜底（核心一死它就跟着死），所以只报不动手。
    pub fn reconcile(&self) -> Report {
        let list = self.list();
        let mut report = Report::default();
        for entry in list {
            if !alive(entry.pid) {
                report.dropped += 1;
                self.forget(entry.pid);
                continue;
            }
            if cfg!(windows) {
                report.alive.push(entry);
                continue;
            }
            match cmdline(entry.pid) {
                Some(line) if !entry.marker.is_empty() && line.contains(&entry.marker) => {
                    if terminate(entry.pid).is_ok() {
                        report.killed += 1;
                        self.forget(entry.pid);
                    } else {
                        report.alive.push(entry);
                    }
                }
                // 命令行里没有我们的标记：这个 pid 已经是别人的了，账上的条目作废。
                _ => {
                    report.dropped += 1;
                    self.forget(entry.pid);
                }
            }
        }
        report
    }

    /// 目的：落盘（临时文件 + 改名：半个文件比没有更糟）。
    fn save(&self, book: &[Entry]) {
        let text = match serde_json::to_string_pretty(book) {
            Ok(t) => t,
            Err(_) => return,
        };
        let tmp = self.path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }
}

/// 目的：给要起的子进程上"父死子亡"的保险（在 spawn 之前调）。
/// 约束：Unix 走 pre_exec 设 PR_SET_PDEATHSIG + 独立进程组；Windows 在 spawn 之后经作业对象绑（见 `bind`）。
pub fn guard(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // 独立进程组：收整棵树时不用牵连自己。
        cmd.process_group(0);
        // 父进程没了就收信号：核心被强杀时，QEMU / 共享进程不该继续攥着镜像。
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                Ok(())
            });
        }
    }
    #[cfg(not(unix))]
    {
        let _ = cmd;
    }
}

/// 目的：起一个我们亲手管的长驻子进程——在专门的长命线程上 spawn。
/// 约束：PR_SET_PDEATHSIG 绑的是**发起 spawn 的那个线程**，不是进程；在每请求一线程（Web）或短命
///   工作线程上起 QEMU / 共享守护进程，线程一退就会把它们一并 SIGKILL。挪到长命线程上，父死子亡
///   仍按进程成立（核心真死了它才死），也不会误杀活着的会话。Windows 没有 PDEATHSIG，作业对象仍
///   由调用方 bind；直接 spawn，不绕线程。
pub fn spawn(cmd: Command) -> std::io::Result<Child> {
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

/// 目的：把刚起的子进程绑到本进程的作业对象上（Windows：核心一死，它们跟着死，不留孤儿）。
/// 约束：作业对象只建一次（KILL_ON_JOB_CLOSE）；绑失败只返回错误，由调用方如实记一条、不阻断起 guest。
pub fn bind(child: &Child) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use std::sync::OnceLock;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        static JOB: OnceLock<usize> = OnceLock::new();
        let job = *JOB.get_or_init(|| unsafe {
            let h = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
            if h.is_null() {
                return 0;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                return 0;
            }
            h as usize
        });
        if job == 0 {
            return Err("建作业对象失败（子进程不会被作业兜底）".to_string());
        }
        if unsafe { AssignProcessToJobObject(job as *mut _, child.as_raw_handle() as *mut _) } == 0
        {
            return Err("把子进程绑进作业对象失败".to_string());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = child;
        Ok(())
    }
}

/// 目的：这个 pid 还活着吗（探不到就当没了：账上留一条死记录比误判活着更糟）。
pub fn alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return false;
            }
            let mut code: u32 = 0;
            let ok = GetExitCodeProcess(h, &mut code);
            CloseHandle(h);
            ok != 0 && code == STILL_ACTIVE as u32
        }
    }
    #[cfg(not(windows))]
    {
        Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

/// 目的：收掉一个进程（先礼后兵：Unix 先 TERM 再 KILL；Windows 连整棵树）。
pub fn terminate(pid: u32) -> Result<(), String> {
    #[cfg(windows)]
    {
        let out = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output()
            .map_err(|e| format!("taskkill 起不来：{}", e))?;
        if out.status.success() {
            return Ok(());
        }
        Err(format!(
            "taskkill 失败：{}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
    #[cfg(not(windows))]
    {
        let pid_s = pid.to_string();
        let _ = Command::new("kill").args(["-TERM", &pid_s]).status();
        // 给它几秒落盘；还赖着就 KILL。
        for _ in 0..30 {
            if !alive(pid) {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let _ = Command::new("kill").args(["-KILL", &pid_s]).status();
        if alive(pid) {
            return Err(format!("收不掉进程 {}", pid));
        }
        Ok(())
    }
}

/// 目的：读一个 pid 的命令行（Unix 对账时确认身份用；Windows 没有便宜的读法，如实返回 None）。
fn cmdline(pid: u32) -> Option<String> {
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
    #[cfg(windows)]
    {
        let _ = pid;
        None
    }
}

/// 目的：当前时间（unix 秒），给登记表用。
pub fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
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
        // 否则并行跑用例时会把活着的自己的进程当认不出的死条目丢掉（用例随机红）。
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if cmdline(pid)
                .map(|l| l.contains("sleep 30"))
                .unwrap_or(false)
            {
                break;
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
        });
        let rep = book.reconcile();
        assert_eq!(rep.killed, 0, "认不出来就不该动手：{:?}", rep);
        assert!(alive(pid), "那个进程要还活着");
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 目的：长驻子进程**按进程**父死子亡，不能按线程——Web 每请求一线程，请求一完线程就退；
    ///   若在请求线程上直接 spawn，PR_SET_PDEATHSIG 会把 QEMU / 共享守护进程一并 SIGKILL（真机验收露的）。
    /// 约束：只在 Unix 上可判（Windows 走作业对象）；跑完收掉子进程。
    #[cfg(unix)]
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
}
