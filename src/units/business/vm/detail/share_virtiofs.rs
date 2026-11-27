//! 目的：ShareBackend 的 virtiofs 实现——宿主侧起 virtiofsd（vhost-user 守护进程），QEMU 经 vhost-user-fs-pci 接上，guest 端 mount -t virtiofs。
//! 管：virtiofsd 的探测与起停、-chardev / -device 参数、guest 端挂载脚本与临时套接字清理。
//! 不管：QEMU 进程本身与共享内存后端（detail/qemu.rs 按 SharePlan.needs_shared_mem 给）。
//! 联动：实现 ports::ShareBackend；由组合根构造、service.rs 持有；只支持 Rust 版 virtiofsd（--shared-dir / --socket-path）。

use crate::units::business::vm::domain::guest::{GuestImage, GuestMount, sh_quote};
use crate::units::business::vm::ports::{ShareBackend, ShareGuard, SharePlan};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::kernel::api::env;
use crate::kernel::api::fs;
use crate::kernel::api::process::{Daemon, Spawn, spawn};
use crate::kernel::detail::procs::ProcessBook;
use crate::units::business::vm::domain::exec::VIRTIOFSD_BIN;

/// 目的：virtiofs 共享后端（QEMU 的 vhost-user-fs；Linux / macOS 宿主可用，Windows 宿主没有）。
pub struct VirtioFsShare {
    /// 长驻子进程的账（组合根注入；None = 不记账）。
    book: Option<std::sync::Arc<ProcessBook>>,
}

impl VirtioFsShare {
    /// 目的：组合根专用。
    pub fn new() -> VirtioFsShare {
        VirtioFsShare { book: None }
    }

    /// 目的：接上长驻子进程的账：每起一个 virtiofsd 登记一条，释放时注销。
    pub fn with_book(mut self, book: std::sync::Arc<ProcessBook>) -> VirtioFsShare {
        self.book = Some(book);
        self
    }
}

/// 目的：探测 virtiofsd——设置里登记的路径优先，其次看 PATH。
/// 返回：可执行文件路径；找不到给 None。
pub fn find_virtiofsd(registered: Option<&str>) -> Option<PathBuf> {
    match registered.map(str::trim) {
        Some(p) if !p.is_empty() => {
            let path = PathBuf::from(p);
            if path.is_file() { Some(path) } else { None }
        }
        _ => crate::kernel::detail::host_probe::find_exe(VIRTIOFSD_BIN),
    }
}

impl ShareBackend for VirtioFsShare {
    fn id(&self) -> &'static str {
        "virtiofs"
    }

    /// 约束：只有 QEMU 有 vhost-user-fs；guest 侧还要声明 virtiofs 客户端（Linux 内核 ≥5.4 自带）。
    fn supports(&self, engine: &str, guest: &GuestImage) -> bool {
        crate::units::business::vm::domain::exec::share_carries("virtiofs", engine, guest)
    }

    fn prepare(
        &self,
        _engine: &str,
        mounts: &[GuestMount],
        share_tool: Option<&str>,
    ) -> Result<(SharePlan, Box<dyn ShareGuard>), String> {
        let exe = find_virtiofsd(share_tool).ok_or_else(|| {
            format!(
                "没找到 {}（把它放进 PATH，或在设置里登记它的完整路径）",
                VIRTIOFSD_BIN
            )
        })?;
        let mut guard = VirtioFsGuard::default();
        guard.book = self.book.clone();
        let mut engine_args: Vec<String> = Vec::new();
        for (i, m) in mounts.iter().enumerate() {
            let sock = socket_path();
            // 先把套接字与子进程交给凭据：中途任一步失败，Drop 都会把它们一起收掉，不留残留。
            guard.paths.push(sock.clone());
            // 起进程这一步把所有兜底与登记一起办好（父死子亡、独立进程组、长命线程、账上那条）。
            let daemon = spawn_virtiofsd(
                &exe,
                &m.host,
                &sock,
                m.read_only,
                guard.book.clone(),
                &m.guest,
            )?;
            guard.children.push(daemon);
            // QEMU 的 chardev 是客户端：必须等守护进程真在监听，引擎才能连上。
            wait_socket(
                &sock,
                guard.children.last_mut().expect("刚推进去"),
                Duration::from_secs(10),
            )?;
            engine_args.push("-chardev".to_string());
            engine_args.push(format!("socket,id=vf{},path={}", i, sock.display()));
            engine_args.push("-device".to_string());
            engine_args.push(format!("vhost-user-fs-pci,chardev=vf{},tag=m{}", i, i));
        }
        Ok((virtiofs_plan(mounts, engine_args), Box::new(guard)))
    }
}

/// 目的：把挂载清单翻成 virtiofs 共享计划——引擎参数由 prepare 按已起的守护进程给，guest 端每条挂载一条
///   mkdir + mount 脚本；guest 路径可能含单引号（外部工作区镜像进 guest 后），拼进脚本一律走 `sh_quote`。
pub(crate) fn virtiofs_plan(mounts: &[GuestMount], engine_args: Vec<String>) -> SharePlan {
    let guest_scripts = mounts
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let mount_ro = if m.read_only { ",ro" } else { "" };
            format!(
                "mkdir -p {g} && mount -t virtiofs{ro} m{i} {g}",
                g = sh_quote(&m.guest),
                ro = mount_ro,
                i = i
            )
        })
        .collect();
    SharePlan {
        kind: "virtiofs",
        engine_args,
        net_options: Vec::new(),
        guest_scripts,
        needs_net: false,
        // 有 vhost-user 设备才要共享内存；没有挂载就没有设备。
        needs_shared_mem: !mounts.is_empty(),
    }
}

/// 目的：每次装配一个唯一的 vhost-user 套接字路径（进程内序号 + pid，避免并发租约互撞）。
fn socket_path() -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    env::temp_dir().join(format!(
        "solomni-virtiofs-{}-{}.sock",
        std::process::id(),
        n
    ))
}

/// 目的：起一个 virtiofsd——Rust 版 CLI（--shared-dir / --socket-path；C 版的 -o source= 不支持）。
/// 约束：--sandbox namespace 是上游默认的隔离方式（挂载命名空间里只剩共享目录）；日志压到 warn，经 stderr 丢弃。
///   `--sandbox namespace` 会 fork 出真正的守护进程（与发起进程同组）：靠进程组整组收，
///   父死子亡盯住发起进程——它一死，沙箱命名空间里的守护进程随之结束。
fn spawn_virtiofsd(
    exe: &Path,
    shared_dir: &Path,
    sock: &Path,
    read_only: bool,
    book: Option<std::sync::Arc<ProcessBook>>,
    guest: &str,
) -> Result<Daemon, String> {
    let mut spec = Spawn::new(exe)
        .arg("--shared-dir")
        .arg(shared_dir)
        .arg("--socket-path")
        .arg(sock)
        .arg("--sandbox")
        .arg("namespace")
        .arg("--log-level")
        .arg("warn")
        .purpose("share:virtiofs")
        .owner(guest)
        // 对账时靠这一段在命令行里认出"还是我们起的那一个"。
        .marker(format!("--socket-path {}", sock.display()))
        .book(book);
    if read_only {
        spec = spec.arg("--readonly");
    }
    spawn(spec)
}

/// 目的：等守护进程把套接字建出来；它提前退出就如实报错（不干等）。
fn wait_socket(sock: &Path, child: &mut Daemon, budget: Duration) -> Result<(), String> {
    let deadline = Instant::now() + budget;
    loop {
        if sock.exists() {
            return Ok(());
        }
        if let Some(st) = child.try_wait() {
            return Err(format!("virtiofsd 提前退出（{:?}）", st.code()));
        }
        if Instant::now() >= deadline {
            return Err("等 virtiofsd 的套接字超时".to_string());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// 目的：共享凭据——持会话期间的所有 virtiofsd 进程与它们的套接字；释放即杀进程、删套接字（幂等）。
#[derive(Default)]
pub struct VirtioFsGuard {
    children: Vec<Daemon>,
    paths: Vec<PathBuf>,
    /// 长驻子进程的账（起进程时交给句柄，收尾时由它注销）。
    book: Option<std::sync::Arc<ProcessBook>>,
}

impl VirtioFsGuard {
    fn cleanup(&mut self) {
        for mut daemon in self.children.drain(..) {
            // 先杀整个进程组（含 sandbox 下 fork 出的守护进程）、再收尸、顺手销账——都在句柄里。
            daemon.stop();
        }
        for p in self.paths.drain(..) {
            let _ = fs::remove_tree(&p);
            // virtiofsd 会在套接字旁写一份 pid 文件，一起删掉，别留残留。
            let _ = fs::remove_tree(&p.with_extension("sock.pid"));
        }
    }
}

impl ShareGuard for VirtioFsGuard {
    fn release(&mut self) {
        self.cleanup();
    }
}

impl Drop for VirtioFsGuard {
    fn drop(&mut self) {
        // service 在 guest 起来失败时直接 drop 凭据（不显式 release），这里兜底清理。
        self.cleanup();
    }
}
