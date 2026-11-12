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
mod tests;
