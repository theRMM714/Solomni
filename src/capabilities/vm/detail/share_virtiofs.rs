//! 目的：ShareBackend 的 virtiofs 实现——宿主侧起 virtiofsd（vhost-user 守护进程），QEMU 经 vhost-user-fs-pci 接上，guest 端 mount -t virtiofs。
//! 管：virtiofsd 的探测与起停、-chardev / -device 参数、guest 端挂载脚本与临时套接字清理。
//! 不管：QEMU 进程本身与共享内存后端（detail/qemu.rs 按 SharePlan.needs_shared_mem 给）。
//! 联动：实现 ports::ShareBackend；由组合根构造、service.rs 持有；只支持 Rust 版 virtiofsd（--shared-dir / --socket-path）。

use crate::capabilities::vm::domain::guest::{GuestImage, GuestMount};
use crate::capabilities::vm::ports::{ShareBackend, ShareGuard, SharePlan};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use crate::capabilities::vm::domain::exec::VIRTIOFSD_BIN;

/// 目的：virtiofs 共享后端（QEMU 的 vhost-user-fs；Linux / macOS 宿主可用，Windows 宿主没有）。
pub struct VirtioFsShare;

impl VirtioFsShare {
    /// 目的：组合根专用。
    pub fn new() -> VirtioFsShare {
        VirtioFsShare
    }
}

/// 目的：探测 virtiofsd——设置里登记的路径优先，其次看 PATH。
/// 返回：可执行文件路径；找不到给 None。
pub fn find_virtiofsd(registered: Option<&str>) -> Option<PathBuf> {
    match registered.map(str::trim) {
        Some(p) if !p.is_empty() => {
            let path = PathBuf::from(p);
            if path.is_file() {
                Some(path)
            } else {
                None
            }
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
        engine == "qemu" && guest.clients.virtio_fs
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
        let mut engine_args: Vec<String> = Vec::new();
        let mut guest_scripts: Vec<String> = Vec::new();
        for (i, m) in mounts.iter().enumerate() {
            let sock = socket_path();
            // 先把套接字与子进程交给凭据：中途任一步失败，Drop 都会把它们一起收掉，不留残留。
            guard.paths.push(sock.clone());
            guard
                .children
                .push(spawn_virtiofsd(&exe, &m.host, &sock, m.read_only)?);
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
            let mount_ro = if m.read_only { ",ro" } else { "" };
            guest_scripts.push(format!(
                "mkdir -p '{g}' && mount -t virtiofs{ro} m{i} '{g}'",
                g = m.guest,
                ro = mount_ro,
                i = i
            ));
        }
        Ok((
            SharePlan {
                kind: "virtiofs",
                engine_args,
                net_options: Vec::new(),
                guest_scripts,
                needs_net: false,
                // 有 vhost-user 设备才要共享内存；没有挂载就没有设备。
                needs_shared_mem: !mounts.is_empty(),
            },
            Box::new(guard),
        ))
    }
}

/// 目的：每次装配一个唯一的 vhost-user 套接字路径（进程内序号 + pid，避免并发租约互撞）。
fn socket_path() -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "solomni-virtiofs-{}-{}.sock",
        std::process::id(),
        n
    ))
}

/// 目的：起一个 virtiofsd——Rust 版 CLI（--shared-dir / --socket-path；C 版的 -o source= 不支持）。
/// 约束：--sandbox namespace 是上游默认的隔离方式（挂载命名空间里只剩共享目录）；日志压到 warn，经 stderr 丢弃。
fn spawn_virtiofsd(
    exe: &Path,
    shared_dir: &Path,
    sock: &Path,
    read_only: bool,
) -> Result<Child, String> {
    let mut cmd = Command::new(exe);
    cmd.arg("--shared-dir")
        .arg(shared_dir)
        .arg("--socket-path")
        .arg(sock)
        .arg("--sandbox")
        .arg("namespace")
        .arg("--log-level")
        .arg("warn");
    if read_only {
        cmd.arg("--readonly");
    }
    let what = exe.display().to_string();
    // --sandbox namespace 会让 virtiofsd fork 出真正的守护进程：把它放进自己的进程组，收尾时按组杀，别留孤儿。
    #[cfg(unix)]
    cmd.process_group(0);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("起不了 virtiofsd（{}）：{}", what, e))
}

/// 目的：等守护进程把套接字建出来；它提前退出就如实报错（不干等）。
fn wait_socket(sock: &Path, child: &mut Child, budget: Duration) -> Result<(), String> {
    let deadline = Instant::now() + budget;
    loop {
        if sock.exists() {
            return Ok(());
        }
        if let Ok(Some(st)) = child.try_wait() {
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
    children: Vec<Child>,
    paths: Vec<PathBuf>,
}

impl VirtioFsGuard {
    fn cleanup(&mut self) {
        for mut c in self.children.drain(..) {
            // 先杀整个进程组（含 sandbox 下 fork 出的守护进程），再收尸。
            #[cfg(unix)]
            unsafe {
                libc::kill(-(c.id() as i32), libc::SIGKILL);
            }
            let _ = c.kill();
            let _ = c.wait();
        }
        for p in self.paths.drain(..) {
            let _ = std::fs::remove_file(&p);
            // virtiofsd 会在套接字旁写一份 pid 文件，一起删掉，别留残留。
            let _ = std::fs::remove_file(p.with_extension("sock.pid"));
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
