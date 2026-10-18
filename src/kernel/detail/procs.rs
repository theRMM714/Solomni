//! 目的：长驻子进程的账与回收——谁在跑、属于谁、什么时候起的；核心没了也不留孤儿。
//! 管：登记表（`.home/processes.json`）、父死子亡的接线与结论（Linux 内核信号 / macOS 守门进程 / Windows 作业对象）、
//!   进程身份（Unix 命令行标记 / Windows 创建时间）、存活探测与终止、启动时对账。
//! 不管：进程怎么起（各适配器自己 new Command）、业务语义（purpose / owner 只是标签，不参与决策）。
//! 联动：vm 的引擎、共享适配器与常驻服务的守门进程在起进程处登记；组合根启动时对一次账，`--doctor` 与 `/api/processes` 把它读出来；
//!   macOS 的父死子亡由本文件的守门模式实现，入口层只做 argv 分发（`main.rs` / `guard::proc_watch`）。

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};

/// 目的：macOS 的长驻子进程守门模式（内部协议）：`--proc-watchdog <核心 pid> <子进程 pid>`。
/// 约束：只在 macOS 上真起作用（Linux 由内核的 PDEATHSIG 兜、Windows 由作业对象兜）；用户不接触。
pub const WATCHDOG_FLAG: &str = "--proc-watchdog";

/// 目的：一条登记——一个我们亲手起的长驻子进程。
/// 约束：身份按平台用**能证明的判据**——Unix 是命令行里的 `marker`，Windows 是进程创建时间 `created`；
///   两者都用来确认"这个 pid 还是我们那一个"，避免误杀别人的进程。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    pub pid: u32,
    /// 目的：起的时间（unix 秒）——只作呈现与"新旧"判断，不参与身份确认。
    pub since: u64,
    /// 目的：用途标签（guest / share:virtiofs / share:smb / resident）。
    pub purpose: String,
    /// 目的：归属（租约 = 会话名；共享则记落点；常驻服务记 `resident/<模块>/<服务>`）。
    pub owner: String,
    /// 目的：命令行里应出现的标记。
    pub marker: String,
    /// 目的：身份（Windows：进程创建时间，FILETIME）；Unix 为 None（身份靠 `marker`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<u64>,
    /// 目的：父死子亡这一层没挂上的原因（None = 挂上了，或本平台由内核信号 / 守门进程兜）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard_note: Option<String>,
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

    /// 目的：启动时对账——已经不在的删掉；还在的确认身份后收掉，认不出来就如实报出来。
    /// 约束：误杀别人的进程是硬伤，所以先证明"这个 pid 还是我们那一个"：Unix 看命令行里的标记，
    ///   Windows 看进程创建时间；两者都证明不了（账上没记身份）就只报不动手。
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
                // Windows 没有便宜的命令行可读：身份靠创建时间。
                // 账上没有创建时间的条目（更早的账）只报不动手——宁可不收，也不误杀别人的进程。
                match (entry.created, created(entry.pid)) {
                    (Some(expected), Some(actual)) if expected == actual => {
                        if terminate(entry.pid).is_ok() {
                            report.killed += 1;
                            self.forget(entry.pid);
                        } else {
                            report.alive.push(entry);
                        }
                    }
                    // 创建时间对不上：这个 pid 已经是别人的了，账上的条目作废。
                    (Some(_), Some(_)) => {
                        report.dropped += 1;
                        self.forget(entry.pid);
                    }
                    // 对账前一瞬退掉了：当作死条目清掉。
                    (Some(_), None) => {
                        report.dropped += 1;
                        self.forget(entry.pid);
                    }
                    (None, _) => report.alive.push(entry),
                }
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
/// 约束：所有 Unix 都开独立进程组（收整棵树时不用牵连自己）；父死子亡只有 Linux 由内核给
///   （`PR_SET_PDEATHSIG`——macOS 的 libc 没有它），macOS 由 spawn 之后起的守门进程负责（见 `watch`）；
///   Windows 在 spawn 之后经作业对象绑（见 `bind`）。
pub fn guard(cmd: &mut Command) {
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

/// 目的：起一个我们亲手管的长驻子进程——在专门的长命线程上 spawn。
/// 约束：PR_SET_PDEATHSIG 绑的是**发起 spawn 的那个线程**，不是进程；在每请求一线程（Web）或短命
///   工作线程上起 QEMU / 共享守护进程，线程一退就会把它们一并 SIGKILL。挪到长命线程上，父死子亡
///   仍按进程成立（核心真死了它才死），也不会误杀活着的会话。Windows 走作业对象、macOS 走守门进程
///   （都与发起线程无关；Windows 直接 spawn，Unix 共用这一条路径，经长命线程没有副作用）。
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

/// 目的：一把起进程之后要记下的事实：身份（Windows 创建时间）与父死子亡兜底的结论。
pub struct Launched {
    pub created: Option<u64>,
    pub guard_note: Option<String>,
}

/// 目的：给刚起的子进程挂上"父死子亡"这一层（Windows 作业对象 / macOS 守门进程；Linux 在 spawn 之前由内核信号兜）。
/// 约束：挂不上只如实报一条、**不阻断启动**——这层兜底缺失时核心被强杀会留下这个进程，
///   所以把原因交给调用方记进账：`--doctor` 与界面据此把该条标成"会留孤儿"。
pub fn attach(child: &Child, purpose: &str) -> Launched {
    let guard_note = match bind(child).and_then(|()| watch(child)) {
        Ok(()) => None,
        Err(e) => {
            eprintln!(
                "[进程] {} 的父死子亡没挂上：{}（核心被强杀时这个进程会留下）",
                purpose, e
            );
            Some(e)
        }
    };
    Launched {
        created: created(child.id()),
        guard_note,
    }
}

/// 目的：把刚起的子进程绑到本进程的作业对象上（Windows：核心一死，它们跟着死，不留孤儿）。
/// 约束：作业对象只建一次（KILL_ON_JOB_CLOSE）；绑失败只返回错误，由 `attach` 如实报出来、不阻断启动。
pub fn bind(child: &Child) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        assign(child.as_raw_handle())
    }
    #[cfg(not(windows))]
    {
        let _ = child;
        Ok(())
    }
}

/// 目的：本进程唯一的作业对象（0 = 建不起来；只建一次，核心一死句柄一关，里面的进程跟着死）。
#[cfg(windows)]
fn job_object() -> usize {
    use std::sync::OnceLock;
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    static JOB: OnceLock<usize> = OnceLock::new();
    *JOB.get_or_init(|| unsafe {
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
    })
}

/// 目的：把一个进程句柄绑进作业对象；系统按句柄的访问权决定成败（权不够就拒绝）。
/// 约束：拆出来是为了让失败分支可注入——传一个只带查询权的句柄就能造出真实失败，不需要测试开关。
#[cfg(windows)]
pub(crate) fn assign(process: *mut std::ffi::c_void) -> Result<(), String> {
    use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
    let job = job_object();
    if job == 0 {
        return Err("建作业对象失败（子进程不会被作业兜底）".to_string());
    }
    if unsafe { AssignProcessToJobObject(job as *mut _, process) } == 0 {
        return Err("把子进程绑进作业对象失败".to_string());
    }
    Ok(())
}

/// 目的：进程身份（Windows：创建时间 FILETIME）——对账时用它区分"还是我们那一个"与"pid 被复用"。
/// 约束：Unix 没有同样便宜可信的读法，身份改由命令行标记证明，这里如实返回 None。
pub fn created(pid: u32) -> Option<u64> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
        use windows_sys::Win32::System::Threading::{
            GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let mut birth: FILETIME = std::mem::zeroed();
            let mut exit: FILETIME = std::mem::zeroed();
            let mut kernel: FILETIME = std::mem::zeroed();
            let mut user: FILETIME = std::mem::zeroed();
            let ok = GetProcessTimes(h, &mut birth, &mut exit, &mut kernel, &mut user);
            CloseHandle(h);
            if ok == 0 {
                return None;
            }
            Some(((birth.dwHighDateTime as u64) << 32) | birth.dwLowDateTime as u64)
        }
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        None
    }
}

/// 目的：macOS 的父死子亡——另起一个本程序的守门进程（`--proc-watchdog`）盯住核心，核心一死就收掉子进程组。
/// 约束：Linux 由内核的 `PR_SET_PDEATHSIG` 兜、Windows 由作业对象兜，这里只对 macOS 生效；
///   守门进程自己**不**挂 PDEATHSIG（它要活过发起 spawn 的线程），进独立进程组（终端信号不会先把它带走），
///   不继承环境（密钥与凭据不跟着它走）。起不来如实返回原因（由 `attach` 记一条），不阻断启动。
pub fn watch(child: &Child) -> Result<(), String> {
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

/// 目的：守门进程本体——盯住核心与子进程，谁先没了决定要不要收掉子进程组。
/// 返回：0 = 该收的收了（或本来就没得收）；2 = 盯不住（如实报错，不假装守住了）。
/// 约束：kqueue 的 `EVFILT_PROC` / `NOTE_EXIT` 在进程**退出**时就报（不要求父进程先收尸），
///   所以核心被强杀也能立刻收；同时盯子进程是为了在它正常收场时不误杀一个可能被复用的进程组。
#[cfg(target_os = "macos")]
pub fn watchdog_serve(parent: u32, child: u32) -> i32 {
    let kq = unsafe { libc::kqueue() };
    if kq < 0 {
        eprintln!(
            "[进程] 守门进程建不起 kqueue（{}）",
            std::io::Error::last_os_error()
        );
        return 2;
    }
    // 先盯核心：注册不上只可能是它已经没了（pid 复用要等整圈，这里按"没了"处理）。
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
        let n = unsafe { libc::kevent(kq, std::ptr::null(), 0, &mut event, 1, std::ptr::null()) };
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

/// 目的：把一个 pid 的"退出"注册进 kqueue；它已经不在了就是 `ESRCH`。
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

/// 目的：按进程组收掉子进程（它起的时候就是组长：`guard` 里 `process_group(0)`）。
#[cfg(target_os = "macos")]
fn kill_group(pid: u32) {
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

/// 目的：其它平台没有这个模式（Linux 由内核信号兜、Windows 由作业对象兜）：真被调起时如实报错。
#[cfg(not(target_os = "macos"))]
pub fn watchdog_serve(_parent: u32, _child: u32) -> i32 {
    eprintln!("[进程] 守门进程只在 macOS 上存在：Linux 走 PR_SET_PDEATHSIG，Windows 走作业对象");
    2
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
/// 约束：Unix 先按**进程组**发信号（我们起的长驻子进程都是组长：`guard` 里 `process_group(0)`，
///   组员含守门进程拉起的服务）；不是组长（测试里直接 spawn 的）就退回单 pid。
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
            let line = cmdline(pid).unwrap_or_default();
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
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
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
        let r = assign(h);
        unsafe { CloseHandle(h) };
        assert!(r.is_err(), "权不够的句柄不该绑得进去：{:?}", r);
        let _ = terminate(pid);
        let _ = child.wait();
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
}
