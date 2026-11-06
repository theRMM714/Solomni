//! 目的：ShareBackend 的 9p（virtfs）实现——把宿主目录经 virtio-9p 暴露给 guest，并给出 guest 端挂载命令。
//! 管：-fsdev / -device virtio-9p-pci 参数与 mkdir + mount -t 9p 脚本。
//! 不管：进程启停与通道（detail/qemu.rs）、镜像判定与挂载清单（domain/guest.rs）。
//! 联动：实现 ports::ShareBackend；由组合根构造、service.rs 持有。

use crate::capabilities::vm::domain::guest::{sh_quote, GuestImage, GuestMount};
use crate::capabilities::vm::ports::{NoShareGuard, ShareBackend, ShareGuard, SharePlan};

/// 目的：9p 共享后端（QEMU 的 virtfs；Linux / macOS 宿主可用，Windows 宿主没有）。
pub struct NinePShare;

impl NinePShare {
    /// 目的：组合根专用。
    pub fn new() -> NinePShare {
        NinePShare
    }
}

/// 目的：把挂载清单翻成 9p 共享计划——每条挂载一条 -fsdev / -device 与一条 mkdir + mount 脚本。
/// 约束：guest 路径可能含单引号（外部工作区把宿主路径镜像进 guest），拼进脚本一律走 `sh_quote`；
///   -fsdev 的 path= 按逗号分隔选项，宿主路径含逗号时 QEMU 会把它拆错——本期不扩展转义（用户决定）。
pub(crate) fn ninep_plan(mounts: &[GuestMount]) -> SharePlan {
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
            "mkdir -p {g} && mount -t 9p -o trans=virtio,version=9p2000.L{ro} m{i} {g}",
            g = sh_quote(&m.guest),
            ro = mount_ro,
            i = i
        ));
    }
    SharePlan {
        kind: "9p",
        engine_args,
        net_options: Vec::new(),
        guest_scripts,
        needs_net: false,
        needs_shared_mem: false,
    }
}

impl ShareBackend for NinePShare {
    fn id(&self) -> &'static str {
        "9p"
    }

    /// 约束：只有 QEMU 有 virtfs；guest 侧还要声明了 9p2000.L 客户端（Linux 内核自带）。
    fn supports(&self, engine: &str, guest: &GuestImage) -> bool {
        crate::capabilities::vm::domain::exec::share_carries("9p", engine, guest)
    }

    fn prepare(
        &self,
        _engine: &str,
        mounts: &[GuestMount],
        _share_tool: Option<&str>,
    ) -> Result<(SharePlan, Box<dyn ShareGuard>), String> {
        Ok((ninep_plan(mounts), Box::new(NoShareGuard)))
    }
}
