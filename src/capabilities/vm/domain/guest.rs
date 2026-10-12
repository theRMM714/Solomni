//! 目的：guest 的纯逻辑——基础根布局判定、挂载清单与 QEMU 参数构造。
//! 管：基础根是不是一份可用的 guest、宿主根到 guest 的挂载、一次启动的 QEMU 命令行。
//! 不管：进程的启停与命令通道（后端 detail/）、包的选择与定版（domain/exec.rs）。
//! 联动：被 service.rs、conductor 与 QEMU 适配器消费；契约见 docs/vm/README.md 与 RUNTIME_SPEC.md。

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

/// 目的：一份用户自备的 guest 基础根——rootfs 目录 + 内核（可选 initrd）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestBase {
    pub root: PathBuf,
    pub kernel: PathBuf,
    pub initrd: Option<PathBuf>,
}

/// 目的：guest 里的一处挂载（宿主路径 → guest 路径；只读或可写）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestMount {
    pub host: PathBuf,
    pub guest: String,
    pub read_only: bool,
}

/// 目的：一次 guest 启动的规格（基础根 + 挂载 + 网络 + 会话租约）。
#[derive(Debug, Clone)]
pub struct GuestSpec {
    pub lease: String,
    pub arch: GuestArch,
    pub base: GuestBase,
    pub mounts: Vec<GuestMount>,
    pub net: bool,
}

/// 目的：在 guest 里跑一次工具（命令 + 参数 + 环境 + 工作目录）。
#[derive(Debug, Clone)]
pub struct GuestRun {
    pub command: String,
    pub args_json: String,
    pub env: Vec<(String, String)>,
    pub cwd: String,
}

/// 目的：QEMU 可执行文件名（架构不同名字不同）。
pub fn qemu_exe(arch: GuestArch) -> &'static str {
    match arch {
        GuestArch::Amd64 => "qemu-system-x86_64",
        GuestArch::Arm64 => "qemu-system-aarch64",
    }
}

/// 目的：把基础根目录判成 guest 基础（rootfs 目录 + vmlinuz/bzImage + 可选 initrd）。
/// 返回：判定出来的基础；不到一处可用的布局时给一句缺什么。
pub fn classify_base(base: &Path, probe: &dyn HostProbe) -> Result<GuestBase, String> {
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
    Ok(GuestBase {
        root: base.to_path_buf(),
        kernel,
        initrd,
    })
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

/// 目的：一次启动的 QEMU 参数（不含可执行文件本身）。
/// 参数：channel_opts 是 QGA chardev 的补充选项（形如 host=127.0.0.1,port=N,server=on,wait=off）。
/// 约束：rootfs 经 virtio-9p 的 base 标签挂成根；net 为假时完全不给网卡。
pub fn qemu_args(spec: &GuestSpec, channel_opts: &str) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    for v in [
        "-m",
        "2048",
        "-smp",
        "2",
        "-nographic",
        "-no-reboot",
        "-nodefaults",
    ] {
        a.push(v.to_string());
    }
    a.push("-kernel".to_string());
    a.push(spec.base.kernel.display().to_string());
    if let Some(initrd) = &spec.base.initrd {
        a.push("-initrd".to_string());
        a.push(initrd.display().to_string());
    }
    a.push("-fsdev".to_string());
    a.push(format!(
        "local,id=fsbase,path={},security_model=none,readonly=on",
        spec.base.root.display()
    ));
    a.push("-device".to_string());
    a.push("virtio-9p-pci,fsdev=fsbase,mount_tag=base".to_string());
    a.push("-append".to_string());
    a.push(
        "console=ttyS0 root=base rootfstype=9p rootflags=trans=virtio,version=9p2000.L rw"
            .to_string(),
    );
    for (i, m) in spec.mounts.iter().enumerate() {
        let ro = if m.read_only { ",readonly=on" } else { "" };
        a.push("-fsdev".to_string());
        a.push(format!(
            "local,id=fs{},path={},security_model=none{}",
            i,
            m.host.display(),
            ro
        ));
        a.push("-device".to_string());
        a.push(format!("virtio-9p-pci,fsdev=fs{},mount_tag=m{}", i, i));
    }
    a.push("-chardev".to_string());
    a.push(format!("socket,id=ch0,{}", channel_opts));
    a.push("-device".to_string());
    a.push("virtio-serial-pci".to_string());
    a.push("-device".to_string());
    a.push("virtserialport,chardev=ch0,name=org.qemu.guest_agent.0".to_string());
    if spec.net {
        a.push("-netdev".to_string());
        a.push("user,id=n0".to_string());
        a.push("-device".to_string());
        a.push("virtio-net-pci,netdev=n0".to_string());
    } else {
        a.push("-nic".to_string());
        a.push("none".to_string());
    }
    a
}
