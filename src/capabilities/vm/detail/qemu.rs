//! 目的：VmEngine / GuestChannel 的 QEMU 实现——拉起 qemu-system-*，经 QGA 执行与销毁。
//! 管：QEMU 进程的启停、QEMU 基础参数（含 rootfs 的 9p 根共享与 QGA 串口）、JSON 收发、命令回执与 base64 数据。
//! 不管：共享后端的额外参数（detail/share_ninep.rs）、镜像判定与挂载清单（domain/guest.rs）。
//! 联动：端口在 ports.rs；由组合根构造、service.rs 持有。

use crate::capabilities::vm::domain::guest::{
    GuestArch, GuestImage, GuestRoot, GuestRun, GuestSpec,
};
use crate::capabilities::vm::ports::{GuestChannel, GuestVm, SharePlan, VmEngine};
use crate::kernel::api::ToolOutcome;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// 目的：QEMU 的加速器有序表（按平台）：本平台的硬件加速排前，TCG 兜底。
/// 约束：QEMU 会把本平台不认识的加速器记为 invalid 并继续试下一项，所以同一份参数可跨平台复用。
pub fn accelerators() -> &'static str {
    if cfg!(windows) {
        "whpx:tcg"
    } else if cfg!(target_os = "macos") {
        "hvf:tcg"
    } else if cfg!(target_os = "linux") {
        "kvm:tcg"
    } else {
        "tcg"
    }
}

/// 目的：QEMU 可执行文件名（架构不同名字不同）。
pub fn qemu_exe(arch: GuestArch) -> &'static str {
    match arch {
        GuestArch::Amd64 => "qemu-system-x86_64",
        GuestArch::Arm64 => "qemu-system-aarch64",
    }
}

/// 目的：QEMU 引擎——每次 boot 拉起一个 qemu-system-* 进程并接上 QGA 通道。
pub struct QemuEngine;

impl QemuEngine {
    /// 目的：组合根专用。
    pub fn new() -> QemuEngine {
        QemuEngine
    }
}

/// 目的：除共享之外的一次启动参数（不含可执行文件本身）——机器、加速器、内核、串口与 QGA 通道。
/// 参数：channel_opts 是 QGA chardev 的补充选项；share_needs_net = 共享后端要不要 guest 有（受限）网卡。
/// 约束：rootfs 经 virtio-9p 的 base 标签挂成根——当前的根仍是目录；磁盘镜像见 docs/vm/README.md 的「多平台组合」。
pub fn engine_args(spec: &GuestSpec, channel_opts: &str, share_needs_net: bool) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    for v in [
        "-m",
        "2048",
        "-smp",
        "2",
        "-machine",
        accelerators(),
        "-display",
        "none",
        "-no-reboot",
        "-nodefaults",
        // -nodefaults 连默认串口一起去掉；console=ttyS0 要有对应设备，所以显式给一个。
        "-serial",
        "stdio",
    ] {
        a.push(v.to_string());
    }
    // 有内核就直启（-kernel / -initrd / -append）；没有内核的磁盘根靠镜像自带的引导（固件启动）。
    if let Some(kernel) = &spec.image.kernel {
        a.push("-kernel".to_string());
        a.push(kernel.display().to_string());
        if let Some(initrd) = &spec.image.initrd {
            a.push("-initrd".to_string());
            a.push(initrd.display().to_string());
        }
    }
    match &spec.image.root {
        GuestRoot::Dir(dir) => {
            a.push("-fsdev".to_string());
            a.push(format!(
                // 目录根共享**可写**：内核按 root=... rw 挂它，额外 9p 挂载点也要在 guest 根里建得出来。
                "local,id=fsbase,path={},security_model=none",
                dir.display()
            ));
            a.push("-device".to_string());
            a.push("virtio-9p-pci,fsdev=fsbase,mount_tag=base".to_string());
            a.push("-append".to_string());
            a.push(
                "console=ttyS0 root=base rootfstype=9p rootflags=trans=virtio,version=9p2000.L rw"
                    .to_string(),
            );
        }
        GuestRoot::Disk(image) => {
            a.push("-drive".to_string());
            a.push(format!("file={},if=virtio", image.display()));
            if spec.image.kernel.is_some() {
                a.push("-append".to_string());
                a.push("console=ttyS0 root=/dev/vda rw".to_string());
            }
        }
    }
    a.push("-chardev".to_string());
    a.push(format!("socket,id=ch0,{}", channel_opts));
    a.push("-device".to_string());
    a.push("virtio-serial-pci".to_string());
    a.push("-device".to_string());
    a.push("virtserialport,chardev=ch0,name=org.qemu.guest_agent.0".to_string());
    // 网卡三种情形：放行出站 = 普通 user 网卡；共享要隧道 = restrict=on 的受限网卡（出站全断、
    // guestfwd/hostfwd 内部隧道可用）；两者都不是 = 干脆不给网卡。
    if spec.net || share_needs_net {
        let restrict = if spec.net { "" } else { ",restrict=on" };
        a.push("-netdev".to_string());
        a.push(format!("user,id=n0{}", restrict));
        a.push("-device".to_string());
        a.push("virtio-net-pci,netdev=n0".to_string());
    } else {
        a.push("-nic".to_string());
        a.push("none".to_string());
    }
    a
}

impl VmEngine for QemuEngine {
    fn id(&self) -> &'static str {
        "qemu"
    }

    /// 约束：架构要匹配；目录根靠 9p，只有 Linux / macOS 宿主的 QEMU 有 virtfs；磁盘根三平台通用。
    fn supports(&self, guest: &GuestImage) -> bool {
        if guest.arch != GuestArch::host() {
            return false;
        }
        match &guest.root {
            GuestRoot::Dir(_) => cfg!(target_os = "linux") || cfg!(target_os = "macos"),
            GuestRoot::Disk(_) => true,
        }
    }

    fn boot(&self, spec: &GuestSpec, share: &SharePlan) -> Result<Box<dyn GuestVm>, String> {
        let listener =
            TcpListener::bind("127.0.0.1:0").map_err(|e| format!("占不到 QGA 端口：{}", e))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let opts = format!("host=127.0.0.1,port={},server=on,wait=off", port);
        let exe = spec
            .engine_path
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| qemu_exe(spec.image.arch).to_string());
        let mut args = engine_args(spec, &opts, share.needs_net);
        args.extend(share.engine_args.iter().cloned());
        let child = Command::new(&exe)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("起不了 QEMU（{}）：{}", exe, e))?;
        drop(listener);
        let stream = connect_within(port, Duration::from_secs(30))?;
        let writer = stream.try_clone().map_err(|e| e.to_string())?;
        let mut handle = QemuHandle {
            child: Some(child),
            reader: BufReader::new(stream),
            writer,
        };
        // QEMU 只把额外共享接进来，不替 guest 挂：接上 QGA 后按共享计划逐个挂到 guest 路径。
        for script in &share.guest_scripts {
            let out = handle.exec(&GuestRun {
                command: script.clone(),
                args_json: "{}".to_string(),
                env: Vec::new(),
                cwd: String::new(),
            });
            if !out.ok {
                handle.stop();
                return Err(format!("把共享挂进 guest 失败：{}", out.output));
            }
        }
        Ok(Box::new(handle))
    }
}

fn connect_within(port: u16, budget: Duration) -> Result<TcpStream, String> {
    let deadline = Instant::now() + budget;
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(s) => return Ok(s),
            Err(e) => {
                if Instant::now() >= deadline {
                    return Err(format!("等 QGA 通道超时（端口 {}）：{}", port, e));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

struct QemuHandle {
    child: Option<Child>,
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl QemuHandle {
    fn send(&mut self, v: &serde_json::Value) -> Result<(), String> {
        let mut line = v.to_string();
        line.push('\n');
        self.writer
            .write_all(line.as_bytes())
            .map_err(|e| e.to_string())
    }

    fn recv(&mut self) -> Result<serde_json::Value, String> {
        let mut line = String::new();
        let n = self
            .reader
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("QGA 通道断了".to_string());
        }
        serde_json::from_str(line.trim()).map_err(|e| format!("QGA 回包不是 JSON：{}", e))
    }

    fn sync(&mut self) -> Result<(), String> {
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1);
        self.send(&serde_json::json!({"execute":"guest-sync","arguments":{"id":id}}))?;
        for _ in 0..64 {
            let v = self.recv()?;
            if v.get("return").and_then(|r| r.as_u64()) == Some(id) {
                return Ok(());
            }
        }
        Err("QGA 同步不上".to_string())
    }

    /// 目的：打开 guest 里的一个文件，拿回 QGA 句柄。
    fn file_open(&mut self, path: &str, mode: &str) -> Result<i64, String> {
        self.send(&serde_json::json!({
            "execute":"guest-file-open",
            "arguments":{"path":path,"mode":mode}
        }))?;
        let v = self.recv()?;
        v.get("return")
            .and_then(|r| r.as_i64())
            .ok_or_else(|| "QGA 没给出文件句柄".to_string())
    }

    /// 目的：关掉 guest 文件句柄（回包不关心）。
    fn file_close(&mut self, handle: i64) {
        let _ = self.send(&serde_json::json!({
            "execute":"guest-file-close",
            "arguments":{"handle":handle}
        }));
        let _ = self.recv();
    }
}

impl GuestChannel for QemuHandle {
    fn exec(&mut self, run: &GuestRun) -> ToolOutcome {
        if let Err(why) = self.sync() {
            return fail(&why);
        }
        let env: Vec<serde_json::Value> = run
            .env
            .iter()
            .map(|(k, v)| serde_json::json!({"name":k,"value":v}))
            .collect();
        let script = if run.cwd.is_empty() {
            run.command.clone()
        } else {
            format!("cd {} && {}", run.cwd, run.command)
        };
        if let Err(why) = self.send(&serde_json::json!({
            "execute":"guest-exec",
            "arguments":{
                "path":"/bin/sh",
                "arg":["-c",script],
                "input-data": b64(&run.args_json),
                "capture-output":true,
                "env":env,
            }
        })) {
            return fail(&why);
        }
        let pid = match self.recv() {
            Ok(v) => match v
                .get("return")
                .and_then(|r| r.get("pid"))
                .and_then(|p| p.as_i64())
            {
                Some(p) => p,
                None => return fail("QGA 没给出进程号"),
            },
            Err(why) => return fail(&why),
        };
        let deadline = Instant::now() + Duration::from_secs(600);
        loop {
            if let Err(why) = self.send(&serde_json::json!({
                "execute":"guest-exec-status","arguments":{"pid":pid}
            })) {
                return fail(&why);
            }
            let v = match self.recv() {
                Ok(v) => v,
                Err(why) => return fail(&why),
            };
            let Some(ret) = v.get("return") else {
                return fail("QGA 回包里没有 return");
            };
            if ret.get("exited").and_then(|b| b.as_bool()) == Some(true) {
                let code = ret.get("exitcode").and_then(|c| c.as_i64()).unwrap_or(-1);
                let out = ret
                    .get("out-data")
                    .and_then(|d| d.as_str())
                    .map(unb64)
                    .unwrap_or_default();
                let err = ret
                    .get("err-data")
                    .and_then(|d| d.as_str())
                    .map(unb64)
                    .unwrap_or_default();
                let output = format!(
                    "{}{}",
                    String::from_utf8_lossy(&out),
                    String::from_utf8_lossy(&err)
                );
                return ToolOutcome {
                    ok: code == 0,
                    output,
                };
            }
            if Instant::now() >= deadline {
                return fail("等 guest 里的命令超时");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// 目的：把字节分块写进 guest 文件（QGA guest-file-write，base64）。
    fn put_file(&mut self, path: &str, bytes: &[u8]) -> Result<(), String> {
        self.sync()?;
        let handle = self.file_open(path, "wb")?;
        let mut off = 0;
        while off < bytes.len() {
            let end = (off + 3072).min(bytes.len());
            let encoded = b64_bytes(&bytes[off..end]);
            if let Err(why) = self.send(&serde_json::json!({
                "execute":"guest-file-write",
                "arguments":{"handle":handle,"buf-b64":encoded}
            })) {
                self.file_close(handle);
                return Err(why);
            }
            let v = match self.recv() {
                Ok(v) => v,
                Err(why) => {
                    self.file_close(handle);
                    return Err(why);
                }
            };
            let n = v
                .get("return")
                .and_then(|r| r.get("count"))
                .and_then(|c| c.as_i64())
                .unwrap_or(0);
            if n <= 0 {
                self.file_close(handle);
                return Err("QGA 写 guest 文件返回 0 字节".to_string());
            }
            off += n as usize;
        }
        self.file_close(handle);
        Ok(())
    }

    /// 目的：把 guest 文件按块读回来（QGA guest-file-read，base64）。
    fn get_file(&mut self, path: &str) -> Result<Vec<u8>, String> {
        self.sync()?;
        let handle = self.file_open(path, "rb")?;
        let mut out: Vec<u8> = Vec::new();
        loop {
            if let Err(why) = self.send(&serde_json::json!({
                "execute":"guest-file-read",
                "arguments":{"handle":handle,"count":65536}
            })) {
                self.file_close(handle);
                return Err(why);
            }
            let v = match self.recv() {
                Ok(v) => v,
                Err(why) => {
                    self.file_close(handle);
                    return Err(why);
                }
            };
            let ret = match v.get("return") {
                Some(r) => r,
                None => {
                    self.file_close(handle);
                    return Err("QGA 读文件回包里没有 return".to_string());
                }
            };
            let n = ret.get("count").and_then(|c| c.as_i64()).unwrap_or(0);
            if n <= 0 {
                break;
            }
            if let Some(s) = ret.get("buf-b64").and_then(|d| d.as_str()) {
                out.extend(unb64(s));
            }
            if n < 65536 {
                break;
            }
        }
        self.file_close(handle);
        Ok(out)
    }
}

impl GuestVm for QemuHandle {
    fn channel(&mut self) -> &mut dyn GuestChannel {
        self
    }

    fn stop(&mut self) {
        let _ = self.send(&serde_json::json!({"execute":"guest-shutdown"}));
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn fail(why: &str) -> ToolOutcome {
    ToolOutcome {
        ok: false,
        output: why.to_string(),
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64(s: &str) -> String {
    b64_bytes(s.as_bytes())
}

fn b64_bytes(data: &[u8]) -> String {
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
        out.push(B64[((n >> 18) & 63) as usize] as char);
        out.push(B64[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            B64[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn unb64(s: &str) -> Vec<u8> {
    let mut vals: Vec<u8> = Vec::new();
    for c in s.bytes() {
        if c == b'=' {
            break;
        }
        if let Some(i) = B64.iter().position(|x| *x == c) {
            vals.push(i as u8);
        }
    }
    let mut out = Vec::new();
    for chunk in vals.chunks(4) {
        let mut n: u32 = 0;
        for (i, v) in chunk.iter().enumerate() {
            n |= (*v as u32) << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    out
}
