//! 目的：回环中继——容器里要访问的宿主本机端口，沿**继承来的匿名管道**接过去，只在 `NetPolicy::Local` 档启用。
//! 管：宿主侧（建管道、拉起中继进程、按帧转发、**在真正 dial 的那一步校验端口**）；容器侧（中继进程：绑 `127.0.0.2`、accept、转发，再跑真正的命令）。
//! 不管：围栏本身（在 `confine::windows`）、端口清单从哪来（在 tools 的 `shell.local_ports`）。
//! 联动：`confine::windows::run_fenced` 在 `net.is_local()` 时调 `run`；`src/entry/args.rs` 把 `--fence-relay` 分派到 `container::relay_entry`。

use super::{FENCE_FAILED, PSID, container_command, spawn_in_container, wait_and_reap};
use crate::kernel::contracts::fence::FencePlan;
use crate::kernel::detail::io::env;
use std::collections::HashMap;
use std::ffi::c_void;
use std::fs::File;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use windows_sys::Win32::Foundation::{
    CloseHandle, HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation,
};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCPROW_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, GetExitCodeProcess, INFINITE, OpenProcess, PROCESS_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW, STARTF_USESTDHANDLES,
    STARTUPINFOW, WaitForSingleObject,
};

/// 目的：容器侧监听地址——宿主的 `127.0.0.1:<端口>` 在容器里以**同一端口、这个地址**出现。
/// 约束：容器与宿主共用一个回环栈，宿主的服务正占着 `127.0.0.1:<端口>`，中继去 bind 同一地址只会撞端口（`WSAEADDRINUSE`）；
///   `127.0.0.2` 是同一条回环上的另一个地址，同端口即可并存。
pub(crate) const RELAY_ADDR: &str = "127.0.0.2";

/// 目的：内部协议旗（不进命令表、不进 `--help`）。
pub(crate) const RELAY_FLAG: &str = "--fence-relay";

/// 目的：宿主交给中继进程的三份事实——已经算好的整条命令行、它的 cwd、以及系统 shell 的路径。
/// 约束：为什么经环境变量给：命令行本身含中文与引号，塞进中继的命令行会被 `cmd` 再解释一遍。
const ENV_CMD: &str = "SOLOMNI_FENCE_RELAY_CMD";
const ENV_CWD: &str = "SOLOMNI_FENCE_RELAY_CWD";
const ENV_APP: &str = "SOLOMNI_FENCE_RELAY_APP";

/// 目的：一帧的种类：容器侧要连一个端口 / 一段字节 / 收尾 / 宿主拒绝。
const KIND_OPEN: u8 = 1;
const KIND_DATA: u8 = 2;
const KIND_CLOSE: u8 = 3;
const KIND_REFUSE: u8 = 4;

/// 目的：帧头长度（conn:u32 + kind:u8 + len:u32）。
const HEAD_LEN: usize = 9;
/// 目的：单帧载荷上限（两条管道都是本机句柄，超过这个数只可能是帧错位）。
const MAX_FRAME: usize = 1 << 20;

/// 目的：宿主侧——在一个容器身份下起中继进程，把 `ports` 里每个端口接到宿主的同名端口，跑完真正的命令后返回它的退出码。
/// 约束：任何一步起不来都如实返回原因（调用方据此拒绝执行，不降级到无围栏——那等于白送整个回环）。
/// 约束：不碰 loopback 边界（豁免要管理员、机器级、覆盖全端口），字节全走句柄继承，宿主一个端口都不对外开。
pub(crate) fn run(
    sid: PSID,
    spec: &FencePlan,
    command: &str,
    ports: &[u16],
) -> Result<i32, String> {
    let exe = env::current_exe().map_err(|e| format!("取本产品可执行文件路径失败：{e}"))?;
    let (in_read, in_write) = pipe()?;
    let (out_read, out_write) = pipe()?;
    // 子进程只拿「读命令」「写结果」这两端；另两端留在宿主手里，是另一端的存活信号。
    let ok = unsafe {
        SetHandleInformation(in_write, HANDLE_FLAG_INHERIT, 0) != 0
            && SetHandleInformation(out_read, HANDLE_FLAG_INHERIT, 0) != 0
    };
    if !ok {
        unsafe {
            CloseHandle(in_read);
            CloseHandle(in_write);
            CloseHandle(out_read);
            CloseHandle(out_write);
        }
        return Err(format!(
            "SetHandleInformation 失败（{}）",
            std::io::Error::last_os_error()
        ));
    }
    let (shell_app, inner_cmdline, cwd) = container_command(spec, command);
    let ports_csv = ports
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let extra = vec![
        (ENV_CMD.to_string(), inner_cmdline),
        (ENV_CWD.to_string(), cwd.to_string_lossy().into_owned()),
        (
            ENV_APP.to_string(),
            shell_app.to_string_lossy().into_owned(),
        ),
    ];
    let cmdline = format!(
        "\"{}\" {} {} {} {}",
        exe.display(),
        RELAY_FLAG,
        in_read as usize,
        out_write as usize,
        ports_csv
    );
    let spawned = spawn_in_container(sid, spec, command, Some(&exe), &cmdline, &cwd, &extra);
    // 子进程那两端在宿主侧立刻放掉：留着会让自己永远等不到 EOF。
    unsafe {
        CloseHandle(in_read);
        CloseHandle(out_write);
    }
    let pi = match spawned {
        Ok(pi) => pi,
        Err(e) => {
            unsafe {
                CloseHandle(in_write);
                CloseHandle(out_read);
            }
            return Err(e);
        }
    };
    let to_relay = unsafe { File::from_raw_handle(in_write as RawHandle) };
    let from_relay = unsafe { File::from_raw_handle(out_read as RawHandle) };
    pump(to_relay, from_relay, ports.to_vec());
    wait_and_reap(pi)
}

/// 目的：中继进程的入口（容器里跑）：解析宿主给的句柄与端口，起监听与真正的命令，命令退了自己也退。
pub(crate) fn relay_entry(args: &[String]) -> i32 {
    match relay_child(args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("[中继] 起不来：{}", e);
            FENCE_FAILED
        }
    }
}

/// 目的：中继进程的全部动作：关掉自己管道句柄的继承 → 绑端口 → 起真正的命令 → 双向转发 → 等命令退出。
fn relay_child(args: &[String]) -> Result<i32, String> {
    let read_end = parse_handle(args, 0, "宿主→中继的读端")?;
    let write_end = parse_handle(args, 1, "中继→宿主的写端")?;
    let ports = parse_ports(args.get(2))?;
    // 这两端不能再被真正的命令继承：命令拿到管道就等于能顶替中继、自己报端口。
    for h in [read_end, write_end] {
        if unsafe { SetHandleInformation(h, HANDLE_FLAG_INHERIT, 0) } == 0 {
            return Err(format!(
                "SetHandleInformation 失败（{}）",
                std::io::Error::last_os_error()
            ));
        }
    }
    let to_host = Arc::new(Mutex::new(unsafe {
        File::from_raw_handle(write_end as RawHandle)
    }));
    let mut from_host = unsafe { File::from_raw_handle(read_end as RawHandle) };
    // 先绑好端口再起命令：命令一启动就可能连过来。
    let mut listeners: Vec<(u16, TcpListener)> = Vec::new();
    for port in &ports {
        match TcpListener::bind((RELAY_ADDR, *port)) {
            Ok(l) => listeners.push((*port, l)),
            Err(e) => {
                return Err(format!(
                    "在容器里绑不上 {}:{}（{}）：这个地址端口已被占用",
                    RELAY_ADDR, port, e
                ));
            }
        }
    }
    let child = spawn_inner()?;
    let conns: Arc<Mutex<HashMap<u32, TcpStream>>> = Arc::new(Mutex::new(HashMap::new()));
    let next = Arc::new(AtomicU32::new(1));
    for (port, listener) in listeners {
        let to_host = to_host.clone();
        let conns = conns.clone();
        let next = next.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let stream = match stream {
                    Ok(s) => s,
                    Err(_) => break,
                };
                let conn = next.fetch_add(1, Ordering::Relaxed);
                let _ = stream.set_nodelay(true);
                let reader = match stream.try_clone() {
                    Ok(r) => r,
                    Err(_) => continue,
                };
                lock(&conns).insert(conn, stream);
                // 先报端口：宿主要在**校验通过之后**才会去 dial。
                if send(&to_host, conn, KIND_OPEN, &port.to_le_bytes()).is_err() {
                    break;
                }
                let to_host = to_host.clone();
                let conns = conns.clone();
                std::thread::spawn(move || {
                    let mut reader = reader;
                    let mut buf = [0u8; 16 * 1024];
                    loop {
                        match reader.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                if send(&to_host, conn, KIND_DATA, &buf[..n]).is_err() {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    lock(&conns).remove(&conn);
                    let _ = send(&to_host, conn, KIND_CLOSE, &[]);
                });
            }
        });
    }
    // 宿主回来的字节：按 conn 写回容器侧的 socket；CLOSE / REFUSE 就是收尾。
    std::thread::spawn(move || {
        while let Ok(Some((conn, kind, payload))) = read_frame(&mut from_host) {
            match kind {
                KIND_DATA => {
                    let mut map = lock(&conns);
                    if let Some(stream) = map.get_mut(&conn) {
                        let _ = stream.write_all(&payload);
                    }
                }
                KIND_CLOSE | KIND_REFUSE => {
                    if let Some(stream) = lock(&conns).remove(&conn) {
                        let _ = stream.shutdown(Shutdown::Both);
                    }
                }
                _ => {}
            }
        }
    });
    let mut code: u32 = FENCE_FAILED as u32;
    unsafe {
        WaitForSingleObject(child.hProcess, INFINITE);
        GetExitCodeProcess(child.hProcess, &mut code);
        CloseHandle(child.hThread);
        CloseHandle(child.hProcess);
    }
    Ok(code as i32)
}

/// 目的：宿主侧转发：一个线程按帧分发，每条连接一个线程把宿主 socket 的字节读回来。
fn pump(to_relay: File, mut from_relay: File, ports: Vec<u16>) {
    let writer = Arc::new(Mutex::new(to_relay));
    let conns: Arc<Mutex<HashMap<u32, TcpStream>>> = Arc::new(Mutex::new(HashMap::new()));
    let owner = std::process::id();
    std::thread::spawn(move || {
        while let Ok(Some((conn, kind, payload))) = read_frame(&mut from_relay) {
            match kind {
                KIND_OPEN => open_conn(conn, &payload, &ports, owner, &writer, &conns),
                KIND_DATA => {
                    let mut map = lock(&conns);
                    if let Some(stream) = map.get_mut(&conn) {
                        let _ = stream.write_all(&payload);
                    }
                }
                KIND_CLOSE | KIND_REFUSE => {
                    if let Some(stream) = lock(&conns).remove(&conn) {
                        let _ = stream.shutdown(Shutdown::Both);
                    }
                }
                _ => {}
            }
        }
    });
}

/// 目的：容器侧报来一个端口——**在这里**校验（白名单 + 不是本产品自己的端口）并 dial 宿主的同名端口。
/// 约束：校验不通过一律回 `REFUSE`：容器侧只会看到连接被关。
fn open_conn(
    conn: u32,
    payload: &[u8],
    ports: &[u16],
    owner: u32,
    writer: &Arc<Mutex<File>>,
    conns: &Arc<Mutex<HashMap<u32, TcpStream>>>,
) {
    let port = if payload.len() == 2 {
        u16::from_le_bytes([payload[0], payload[1]])
    } else {
        0
    };
    // 不在清单里、或者这个端口是本产品自己在听：一律拒。容器侧只会看到连接被关，
    // 不告诉它"这个端口存在但你不许连"——那本身就是信息。
    if port == 0 || !ports.contains(&port) || served_by_product(port, owner) {
        let _ = send(writer, conn, KIND_REFUSE, &[]);
        return;
    }
    let stream = match crate::kernel::detail::io::net::tcp::dial(port) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[中继] 宿主 127.0.0.1:{} 连不上（{}）", port, e);
            let _ = send(writer, conn, KIND_REFUSE, &[]);
            return;
        }
    };
    let _ = stream.set_nodelay(true);
    let reader = match stream.try_clone() {
        Ok(r) => r,
        Err(_) => {
            let _ = send(writer, conn, KIND_REFUSE, &[]);
            return;
        }
    };
    lock(conns).insert(conn, stream);
    let writer = writer.clone();
    let conns = conns.clone();
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 16 * 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if send(&writer, conn, KIND_DATA, &buf[..n]).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        lock(&conns).remove(&conn);
        let _ = send(&writer, conn, KIND_CLOSE, &[]);
    });
}

/// 目的：这个端口是不是**本产品自己**在听（转录中心、guest 通道、共享适配器都在内）：是就一律拒。
/// 约束：判定靠「监听者的 pid 在本产品进程树里，或它跑的就是本产品这个可执行文件」，不靠端口表——端口会漂移，表会漏。
fn served_by_product(port: u16, owner: u32) -> bool {
    listening_pids(port)
        .into_iter()
        .any(|pid| in_process_tree(pid, owner) || same_executable(pid))
}

/// 目的：`pid` 跑的是不是本产品这个可执行文件——盖住「用户另起了一个产品实例」：它不在本进程树里，控制面一样不该被摸到。
/// 约束：读不到镜像路径（受保护或已退出）就当不是——判定失败只能更宽松，不能把用户自己的服务拒掉。
fn same_executable(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    let Ok(mine) = env::current_exe() else {
        return false;
    };
    let Some(theirs) = process_image(pid) else {
        return false;
    };
    mine.to_string_lossy()
        .eq_ignore_ascii_case(&theirs.to_string_lossy())
}

/// 目的：某个进程的镜像完整路径；读不到（权限不足 / 已退出）返回 `None`。
fn process_image(pid: u32) -> Option<std::path::PathBuf> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len);
        CloseHandle(handle);
        if ok == 0 || len == 0 {
            return None;
        }
        Some(std::path::PathBuf::from(String::from_utf16_lossy(
            &buf[..len as usize],
        )))
    }
}

/// 目的：本机（IPv4）**从回环可达**地监听这个端口的进程号；查不到就当作没人听。
/// 约束：必须按地址筛，不能只看端口——中继自己在容器里绑的是 127.0.0.2:<端口>，
///   它与宿主 127.0.0.1:<端口> 是同一张 TCP 表、同一个端口号（容器与宿主共用一个回环栈），
///   不筛地址就会把中继自己认成「产品在听」，把每一条合法连接都拒掉。
fn listening_pids(port: u16) -> Vec<u32> {
    // AF_INET：这里只要 IPv4 回环，产品没有引入 WinSock 那一组 feature。
    const AF_INET: u32 = 2;
    // 表里的地址是按网络序装的字节，按本机字节序读回来就是下面这两个值。
    // 0.0.0.0 也算：绑全地址的服务照样能从 127.0.0.1 摸到。
    const LOOPBACK: u32 = u32::from_ne_bytes([127, 0, 0, 1]);
    const ANY: u32 = 0;
    let mut size: u32 = 0;
    unsafe {
        GetExtendedTcpTable(
            std::ptr::null_mut(),
            &mut size,
            0,
            AF_INET,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        );
    }
    if size == 0 {
        return Vec::new();
    }
    let mut buf = vec![0u8; size as usize];
    let rc = unsafe {
        GetExtendedTcpTable(
            buf.as_mut_ptr() as *mut c_void,
            &mut size,
            0,
            AF_INET,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        )
    };
    // 非 0 = 两次调用之间表变大了（ERROR_INSUFFICIENT_BUFFER）：当作没查到，下一次连接还会再问。
    if rc != 0 {
        return Vec::new();
    }
    let count = unsafe { *(buf.as_ptr() as *const u32) } as usize;
    let rows = unsafe {
        std::slice::from_raw_parts(
            buf.as_ptr().add(4) as *const MIB_TCPROW_OWNER_PID,
            count.min(buf.len() / std::mem::size_of::<MIB_TCPROW_OWNER_PID>()),
        )
    };
    rows.iter()
        .filter(|row| row.dwLocalPort as u16 == port.to_be())
        .filter(|row| row.dwLocalAddr == LOOPBACK || row.dwLocalAddr == ANY)
        .map(|row| row.dwOwningPid)
        .collect()
}

/// 目的：`pid` 是不是 `owner` 自己或它的后代（沿父进程链走，最多 64 层防环）。
fn in_process_tree(pid: u32, owner: u32) -> bool {
    let parents = parent_map();
    let mut cur = pid;
    for _ in 0..64 {
        if cur == owner {
            return true;
        }
        match parents.get(&cur).copied() {
            Some(parent) if parent != 0 && parent != cur => cur = parent,
            _ => return false,
        }
    }
    false
}

/// 目的：本机进程表里的「子 → 父」映射（查不到就返回空表，按「不是本产品」处理）。
fn parent_map() -> HashMap<u32, u32> {
    let mut map = HashMap::new();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot.is_null() {
            return map;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snapshot, &mut entry);
        while ok != 0 {
            map.insert(entry.th32ProcessID, entry.th32ParentProcessID);
            ok = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
    }
    map
}

/// 目的：把真正的命令起在容器里（子进程原样继承容器令牌——它仍在围栏内，只 stdio 继承中继的）。
fn spawn_inner() -> Result<PROCESS_INFORMATION, String> {
    let app = env::var(ENV_APP).ok_or_else(|| format!("缺环境变量 {}", ENV_APP))?;
    let cmdline = env::var(ENV_CMD).ok_or_else(|| format!("缺环境变量 {}", ENV_CMD))?;
    let cwd = env::var(ENV_CWD).ok_or_else(|| format!("缺环境变量 {}", ENV_CWD))?;
    let mut line: Vec<u16> = std::ffi::OsStr::new(&cmdline)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let app_wide = to_wide(Path::new(&app));
    let cwd_wide = to_wide(Path::new(&cwd));
    let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
    si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    si.dwFlags = STARTF_USESTDHANDLES;
    unsafe {
        si.hStdInput = GetStdHandle(STD_INPUT_HANDLE);
        si.hStdOutput = GetStdHandle(STD_OUTPUT_HANDLE);
        si.hStdError = GetStdHandle(STD_ERROR_HANDLE);
    }
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // 环境块给 null：命令原样继承中继的环境（它就是围栏的白名单，不必再拼一遍）。
    let created = unsafe {
        CreateProcessW(
            app_wide.as_ptr(),
            line.as_mut_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            1,
            0,
            std::ptr::null(),
            cwd_wide.as_ptr(),
            &si,
            &mut pi,
        )
    };
    if created == 0 {
        return Err(format!(
            "CreateProcessW 失败（{}）：{}",
            std::io::Error::last_os_error(),
            cmdline
        ));
    }
    Ok(pi)
}

fn to_wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// 目的：一对匿名管道（两端都可继承；调用方自己决定哪端不给子进程）。
fn pipe() -> Result<(HANDLE, HANDLE), String> {
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    let mut read: HANDLE = std::ptr::null_mut();
    let mut write: HANDLE = std::ptr::null_mut();
    if unsafe { CreatePipe(&mut read, &mut write, &sa, 0) } == 0 {
        return Err(format!(
            "CreatePipe 失败（{}）",
            std::io::Error::last_os_error()
        ));
    }
    Ok((read, write))
}

/// 目的：发一帧（头与载荷在同一把锁里写完，两条连接不会互相切字节）。
fn send(writer: &Arc<Mutex<File>>, conn: u32, kind: u8, payload: &[u8]) -> std::io::Result<()> {
    let mut head = [0u8; HEAD_LEN];
    head[0..4].copy_from_slice(&conn.to_le_bytes());
    head[4] = kind;
    head[5..HEAD_LEN].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    let mut file = lock(writer);
    file.write_all(&head)?;
    file.write_all(payload)?;
    file.flush()
}

/// 目的：收一帧；对端关掉就给 `None`（帧错位与超长都按收尾处理，不猜）。
fn read_frame(file: &mut File) -> std::io::Result<Option<(u32, u8, Vec<u8>)>> {
    let mut head = [0u8; HEAD_LEN];
    if !read_full(file, &mut head)? {
        return Ok(None);
    }
    let conn = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
    let kind = head[4];
    let len = u32::from_le_bytes([head[5], head[6], head[7], head[8]]) as usize;
    if len > MAX_FRAME {
        return Ok(None);
    }
    let mut payload = vec![0u8; len];
    if len > 0 && !read_full(file, &mut payload)? {
        return Ok(None);
    }
    Ok(Some((conn, kind, payload)))
}

fn read_full(file: &mut File, buf: &mut [u8]) -> std::io::Result<bool> {
    let mut off = 0;
    while off < buf.len() {
        match file.read(&mut buf[off..]) {
            Ok(0) => return Ok(false),
            Ok(n) => off += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(true)
}

/// 目的：拿锁（被 poison 也继续：这里没有「锁里的状态不能再用」的情形）。
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn parse_handle(args: &[String], index: usize, what: &str) -> Result<HANDLE, String> {
    let text = args
        .get(index)
        .ok_or_else(|| format!("缺 {}（内部协议，参数由宿主给）", what))?;
    let value: usize = text
        .parse()
        .map_err(|_| format!("{} 不是句柄号：{}", what, text))?;
    if value == 0 {
        return Err(format!("{} 是空句柄", what));
    }
    Ok(value as HANDLE)
}

fn parse_ports(text: Option<&String>) -> Result<Vec<u16>, String> {
    let text = text.ok_or_else(|| "缺端口清单（内部协议，参数由宿主给）".to_string())?;
    let mut ports: Vec<u16> = Vec::new();
    for one in text.split(',') {
        let port: u16 = one
            .trim()
            .parse()
            .map_err(|_| format!("端口清单里有非法端口：{}", one))?;
        if port != 0 {
            ports.push(port);
        }
    }
    Ok(ports)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 目的：**产品端口一律拒**的判据本体：查本机监听者是哪个进程、再沿父进程链认归属，
    ///   不维护静态黑名单（端口会漂移：转录中心 3081 默认、QGA 随机、SMB 助手 45678）。
    ///   约束：这条只在真机上说得清（要读系统 TCP 表），所以断言的是"自己进程在听的端口算产品的"。
    #[test]
    fn a_port_listened_by_this_process_belongs_to_the_product() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("占一个回环端口");
        let port = listener.local_addr().expect("取端口").port();
        assert!(
            served_by_product(port, std::process::id()),
            "自己进程在听的端口要认成产品的，否则产品端口会被放行"
        );
        drop(listener);
        assert!(
            !served_by_product(port, std::process::id()),
            "没人听的端口不该算产品的"
        );
    }

    /// 目的：盖住「用户另起了一个产品实例」——它不在本进程树里，但跑的是同一个可执行文件，控制面一样不该被摸到。
    #[test]
    fn the_product_executable_counts_as_the_product_wherever_it_runs() {
        assert!(
            same_executable(std::process::id()),
            "本进程自己跑的就是产品这个可执行文件"
        );
    }

    /// 目的：**回归**——中继自己在容器里绑 127.0.0.2:<端口>，它与宿主 127.0.0.1:<端口>
    ///   在 TCP 表里同端口号；只按端口判就会把中继自己认成「产品在听」，每条合法连接都被拒。
    #[test]
    fn a_relay_listener_on_the_other_loopback_address_is_not_a_product_port() {
        let relay = match std::net::TcpListener::bind((RELAY_ADDR, 0)) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[单元] 本机绑不上 {}，跳过：{}", RELAY_ADDR, e);
                return;
            }
        };
        let port = relay.local_addr().expect("取端口").port();
        assert!(
            !served_by_product(port, std::process::id()),
            "中继自己那条 127.0.0.2 监听不能算成产品端口，否则合法连接全被拒"
        );
    }

    /// 目的：中继命令行里的端口清单只收合法端口；`0` 静默跳过（宿主侧还会再按清单校验一次）。
    #[test]
    fn the_port_list_takes_legal_ports_only() {
        let good = "997,8080".to_string();
        assert_eq!(parse_ports(Some(&good)).expect("合法"), vec![997, 8080]);
        assert_eq!(
            parse_ports(Some(&"0".to_string())).expect("0 跳过"),
            Vec::<u16>::new()
        );
        assert!(parse_ports(Some(&"http".to_string())).is_err());
        assert!(parse_ports(Some(&"70000".to_string())).is_err());
        assert!(parse_ports(None).is_err());
    }
}
