//! 目的：guest 基础根的纯逻辑——布局判定。
//! 管：基础根目录是不是一份可用的 guest（发行版 rootfs + vmlinuz/bzImage + 可选 initrd）。
//! 不管：启动与命令通道（后端 detail/）、包的选择与定版（domain/exec.rs）。
//! 联动：被 domain/exec.rs 的基础根检测消费；契约见 docs/vm/README.md 与 RUNTIME_SPEC.md。

use crate::kernel::ports::HostProbe;
use std::path::{Path, PathBuf};

/// 目的：一份用户自备的 guest 基础根——rootfs 目录 + 内核（可选 initrd）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestBase {
    pub root: PathBuf,
    pub kernel: PathBuf,
    pub initrd: Option<PathBuf>,
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
