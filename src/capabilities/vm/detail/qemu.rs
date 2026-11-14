//! 目的：VmEngine / GuestChannel 的 QEMU 实现——拉起 qemu-system-*，经 QGA 执行与销毁。
//! 管：QEMU 进程的启停、QEMU 基础参数（含 rootfs 的 9p 根共享与 QGA 串口）、JSON 收发、命令回执与 base64 数据。
//! 不管：共享后端的额外参数（detail/share_ninep.rs）、镜像判定与挂载清单（domain/guest.rs）。
//! 联动：端口在 ports.rs；由组合根构造、service.rs 持有。

use crate::capabilities::vm::domain::guest::{GuestArch, GuestImage, GuestOs, GuestRun, GuestSpec};
use crate::capabilities::vm::ports::{GuestChannel, GuestVm, SharePlan, VmEngine};
use crate::kernel::detail::process::CodeProcessTexts;
use crate::kernel::detail::procs::{self, Entry, ProcessBook};
use crate::kernel::ports::ProcessTexts;
use std::collections::VecDeque;
use std::io::BufReader;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// 目的：**无围栏** guest 命令（供给编排 / apt / 下载工具链）的外层等待预算。
/// 约束：有围栏的命令仍按 guest helper 的 30 秒杀树；供给编排可能下载几百 MB，30 秒不够。
const UNFENCED_GUEST_RUN_SECS: u64 = 1800;

/// 目的：本平台的硬件加速参数（-machine 的 accel=…）；None = 这个平台没有可用的硬件加速，虚拟机档不可用。
/// 约束：**只用硬件加速，不退回软件模拟（TCG）**：软件模拟慢到没有可用性（实测同一台 guest 慢 2–5 倍，
///   共享挂载在慢启动里间歇失败），静默回退会把"这台机器没有加速"伪装成"虚拟机档时好时坏"。
pub fn accelerators() -> Option<&'static str> {
    if crate::kernel::api::IS_WINDOWS {
        Some("accel=whpx")
    } else if crate::kernel::api::IS_MACOS {
        Some("accel=hvf")
    } else if crate::kernel::api::IS_LINUX {
        Some("accel=kvm")
    } else {
        None
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
    /// 长驻子进程的账（组合根注入；None = 不记账——测试与不需要对账的场合）。
    book: Option<std::sync::Arc<ProcessBook>>,
    /// 运行日志（组合根注入；None = 只转控制台）：QEMU 自己的话要能进 logs/ 与对账现场。
    log: Option<std::sync::Arc<dyn crate::kernel::ports::Log + Send + Sync>>,
    /// shell 回执的收尾文案（与本机守门进程同一份 ProcessTexts）：guest 通道据此产出同形状态行。
    texts: std::sync::Arc<dyn ProcessTexts>,
}

impl QemuEngine {
    /// 目的：组合根专用（不带可写层：供给编排与测试用）。
    pub fn new() -> QemuEngine {
        QemuEngine {
            overlay_dir: None,
            book: None,
            log: None,
            texts: std::sync::Arc::new(CodeProcessTexts),
        }
    }

    /// 目的：组合根专用（带每租约可写层的落点：会话的 guest 盘写在这里，删会话即丢）。
    pub fn with_overlay_dir(dir: PathBuf) -> QemuEngine {
        QemuEngine {
            overlay_dir: Some(dir),
            book: None,
            log: None,
            texts: std::sync::Arc::new(CodeProcessTexts),
        }
    }

    /// 目的：接上 shell 回执文案（组合根注入与本机守门进程同一份；不接就用生产实现）。
    pub fn with_texts(mut self, texts: std::sync::Arc<dyn ProcessTexts>) -> QemuEngine {
        self.texts = texts;
        self
    }

    /// 目的：接上长驻子进程的账（组合根注入）：每次 boot 登记一条，销毁时注销。
    pub fn with_book(mut self, book: std::sync::Arc<ProcessBook>) -> QemuEngine {
        self.book = Some(book);
        self
    }

    /// 目的：接上运行日志（组合根注入）：QEMU 的 stderr 逐行进 logs/，同时仍转控制台。
    pub fn with_log(
        mut self,
        log: std::sync::Arc<dyn crate::kernel::ports::Log + Send + Sync>,
    ) -> QemuEngine {
        self.log = Some(log);
        self
    }

    /// 目的：把 QEMU 的 stderr 收成管道并持续抽干——裸字节直写 CP936 控制台会把 UTF-8 文本解成乱码。
    /// 返回：最近几行（启动失败时拼进错误，别丢 QEMU 自己说的话）。
    /// 约束：管道必须一直抽，否则写满会把 QEMU 卡住。
    fn supervise_stderr(
        &self,
        child: &mut Child,
    ) -> std::sync::Arc<std::sync::Mutex<VecDeque<String>>> {
        let tail = std::sync::Arc::new(std::sync::Mutex::new(VecDeque::new()));
        let Some(stderr) = child.stderr.take() else {
            return tail;
        };
        let kept = std::sync::Arc::clone(&tail);
        let log = self.log.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let mut pending: Vec<u8> = Vec::new();
            let mut reader = std::io::BufReader::new(stderr);
            loop {
                use std::io::Read;
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        pending.extend_from_slice(&buf[..n]);
                        while let Some(at) = pending.iter().position(|b| *b == b'\n') {
                            let line: Vec<u8> = pending.drain(..=at).collect();
                            emit_stderr_line(&line, log.as_deref(), &kept);
                        }
                    }
                }
            }
            if !pending.is_empty() {
                emit_stderr_line(&pending, log.as_deref(), &kept);
            }
        });
        tail
    }
}

/// 目的：QEMU 的一行 stderr 去哪——日志、控制台与最近几行各一份。
fn emit_stderr_line(
    raw: &[u8],
    log: Option<&(dyn crate::kernel::ports::Log + Send + Sync)>,
    tail: &std::sync::Arc<std::sync::Mutex<VecDeque<String>>>,
) {
    let text = decode_stderr_line(raw);
    if text.is_empty() {
        return;
    }
    if let Some(log) = log {
        log.warn("vm::qemu", &text);
    }
    eprintln!("[虚拟机] qemu：{}", text);
    if let Ok(mut q) = tail.lock() {
        q.push_back(text);
        while q.len() > 20 {
            q.pop_front();
        }
    }
}

/// 目的：把一行 stderr 字节解成可读文本——QEMU 的 Windows 错误文本是 UTF-8，按 UTF-8 宽容解码即可。
fn decode_stderr_line(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw)
        .trim_end_matches(['\r', '\n'])
        .trim()
        .to_string()
}

/// 目的：启动失败时把 QEMU 最近说的话拼进错误（没有就空串）。
fn last_words(tail: &std::sync::Mutex<VecDeque<String>>) -> String {
    let Ok(q) = tail.lock() else {
        return String::new();
    };
    if q.is_empty() {
        return String::new();
    }
    format!(
        "；QEMU：{}",
        q.iter().cloned().collect::<Vec<_>>().join("；")
    )
}

/// 目的：guest 内存大小（MiB）——`-m` 与共享内存后端（vhost-user 要）必须同一尺寸，所以只写一处。
pub const GUEST_MEM_MB: u32 = 2048;

/// 目的：给 QEMU 的磁盘格式——`.qcow2` 是每租约可写层，其余布局按 raw。
/// 约束：必须显式传 `format=`：raw 被自动探测时 QEMU 会限制 block 0 的写，并每次打警告。
fn disk_format(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("qcow2") => "qcow2",
        _ => "raw",
    }
}

/// 目的：除共享之外的一次启动参数（不含可执行文件本身）——机器、加速器、内存、内核、串口、QGA 通道与首启种子。
/// 参数：channel_opts 是 QGA chardev 的补充选项；share_needs_net = 共享后端要不要 guest 有（受限）网卡；
///   share_needs_mem = 共享后端要不要 guest RAM 是**可共享内存**（vhost-user 设备，如 virtiofs）；
///   seed_url = 首启 cloud-init 的 NoCloud net 种子根 URL（None = 不投种子）。
/// 约束：根永远是磁盘镜像（`-drive file=…,if=virtio,format=…`；会话的那一份是每租约可写层），有内核就 `root=/dev/vda` 直启。
pub fn engine_args(
    spec: &GuestSpec,
    channel_opts: &str,
    share_needs_net: bool,
    share_needs_mem: bool,
    seed_url: Option<&str>,
    net_options: &[String],
) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    // 没有硬件加速的平台不会走到启动（boot 先拒绝）；这里给一个 QEMU 不认识的加速器，
    // 万一被绕过也在启动时立刻报错，绝不悄悄退回 TCG。
    let accel = accelerators().unwrap_or("accel=none");
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
        accel,
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
    // 进程名里带上归属：对账时靠它在进程表里认出"这是我们那个租约的 QEMU"。
    a.push("-name".to_string());
    a.push(format!("solomni:{}", spec.lease));
    // 有内核就直启（-kernel / -initrd / -append）；没有内核的磁盘根靠镜像自带的引导（固件启动）。
    if let Some(kernel) = &spec.image.kernel {
        a.push("-kernel".to_string());
        a.push(kernel.display().to_string());
        if let Some(initrd) = &spec.image.initrd {
            a.push("-initrd".to_string());
            a.push(initrd.display().to_string());
        }
    }
    // 根永远是磁盘镜像（会话的那一份是每租约可写层）。
    a.push("-drive".to_string());
    a.push(format!(
        "file={},if=virtio,format={}",
        spec.image.disk.display(),
        disk_format(&spec.image.disk)
    ));
    // 构建缓存卷：第二块 virtio-blk（稀疏 qcow2）。discard / detect-zeroes 让 guest 的 fstrim 把空块还给宿主。
    if let Some(cache) = &spec.cache_disk {
        a.push("-drive".to_string());
        a.push(format!(
            "file={},if=virtio,format=qcow2,discard=unmap,detect-zeroes=unmap",
            cache.display()
        ));
    }
    // 有内核就直启、由命令行给根设备；没有内核靠镜像自带引导（固件）。
    if spec.image.kernel.is_some() {
        a.push("-append".to_string());
        a.push("console=ttyS0 root=/dev/vda rw".to_string());
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

    /// 约束：架构要匹配；根是磁盘镜像，三平台通用。
    fn supports(&self, guest: &GuestImage) -> bool {
        if guest.arch != GuestArch::host() {
            return false;
        }
        true
    }

    fn boot(&self, spec: &GuestSpec, share: &SharePlan) -> Result<Box<dyn GuestVm>, String> {
        // 只用硬件加速：没有就如实拒绝（不回退软件模拟，TCG 的启动与共享都慢到没有可用性）。
        let Some(accel) = accelerators() else {
            return Err(
                "本平台没有可用的硬件加速：虚拟机档只走硬件加速（KVM / WHPX / HVF），不退回软件模拟"
                    .to_string(),
            );
        };
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
        let overlay = match (&spec.image.disk, spec.isolated) {
            (base, true) => {
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
        let mut cmd = Command::new(&exe);
        cmd.args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            // stderr 收成管道再逐行转出来：QEMU 的 Windows 错误文本是 UTF-8，裸字节直写 CP936 控制台会变乱码。
            .stderr(Stdio::piped());
        // 父死子亡（Unix）与独立进程组：核心被强杀时不许 QEMU 继续攥着镜像。
        procs::guard(&mut cmd);
        let mut child = procs::spawn(cmd).map_err(|e| format!("起不了 QEMU（{}）：{}", exe, e))?;
        // 管道必须一直抽干（写满会把 QEMU 卡住）；同时留最近几行给启动失败用。
        let said = self.supervise_stderr(&mut child);
        // 作业对象（Windows）/ 守门进程（macOS）：核心一死，它跟着死；挂不上只如实少一层兜底，不阻断启动。
        let launched = procs::attach(&child, "guest");
        let pid = child.id();
        if let Some(book) = &self.book {
            book.record(Entry {
                pid,
                since: procs::now_ts(),
                purpose: "guest".to_string(),
                owner: spec.lease.clone(),
                marker: format!("solomni:{}", spec.lease),
                created: launched.created,
                guard_note: launched.guard_note,
            });
        }
        drop(listener);
        let stream = match connect_within(port, Duration::from_secs(30), Some(&mut child)) {
            Ok(s) => s,
            Err(e) => {
                // 连不上就把刚拉起的 QEMU 与刚建的可写层都收掉，别留孤儿。
                if let Some(book) = &self.book {
                    book.forget(pid);
                }
                let _ = child.kill();
                let _ = child.wait();
                if let Some(path) = &overlay {
                    let _ = std::fs::remove_file(path);
                }
                // QEMU 一到就被加速器判死：错误里给出去处，避免用户以为是共享或镜像的问题。
                let hint = if e.contains("已退出") {
                    format!(
                        "；本档只用硬件加速（{}），加速器不可用时不退回软件模拟",
                        accel
                    )
                } else {
                    String::new()
                };
                return Err(format!("{}{}{}", e, hint, last_words(&said)));
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
            book: self.book.clone(),
            pid,
            texts: std::sync::Arc::clone(&self.texts),
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
                timeout_secs: 30,
                output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
            });
            if !out.ok {
                handle.stop();
                return Err(format!("把共享挂进 guest 失败：{}", out.output));
            }
        }
        // 会话 guest 每次都是新的可写层：把 per-agent 围栏助手装进去；装不上这台 guest 就不能用。
        if spec.isolated
            && spec.image.os == GuestOs::Linux
            && let Err(e) = super::guest_helper::install(&mut handle)
        {
            handle.stop();
            return Err(e);
        }
        Ok(Box::new(handle))
    }
}

/// 目的：便携 QEMU 的固件目录——可执行文件的 `bin/` 的兄弟 `share/qemu`（存在才算）。
fn firmware_dir(exe: &str) -> Option<PathBuf> {
    let bin = Path::new(exe).parent()?;
    let dir = bin.parent()?.join("share").join("qemu");
    if dir.is_dir() { Some(dir) } else { None }
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
        out.disk = path.to_path_buf();
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
    let name = format!("qemu-img{}", crate::kernel::api::EXE_SUFFIX);
    match Path::new(engine_exe).parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join(name).to_string_lossy().into_owned(),
        _ => name.to_string(),
    }
}

/// 目的：建一块稀疏的空盘（qcow2）——构建缓存卷用，与任何镜像无关。
/// 参数：`engine_exe` = QEMU 可执行文件（同目录找 qemu-img）；`path` = 落点；`mib` = 虚拟大小。
/// 错误：起不了 qemu-img 或它报失败。
pub fn create_disk(engine_exe: &str, path: &Path, mib: u64) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("建缓存卷目录失败（{}）：{}", dir.display(), e))?;
    }
    let out = Command::new(qemu_img_for(engine_exe))
        .arg("create")
        .arg("-f")
        .arg("qcow2")
        .arg(path)
        .arg(format!("{}M", mib))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("起不了 qemu-img（{}）：{}", qemu_img_for(engine_exe), e))?;
    if !out.status.success() {
        return Err(format!(
            "建缓存卷失败：{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
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

/// 目的：在预算内等 QGA 通道；同时盯着 QEMU 进程——它提前退出（例如加速器不可用）就立刻报出来，
///   不把"加速器起不来"拖成 30 秒的"等通道超时"。
mod qga;
mod seed;

use qga::{QemuHandle, connect_within};
use seed::SeedServer;

#[cfg(test)]
mod tests {
    use super::qga::*;
    use super::*;
    use crate::capabilities::vm::domain::guest::{OutputLimit, SeedDocs};
    use crate::kernel::domain::receipt::{ShellEnd, shell_receipt};
    use std::net::TcpStream;

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
            book: None,
            pid: 0,
            texts: std::sync::Arc::new(CodeProcessTexts),
        };
        assert!(handle.ping(), "被 RST 的那条连接要能靠重连救回来");
        server.join().expect("服务器线程");
    }
    /// 目的：QEMU 的一行 stderr 按 UTF-8 宽容解码——中文系统错误不再被 CP936 解成乱码；坏字节也不炸。
    #[test]
    fn qemu_stderr_lines_decode_as_utf8() {
        let chinese = "qemu-system-x86_64.exe: warning: failed to WSAEventSelect(): 在一个非套接字上尝试了一个操作。";
        assert_eq!(decode_stderr_line(chinese.as_bytes()), chinese);
        assert_eq!(decode_stderr_line(b"line\r\n"), "line");
        assert!(decode_stderr_line(&[0xff, 0xfe, b'x']).contains('x'));
        assert_eq!(decode_stderr_line(b"   "), "");
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
        let image = GuestImage::linux("t", std::path::PathBuf::from("/img/root.qcow2"), None, None);
        let spec = GuestSpec {
            lease: "w".to_string(),
            image,
            mounts: Vec::new(),
            net: true,
            engine_path: None,
            share_path: None,
            shares: Vec::new(),
            seed: None,
            isolated: false,
            cache_disk: None,
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

    /// 目的：给了构建缓存卷就多一块 virtio-blk（qcow2 + discard，好让 fstrim 归还空间）；没给就不加。
    #[test]
    fn cache_disk_becomes_a_second_virtio_drive() {
        let image = GuestImage::linux("t", std::path::PathBuf::from("/img/root.qcow2"), None, None);
        let spec = GuestSpec {
            lease: "w".to_string(),
            image,
            mounts: Vec::new(),
            net: false,
            engine_path: None,
            share_path: None,
            shares: Vec::new(),
            seed: None,
            isolated: false,
            cache_disk: None,
        };
        let without = engine_args(&spec, "opts", false, false, None, &[]);
        assert!(
            !without.iter().any(|a| a.contains("discard=unmap")),
            "没有缓存卷就不该多一块盘：{:?}",
            without
        );
        let mut with = spec.clone();
        with.cache_disk = Some(std::path::PathBuf::from("/cache/x.qcow2"));
        let args = engine_args(&with, "opts", false, false, None, &[]);
        assert!(
            args.windows(2).any(|w| w[0] == "-drive"
                && w[1].contains("file=/cache/x.qcow2")
                && w[1].contains("format=qcow2")
                && w[1].contains("discard=unmap")),
            "缓存卷要作为第二块 virtio 盘挂上：{:?}",
            args
        );
    }

    /// 目的：有可写层就把镜像的根换成它（内核仍取自基础根）；没有就原样。
    #[test]
    fn overlay_replaces_the_disk_root() {
        let base = std::path::PathBuf::from("/img/root.qcow2");
        let kernel = std::path::PathBuf::from("/img/vmlinuz");
        let image = GuestImage::linux("t", base.clone(), Some(kernel.clone()), None);
        let with = with_overlay(&image, Some(Path::new("/ov/x.qcow2")));
        assert_eq!(with.disk, std::path::PathBuf::from("/ov/x.qcow2"));
        assert_eq!(with.kernel, Some(kernel), "内核仍取自基础根");
        assert_eq!(with_overlay(&image, None).disk, base);
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

    /// 目的：guest shell 回执的收尾只看 helper 机器标记与宿主取消旗，不猜自然语言。
    #[test]
    fn guest_shell_end_reads_machine_markers_only() {
        assert_eq!(guest_shell_end("", 3, false), GuestShellEnd::Exit(3));
        assert_eq!(
            guest_shell_end("warn\nsolomni-guest-fence: timeout\n", 124, false),
            GuestShellEnd::Timeout
        );
        assert_eq!(
            guest_shell_end("solomni-guest-fence: signal\n", 111, false),
            GuestShellEnd::Signal
        );
        assert_eq!(
            guest_shell_end("solomni-guest-fence: landlock is unavailable\n", 111, false),
            GuestShellEnd::NotRun,
            "helper 的 die 诊断 = 没执行，不产出 shell 收尾行"
        );
        assert_eq!(
            guest_shell_end("solomni-guest-fence: something\n", 0, true),
            GuestShellEnd::Cancelled,
            "取消旗优先"
        );
        assert_eq!(
            guest_shell_end("用户脚本自己的报错\n", 1, false),
            GuestShellEnd::Exit(1),
            "普通 stderr 不是标记"
        );
    }

    /// 目的：helper 的机器标记不能漏进最终回执；围栏失败的诊断原文要保留。
    #[test]
    fn helper_markers_are_stripped_but_diagnostics_are_kept() {
        assert_eq!(
            strip_helper_markers("warn\nsolomni-guest-fence: timeout\n"),
            "warn\n"
        );
        assert_eq!(strip_helper_markers("solomni-guest-fence: timeout\n"), "");
        assert_eq!(strip_helper_markers("solomni-guest-fence: signal\n"), "");
        assert_eq!(
            strip_helper_markers("solomni-guest-fence: landlock is unavailable\n"),
            "solomni-guest-fence: landlock is unavailable\n"
        );
    }

    /// 目的：guest 通道装出来的 shell 回执与本机（kernel 装配）逐字节同形。
    #[test]
    fn guest_shell_receipt_matches_the_host_shape() {
        let run = GuestRun {
            command: "true".to_string(),
            args_json: "{}".to_string(),
            env: Vec::new(),
            cwd: "/w".to_string(),
            timeout_secs: 3,
            output_limit: OutputLimit::Shell,
        };
        let texts = CodeProcessTexts;
        let host = shell_receipt(
            &texts,
            "hello\n",
            "",
            ShellEnd::Exit(Some(0)),
            GUEST_SHELL_MAX_CHARS,
        );
        let guest = shell_outcome(&texts, &run, "hello\n", "", 0, false);
        assert_eq!(guest.output, host, "正常回执要同形");
        assert!(guest.ok);

        let host_timeout = shell_receipt(
            &texts,
            "",
            "",
            ShellEnd::TimedOut { secs: 3 },
            GUEST_SHELL_MAX_CHARS,
        );
        let guest_timeout = shell_outcome(
            &texts,
            &run,
            "",
            "solomni-guest-fence: timeout\n",
            124,
            false,
        );
        assert_eq!(guest_timeout.output, host_timeout, "超时回执要同形");
        assert!(!guest_timeout.ok);
    }
}
