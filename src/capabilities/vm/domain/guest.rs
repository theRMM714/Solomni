//! 目的：guest 的纯逻辑——镜像布局判定与挂载清单。
//! 管：一份 guest 镜像是不是可用、宿主根到 guest 的挂载清单。
//! 不管：进程的启停与命令通道（detail/ 的引擎与通道）、引擎参数（detail/ 的适配器）。
//! 联动：被 service.rs 与 detail/ 消费；契约见 docs/vm/README.md 与 RUNTIME_SPEC.md。

use crate::capabilities::vm::domain::exec::ExecPlan;
use crate::capabilities::vm::domain::packages::KIND_SYSTEM;
use crate::kernel::api::FenceSpec;
use crate::kernel::ports::HostProbe;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// 目的：guest 的处理器架构（镜像架构要与宿主能虚拟化的架构一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestArch {
    Amd64,
    Arm64,
}

impl GuestArch {
    /// 目的：本机能虚拟化的架构（x86_64 → amd64；aarch64 → arm64）。
    pub fn host() -> GuestArch {
        if cfg!(target_arch = "aarch64") {
            GuestArch::Arm64
        } else {
            GuestArch::Amd64
        }
    }
}

/// 目的：guest 根的承载方式——目录（QEMU 经 9p 当根，仅 Linux / macOS 宿主）或磁盘镜像（三平台通用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestRoot {
    /// 目录根：QEMU 的 virtfs 把它挂成 guest 的 /；Windows 宿主没有 virtfs。
    Dir(PathBuf),
    /// 磁盘镜像根（qcow2 / raw）：宿主只是普通文件 I/O，三平台与各引擎通用（Windows 宿主没有 9p）。
    Disk(PathBuf),
}

/// 目的：guest 镜像里装的命令通道 agent（命令通道按它选实现）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestAgent {
    /// QEMU guest agent（QGA），走 virtio-serial。
    Qga,
}

/// 目的：guest 里具备哪些共享客户端——决定哪个共享后端能落上去（缺客户端就是缺，不假装）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestClients {
    /// 目的：9p2000.L 客户端（Linux 内核自带）。
    pub nine_p: bool,
    /// 目的：virtio-fs 客户端（Linux 内核 ≥5.4 自带）。
    pub virtio_fs: bool,
    /// 目的：CIFS 客户端（要镜像装 cifs-utils）。
    pub cifs: bool,
}

impl GuestClients {
    /// 目的：默认 Debian guest 具备的客户端（9p + virtio-fs；CIFS 要镜像显式装 cifs-utils）。
    pub const fn debian() -> GuestClients {
        GuestClients {
            nine_p: true,
            virtio_fs: true,
            cifs: false,
        }
    }
}

/// 目的：一份 guest 镜像（四轴里的 guest 轴）——根（目录或磁盘镜像）+ 内核（可选 initrd）+ agent 与共享客户端能力。
/// 约束：磁盘镜像与更多 guest 见 docs/vm/README.md 的「多平台组合」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestImage {
    /// 目的：镜像名（诊断与日志用；默认取基础根目录名）。
    pub id: String,
    pub arch: GuestArch,
    pub root: GuestRoot,
    pub kernel: PathBuf,
    pub initrd: Option<PathBuf>,
    pub agent: GuestAgent,
    pub clients: GuestClients,
}

impl GuestImage {
    /// 目的：一份 Linux guest 镜像——默认 QGA 命令通道与 Debian 的共享客户端能力。
    pub fn linux(
        id: &str,
        root: GuestRoot,
        kernel: PathBuf,
        initrd: Option<PathBuf>,
    ) -> GuestImage {
        GuestImage {
            id: id.to_string(),
            arch: GuestArch::host(),
            root,
            kernel,
            initrd,
            agent: GuestAgent::Qga,
            clients: GuestClients::debian(),
        }
    }
}

/// 目的：guest 里的一处挂载（宿主路径 → guest 路径；只读或可写）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestMount {
    pub host: PathBuf,
    pub guest: String,
    pub read_only: bool,
}

/// 目的：一次 guest 启动的规格（镜像 + 挂载 + 网络 + 会话租约 + 引擎路径登记）。
#[derive(Debug, Clone)]
pub struct GuestSpec {
    pub lease: String,
    pub image: GuestImage,
    pub mounts: Vec<GuestMount>,
    pub net: bool,
    /// 目的：设置里登记的**引擎**可执行文件路径（空 = 兜底按 PATH 找，名字由 arch 定）。
    /// 约束：与就绪检测共用同一份登记——检测说"按登记路径找到"，启动就必须走同一个路径。
    pub engine_path: Option<String>,
}

/// 目的：在 guest 里跑一次工具（命令 + 参数 + 环境 + 工作目录）。
#[derive(Debug, Clone)]
pub struct GuestRun {
    pub command: String,
    pub args_json: String,
    pub env: Vec<(String, String)>,
    pub cwd: String,
}

/// 目的：把基础根目录判成一份可用的 guest 镜像（rootfs 目录 + vmlinuz/bzImage + 可选 initrd）。
/// 返回：判定出来的镜像；不到一处可用的布局时给一句缺什么。
pub fn classify_base(base: &Path, probe: &dyn HostProbe) -> Result<GuestImage, String> {
    if !probe.is_dir(base) {
        return Err("基础根不在场（不是一个目录）".to_string());
    }
    let kernel = ["vmlinuz", "bzImage"]
        .iter()
        .map(|n| base.join(n))
        .find(|p| probe.is_file(p));
    let Some(kernel) = kernel else {
        return Err("基础根里没有 vmlinuz / bzImage".to_string());
    };
    let initrd = ["initrd.img", "initrd", "initramfs.img"]
        .iter()
        .map(|n| base.join(n))
        .find(|p| probe.is_file(p));
    // 目录里放着 rootfs 磁盘镜像就用磁盘根（三平台通用），否则用目录根（QEMU 的 virtfs 把它挂成 /）。
    let disk = [
        "rootfs.ext4",
        "rootfs.qcow2",
        "disk.qcow2",
        "disk.raw",
        "rootdisk.qcow2",
    ]
    .iter()
    .map(|n| base.join(n))
    .find(|p| probe.is_file(p));
    let root = match disk {
        Some(image) => GuestRoot::Disk(image),
        None => GuestRoot::Dir(base.to_path_buf()),
    };
    let id = base
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "guest".to_string());
    Ok(GuestImage::linux(&id, root, kernel, initrd))
}

/// 目的：把执行计划与围栏翻成 guest 挂载——prefix 包只读挂到 /<prefix>，围栏根按宿主原路径挂。
/// 约束：围栏根在 guest 里保持宿主原路径，模块命令与参数里的真实路径因此仍然成立；system 叠层不做。
pub fn mounts_from(plan: &ExecPlan, fence: &FenceSpec) -> Vec<GuestMount> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut out: Vec<GuestMount> = Vec::new();
    for p in &plan.packages {
        if p.kind == KIND_SYSTEM || p.source_dir.as_os_str().is_empty() {
            continue;
        }
        let guest = format!("/{}", p.prefix.trim_matches('/'));
        if !seen.insert(guest.clone()) {
            continue;
        }
        out.push(GuestMount {
            host: p.source_dir.clone(),
            guest,
            read_only: true,
        });
    }
    let roots: [(&[PathBuf], bool); 3] = [
        (&fence.rw, false),
        (&fence.ro, true),
        (&fence.ro_tree, true),
    ];
    for (list, read_only) in roots {
        for host in list {
            let guest = host.to_string_lossy().into_owned();
            if !seen.insert(guest.clone()) {
                continue;
            }
            out.push(GuestMount {
                host: host.clone(),
                guest,
                read_only,
            });
        }
    }
    out
}
