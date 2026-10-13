//! 目的：ShareBackend 的 9p（virtfs）实现——把宿主目录经 virtio-9p 暴露给 guest，并给出 guest 端挂载命令。
//! 管：-fsdev / -device virtio-9p-pci 参数与 mkdir + mount -t 9p 脚本。
//! 不管：进程启停与通道（detail/qemu.rs）、镜像判定与挂载清单（domain/guest.rs）。
//! 联动：实现 ports::ShareBackend；由组合根构造、service.rs 持有。

use crate::capabilities::vm::domain::guest::{GuestImage, GuestMount};
use crate::capabilities::vm::ports::{ShareBackend, SharePlan};

/// 目的：9p 共享后端（QEMU 的 virtfs；Linux / macOS 宿主可用，Windows 宿主没有）。
pub struct NinePShare;

impl NinePShare {
    /// 目的：组合根专用。
    pub fn new() -> NinePShare {
        NinePShare
    }
}

impl ShareBackend for NinePShare {
    fn id(&self) -> &'static str {
        "9p"
    }

    /// 约束：只有 QEMU 有 virtfs；guest 侧需要 9p2000.L 客户端（Linux 内核自带）。
    fn supports(&self, engine: &str, _guest: &GuestImage) -> bool {
        engine == "qemu"
    }

    fn plan(&self, _engine: &str, mounts: &[GuestMount]) -> Result<SharePlan, String> {
        let mut engine_args: Vec<String> = Vec::new();
        let mut guest_scripts: Vec<String> = Vec::new();
        for (i, m) in mounts.iter().enumerate() {
            let ro = if m.read_only { ",readonly=on" } else { "" };
            engine_args.push("-fsdev".to_string());
            engine_args.push(format!(
                "local,id=fs{},path={},security_model=none{}",
                i,
                m.host.display(),
                ro
            ));
            engine_args.push("-device".to_string());
            engine_args.push(format!("virtio-9p-pci,fsdev=fs{},mount_tag=m{}", i, i));
            let mount_ro = if m.read_only { ",ro" } else { "" };
            guest_scripts.push(format!(
                "mkdir -p '{g}' && mount -t 9p -o trans=virtio,version=9p2000.L{ro} m{i} '{g}'",
                g = m.guest,
                ro = mount_ro,
                i = i
            ));
        }
        Ok(SharePlan {
            kind: "9p",
            engine_args,
            guest_scripts,
            needs_net: false,
        })
    }
}
