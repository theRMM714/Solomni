//! 目的：VmEngine / GuestChannel 的 QEMU 实现——拉起 qemu-system-*，经 QGA 执行与销毁。
//! 管：QEMU 进程的启停、QEMU 基础参数（含 rootfs 的 9p 根共享与 QGA 串口）、JSON 收发、命令回执与 base64 数据。
//! 不管：共享后端的额外参数（detail/share_ninep.rs）、镜像判定与挂载清单（domain/guest.rs）。
//! 联动：端口在 ports.rs；由组合根构造、service.rs 持有。

use crate::capabilities::vm::domain::guest::{
    GuestArch, GuestImage, GuestRoot, GuestRun, GuestSpec, SeedDocs,
};
use crate::capabilities::vm::ports::{GuestChannel, GuestVm, SharePlan, VmEngine};
use crate::kernel::api::ToolOutcome;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// 目的：QEMU 的加速器有序表（按平台）：本平台的硬件加速排前，TCG 兜底；值是 -machine 的 accel=… 参数。
/// 约束：QEMU 会把本平台不认识的加速器记为 invalid 并继续试下一项，所以同一份参数可跨平台复用。
pub fn accelerators() -> &'static str {
    if cfg!(windows) {
        "accel=whpx:tcg"
    } else if cfg!(target_os = "macos") {
        "accel=hvf:tcg"
    } else if cfg!(target_os = "linux") {
        "accel=kvm:tcg"
    } else {
        "accel=tcg"
    }
}

/// 目的：QEMU 可执行文件名（架构不同名字不同）。
pub fn qemu_exe(arch: GuestArch) -> &'static str {
    match arch {
        GuestArch::Amd64 => "qemu-system-x86_64",
        GuestArch::Arm64 => "qemu-system-aarch64",
    }
}

/// 目的：QEMU 引擎——每次 boot 拉起一个 qemu-system-* 进程并接上 QGA 通道；
///   要每租约可写层时先用同目录的 qemu-img 在基础镜像上建一份 qcow2 覆盖（多台并发不能同写一份基础镜像）。
pub struct QemuEngine {
    /// 每租约可写层的落点（组合根注入；None = 不提供可写层——供给编排改的正是基础镜像本身）。
    overlay_dir: Option<PathBuf>,
}

impl QemuEngine {
    /// 目的：组合根专用（不带可写层：供给编排与测试用）。
    pub fn new() -> QemuEngine {
        QemuEngine { overlay_dir: None }
    }

    /// 目的：组合根专用（带每租约可写层的落点：会话的 guest 盘写在这里，删会话即丢）。
    pub fn with_overlay_dir(dir: PathBuf) -> QemuEngine {
        QemuEngine {
            overlay_dir: Some(dir),
        }
    }
}

/// 目的：guest 内存大小（MiB）——`-m` 与共享内存后端（vhost-user 要）必须同一尺寸，所以只写一处。
pub const GUEST_MEM_MB: u32 = 2048;

/// 目的：除共享之外的一次启动参数（不含可执行文件本身）——机器、加速器、内存、内核、串口、QGA 通道与首启种子。
/// 参数：channel_opts 是 QGA chardev 的补充选项；share_needs_net = 共享后端要不要 guest 有（受限）网卡；
///   share_needs_mem = 共享后端要不要 guest RAM 是**可共享内存**（vhost-user 设备，如 virtiofs）；
///   seed_url = 首启 cloud-init 的 NoCloud net 种子根 URL（None = 不投种子）。
/// 约束：rootfs 经 virtio-9p 的 base 标签挂成根——当前的根仍是目录；磁盘镜像见 docs/vm/README.md 的「多平台组合」。
pub fn engine_args(
    spec: &GuestSpec,
    channel_opts: &str,
    share_needs_net: bool,
    share_needs_mem: bool,
    seed_url: Option<&str>,
    net_options: &[String],
) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    a.push("-m".to_string());
    a.push(GUEST_MEM_MB.to_string());
    // vhost-user 设备（virtiofs）要求 guest RAM 是可共享内存：给一个同尺寸的 memfd 后端，挂到唯一 numa 节点。
    if share_needs_mem {
        a.push("-object".to_string());
        a.push(format!(
            "memory-backend-memfd,id=solomni-mem,size={}M,share=on",
            GUEST_MEM_MB
        ));
        a.push("-numa".to_string());
        a.push("node,memdev=solomni-mem".to_string());
    }
    for v in [
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
                // `none`：guest 的 chown 会被顶回 EPERM——所以目录根上**不要**走 dpkg 装包（见 provision 的 node 路径）。
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
    // 首启种子：SMBIOS type 1 的 serial 是 cloud-init NoCloud 认识的数据源指路（ds=nocloud-net;s=<URL>）。
    if let Some(url) = seed_url {
        a.push("-smbios".to_string());
        a.push(format!("type=1,serial=ds=nocloud-net;s={}", url));
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
        let extra = if net_options.is_empty() {
            String::new()
        } else {
            format!(",{}", net_options.join(","))
        };
        a.push("-netdev".to_string());
        a.push(format!("user,id=n0{}{}", restrict, extra));
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
        // 每租约可写层：会话不直写基础镜像（多台并发会同写一份）；供给编排要改基础镜像本身，isolated = false。
        let overlay = match (&spec.image.root, spec.isolated) {
            (GuestRoot::Disk(base), true) => {
                let dir = self
                    .overlay_dir
                    .as_ref()
                    .ok_or("这次启动要每租约可写层，但引擎没配可写层目录".to_string())?;
                std::fs::create_dir_all(dir)
                    .map_err(|e| format!("建可写层目录失败（{}）：{}", dir.display(), e))?;
                let path = dir.join(format!("{}.qcow2", lease_file_name(&spec.lease)));
                let _ = std::fs::remove_file(&path);
                create_overlay(&exe, base, &path)?;
                Some(path)
            }
            _ => None,
        };
        // 有效启动规格：有可写层就用它当根，基础镜像退成只读后备（内核 / initrd 仍取自基础根）。
        let mut effective = spec.clone();
        effective.image = with_overlay(&spec.image, overlay.as_deref());
        // 首启种子：宿主起一个小 HTTP 服务，经 -smbios 告诉 cloud-init 去哪取 user-data / meta-data。
        let seed = match &spec.seed {
            Some(docs) => {
                if !spec.net {
                    return Err("投 cloud-init 种子要求 guest 有网卡（net = true）".to_string());
                }
                Some(SeedServer::start(docs)?)
            }
            None => None,
        };
        let seed_url = seed.as_ref().map(SeedServer::url);
        let mut args = engine_args(
            &effective,
            &opts,
            share.needs_net,
            share.needs_shared_mem,
            seed_url.as_deref(),
            &share.net_options,
        );
        // 便携包把固件放在可执行文件旁（`bin/` 的兄弟 `share/qemu`）：显式 `-L` 指过去，
        // 否则 QEMU 只按编译期前缀找，便携布局下会报 "could not load PC BIOS"。
        if let Some(fw) = firmware_dir(&exe) {
            args.insert(0, fw.display().to_string());
            args.insert(0, "-L".to_string());
        }
        args.extend(share.engine_args.iter().cloned());
        let child = Command::new(&exe)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            // stderr 继承：QEMU 起不来时（固件缺失这类）错误要能看见，不能吞进 null。
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("起不了 QEMU（{}）：{}", exe, e))?;
        drop(listener);
        let stream = match connect_within(port, Duration::from_secs(30)) {
            Ok(s) => s,
            Err(e) => {
                // 连不上就把刚拉起的 QEMU 与刚建的可写层都收掉，别留孤儿。
                let mut c = child;
                let _ = c.kill();
                let _ = c.wait();
                if let Some(path) = &overlay {
                    let _ = std::fs::remove_file(path);
                }
                return Err(e);
            }
        };
        // 读超时：agent 还没起时不能让 read_line 干等——首启轮询与运行期失败都要有界。
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .map_err(|e| format!("给 QGA 通道设读超时失败：{}", e))?;
        let writer = stream.try_clone().map_err(|e| e.to_string())?;
        let mut handle = QemuHandle {
            child: Some(child),
            reader: BufReader::new(stream),
            writer,
            port,
            seed,
            overlay,
        };
        // agent 要等内核起来才通；投种子时（首启装 agent）不等，交给编排轮询，其余情况先等到就绪再挂载/回命令。
        if spec.seed.is_none() {
            let deadline = Instant::now() + Duration::from_secs(300);
            loop {
                if handle.ping() {
                    break;
                }
                if Instant::now() >= deadline {
                    handle.stop();
                    return Err("等 guest 里的 qemu-guest-agent 超时".to_string());
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        }
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

/// 目的：便携 QEMU 的固件目录——可执行文件的 `bin/` 的兄弟 `share/qemu`（存在才算）。
fn firmware_dir(exe: &str) -> Option<PathBuf> {
    let bin = Path::new(exe).parent()?;
    let dir = bin.parent()?.join("share").join("qemu");
    if dir.is_dir() {
        Some(dir)
    } else {
        None
    }
}

/// 目的：可写层文件名——租约名可能含路径分隔符或怪字符，这里只留字母数字与 `-_.`。
fn lease_file_name(lease: &str) -> String {
    lease
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// 目的：把镜像的根换成可写层（None = 原样）；有效启动规格由它派生。
fn with_overlay(image: &GuestImage, overlay: Option<&Path>) -> GuestImage {
    let mut out = image.clone();
    if let Some(path) = overlay {
        out.root = GuestRoot::Disk(path.to_path_buf());
    }
    out
}

/// 目的：基础镜像的格式——qemu-img 建可写层要显式给 `-F`，否则 raw（ext4 这类）会被按扩展名猜错。
fn backing_format(base: &Path) -> &'static str {
    let ext = base
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase());
    match ext.as_deref() {
        Some("qcow2") => "qcow2",
        _ => "raw",
    }
}

/// 目的：同目录的 qemu-img 可执行文件名（便携包里与 qemu-system-* 并排）。
fn qemu_img_for(engine_exe: &str) -> String {
    let name = if cfg!(windows) {
        "qemu-img.exe"
    } else {
        "qemu-img"
    };
    match Path::new(engine_exe).parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join(name).to_string_lossy().into_owned(),
        _ => name.to_string(),
    }
}

/// 目的：在基础镜像上建一份每租约可写层（qcow2 覆盖）。
/// 约束：base 转绝对路径——覆盖文件里的后备路径是相对**覆盖文件所在目录**解析的。
fn create_overlay(engine_exe: &str, base: &Path, overlay: &Path) -> Result<(), String> {
    let base_abs =
        std::path::absolute(base).map_err(|e| format!("基础镜像路径取绝对失败：{}", e))?;
    let out = Command::new(qemu_img_for(engine_exe))
        .arg("create")
        .arg("-f")
        .arg("qcow2")
        .arg("-b")
        .arg(&base_abs)
        .arg("-F")
        .arg(backing_format(base))
        .arg(overlay)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("起不了 qemu-img（{}）：{}", qemu_img_for(engine_exe), e))?;
    if !out.status.success() {
        return Err(format!(
            "建可写层失败：{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
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
    /// 目的：QGA 的本地端口——通道被 QEMU 重置时照它重连（chardev 准备好之前连上的那条会被 RST）。
    port: u16,
    /// 首启种子服务；正常会话启动是 None。
    seed: Option<SeedServer>,
    /// 每租约可写层文件；stop / Drop 时删掉（删会话即丢写层）。
    overlay: Option<PathBuf>,
}

/// 目的：首启种子的小 HTTP 服务（cloud-init NoCloud net）——只在这次 boot 期间活着，随 guest 一起收掉。
struct SeedServer {
    server: std::sync::Arc<tiny_http::Server>,
    thread: Option<std::thread::JoinHandle<()>>,
    port: u16,
}

impl SeedServer {
    /// 目的：绑 127.0.0.1 的随机端口起服务；路径只认 `/user-data` 与 `/meta-data`，其余 404。
    fn start(docs: &SeedDocs) -> Result<SeedServer, String> {
        let server = std::sync::Arc::new(
            tiny_http::Server::http("127.0.0.1:0")
                .map_err(|e| format!("种子服务占不到端口：{}", e))?,
        );
        let port = server
            .server_addr()
            .to_ip()
            .map(|a| a.port())
            .ok_or_else(|| "种子服务没有 TCP 端口".to_string())?;
        let user_data = docs.user_data.clone();
        let meta_data = docs.meta_data.clone();
        let worker = std::sync::Arc::clone(&server);
        let thread = std::thread::spawn(move || {
            for req in worker.incoming_requests() {
                let path = req.url().split(['?', '#']).next().unwrap_or("").to_string();
                let resp = match path.as_str() {
                    "/user-data" => tiny_http::Response::from_string(user_data.clone()),
                    "/meta-data" => tiny_http::Response::from_string(meta_data.clone()),
                    _ => tiny_http::Response::from_string("").with_status_code(404),
                };
                let _ = req.respond(resp);
            }
        });
        Ok(SeedServer {
            server,
            thread: Some(thread),
            port,
        })
    }

    /// 目的：cloud-init NoCloud net 的种子 URL（guest 经 slirp 的 10.0.2.2 访问宿主）。
    fn url(&self) -> String {
        format!("http://10.0.2.2:{}/", self.port)
    }

    /// 目的：停服务并等线程退出（幂等）。
    fn stop(&mut self) {
        self.server.unblock();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for SeedServer {
    fn drop(&mut self) {
        self.stop();
    }
}

impl QemuHandle {
    /// 目的：优雅收尾——请 guest 自己断电，**等它落盘退出**（超时才杀）；再收种子服务与可写层。
    /// 约束：guest-shutdown 是异步的——不等它，guest 里刚要落盘的元数据（如刚建的 python 符号链接）会丢。
    fn shutdown_and_wait(&mut self) {
        let asked = self
            .send(&serde_json::json!({"execute":"guest-shutdown"}))
            .is_ok();
        if let Some(mut s) = self.seed.take() {
            s.stop();
        }
        if let Some(mut c) = self.child.take() {
            let mut waited = false;
            if asked {
                let deadline = Instant::now() + Duration::from_secs(8);
                loop {
                    match c.try_wait() {
                        Ok(Some(_)) => {
                            waited = true;
                            break;
                        }
                        Ok(None) if Instant::now() < deadline => {
                            std::thread::sleep(Duration::from_millis(100));
                        }
                        _ => break,
                    }
                }
            }
            if !waited {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
        if let Some(path) = self.overlay.take() {
            let _ = std::fs::remove_file(&path);
        }
    }

    /// 目的：通道断了就照同一个端口重连一次，并把读超时照原样装回去。
    /// 约束：QEMU 的 chardev 还没就绪时连上的那条会被它 RST——"连上"不等于"通"，断了就重连。
    fn reconnect(&mut self) -> Result<(), String> {
        let stream = connect_within(self.port, Duration::from_secs(5))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .map_err(|e| format!("给 QGA 通道设读超时失败：{}", e))?;
        self.writer = stream.try_clone().map_err(|e| e.to_string())?;
        self.reader = BufReader::new(stream);
        Ok(())
    }

    /// 目的：握手一次；不通就先重连再握手（通道被 QEMU 换掉是常态，不是故障）。
    fn sync_or_heal(&mut self) -> Result<(), String> {
        match self.sync() {
            Ok(()) => Ok(()),
            Err(_) => {
                self.reconnect()?;
                self.sync()
            }
        }
    }

    /// 目的：问一次进程状态；通道断了先重连再问一次（进程状态在 guest 侧，换通道不影响）。
    fn status_once(&mut self, pid: i64) -> Result<serde_json::Value, String> {
        let ask = |h: &mut QemuHandle| -> Result<serde_json::Value, String> {
            h.send(&serde_json::json!({
                "execute":"guest-exec-status","arguments":{"pid":pid}
            }))?;
            h.recv()
        };
        match ask(self) {
            Ok(v) => Ok(v),
            Err(_) => {
                self.reconnect()?;
                ask(self)
            }
        }
    }
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
    /// 目的：QGA 就绪探测——同步握手能过一次就算通道已就绪（读超时保证有界）。
    fn ping(&mut self) -> bool {
        self.sync_or_heal().is_ok()
    }

    fn exec(&mut self, run: &GuestRun) -> ToolOutcome {
        if let Err(why) = self.sync_or_heal() {
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
            let v = match self.status_once(pid) {
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
        self.shutdown_and_wait();
    }
}

impl Drop for QemuHandle {
    /// 目的：兜底收尾——种子服务、QEMU 进程与可写层都不能因为提前 drop 而漏掉；同样先请 guest 优雅断电。
    fn drop(&mut self) {
        self.shutdown_and_wait();
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 目的：QGA 通道被 QEMU 重置（chardev 未就绪时连上的那条会被 RST）不是故障——`ping` 要自己重连一次再握手。
    #[test]
    fn qga_ping_reconnects_after_a_reset_channel() {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").expect("绑端口");
        let port = listener.local_addr().expect("地址").port();
        let server = std::thread::spawn(move || {
            // 第一条：接受后立刻丢掉（模拟 QEMU 在 chardev 就绪前 RST 掉的那条连接）。
            let (first, _) = listener.accept().expect("accept#1");
            drop(first);
            // 第二条：正常服务一次 guest-sync（回它收到的那个 id）。
            let (mut second, _) = listener.accept().expect("accept#2");
            let read = second.try_clone().expect("clone");
            let mut r = BufReader::new(read);
            let mut line = String::new();
            r.read_line(&mut line).expect("读请求");
            let v: serde_json::Value = serde_json::from_str(line.trim()).expect("请求是 JSON");
            let id = v
                .get("arguments")
                .and_then(|a| a.get("id"))
                .and_then(|i| i.as_u64())
                .expect("guest-sync 要带 id");
            second
                .write_all(format!("{{\"return\": {}}}\n", id).as_bytes())
                .expect("写回包");
            std::thread::sleep(Duration::from_millis(200));
        });
        let first = TcpStream::connect(("127.0.0.1", port)).expect("先连上（会被服务器丢掉）");
        first
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("读超时");
        let mut handle = QemuHandle {
            child: None,
            reader: BufReader::new(first.try_clone().expect("clone")),
            writer: first,
            port,
            seed: None,
            overlay: None,
        };
        assert!(handle.ping(), "被 RST 的那条连接要能靠重连救回来");
        server.join().expect("服务器线程");
    }
    /// 目的：便携包的固件目录认 `bin/` 的兄弟 `share/qemu`；没有就给 None（不瞎指）。
    #[test]
    fn firmware_dir_is_beside_bin() {
        let root = crate::tests::scratch("qemu-fw");
        std::fs::create_dir_all(root.join("bin")).expect("建 bin");
        std::fs::create_dir_all(root.join("share").join("qemu")).expect("建固件目录");
        let exe = root.join("bin").join("qemu-system-x86_64");
        let fw = firmware_dir(&exe.to_string_lossy()).expect("有固件目录");
        assert_eq!(fw, root.join("share").join("qemu"));
        assert!(
            firmware_dir("/nonexistent/bin/qemu-system-x86_64").is_none(),
            "没有固件目录就不能瞎指 -L"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 目的：首启种子服务只认 `/user-data` 与 `/meta-data`，其余 404；是真的本机 HTTP。
    #[test]
    fn seed_server_serves_cloud_init_documents() {
        fn http_get(port: u16, path: &str) -> (u16, String) {
            use std::io::{Read, Write};
            let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).expect("连种子服务");
            s.set_read_timeout(Some(Duration::from_secs(3)))
                .expect("读超时");
            write!(s, "GET {} HTTP/1.0\r\nHost: x\r\n\r\n", path).expect("写请求");
            let mut buf = String::new();
            let _ = s.read_to_string(&mut buf);
            let status = buf
                .split_whitespace()
                .nth(1)
                .and_then(|c| c.parse().ok())
                .unwrap_or(0);
            let body = buf.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
            (status, body)
        }
        let mut server = SeedServer::start(&SeedDocs {
            user_data: "#cloud-config\nruncmd: [true]\n".to_string(),
            meta_data: "instance-id: t\n".to_string(),
        })
        .expect("起种子服务");
        assert!(
            server.url().starts_with("http://10.0.2.2:"),
            "{}",
            server.url()
        );
        let (s1, b1) = http_get(server.port, "/user-data");
        assert_eq!(s1, 200);
        assert!(b1.contains("#cloud-config"), "{}", b1);
        let (s2, b2) = http_get(server.port, "/meta-data");
        assert_eq!(s2, 200);
        assert!(b2.contains("instance-id"), "{}", b2);
        let (s3, _) = http_get(server.port, "/nope");
        assert_eq!(s3, 404);
        server.stop();
    }

    /// 目的：有种子就给 -smbios 数据源指路；没有种子就不给。
    #[test]
    fn seed_url_adds_smbios_datasource() {
        let image = GuestImage::linux(
            "t",
            GuestRoot::Disk(std::path::PathBuf::from("/img/root.qcow2")),
            None,
            None,
        );
        let spec = GuestSpec {
            lease: "w".to_string(),
            image,
            mounts: Vec::new(),
            net: true,
            engine_path: None,
            share_path: None,
            seed: None,
            isolated: false,
        };
        let with = engine_args(&spec, "opts", false, false, Some("http://10.0.2.2:9/"), &[]);
        assert!(
            with.windows(2)
                .any(|w| w[0] == "-smbios" && w[1].contains("ds=nocloud-net;s=http://10.0.2.2:9/")),
            "{:?}",
            with
        );
        let without = engine_args(&spec, "opts", false, false, None, &[]);
        assert!(!without.iter().any(|a| a == "-smbios"), "{:?}", without);
    }

    /// 目的：有可写层就把镜像的根换成它（内核仍取自基础根）；没有就原样。
    #[test]
    fn overlay_replaces_the_disk_root() {
        let base = std::path::PathBuf::from("/img/root.qcow2");
        let kernel = std::path::PathBuf::from("/img/vmlinuz");
        let image = GuestImage::linux(
            "t",
            GuestRoot::Disk(base.clone()),
            Some(kernel.clone()),
            None,
        );
        let with = with_overlay(&image, Some(Path::new("/ov/x.qcow2")));
        assert_eq!(
            with.root,
            GuestRoot::Disk(std::path::PathBuf::from("/ov/x.qcow2"))
        );
        assert_eq!(with.kernel, Some(kernel), "内核仍取自基础根");
        assert_eq!(with_overlay(&image, None).root, GuestRoot::Disk(base));
    }

    /// 目的：后备格式按扩展名认——qcow2 是 qcow2，ext4 / raw 这类按 raw（否则 qemu-img 会猜错）。
    #[test]
    fn backing_format_reads_the_extension() {
        assert_eq!(backing_format(Path::new("/x/rootfs.qcow2")), "qcow2");
        assert_eq!(backing_format(Path::new("/x/ROOTFS.QCOW2")), "qcow2");
        assert_eq!(backing_format(Path::new("/x/rootfs.ext4")), "raw");
        assert_eq!(backing_format(Path::new("/x/disk")), "raw");
    }

    /// 目的：可写层文件名不把租约名里的路径分隔符带进目录（否则会写到别的目录去）。
    #[test]
    fn overlay_file_name_is_safe() {
        assert_eq!(lease_file_name("demo-1"), "demo-1");
        assert_eq!(lease_file_name("a/b\\c"), "a_b_c");
        assert_eq!(lease_file_name("会话"), "__");
    }
}
