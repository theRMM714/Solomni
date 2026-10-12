//! 目的：vm 能力的出站端口。
//! 管：PackageSource（扫描 runtimes/ 包清单）与 GuestHost（起 / 跑 / 销毁 guest）。
//! 不管：清单校验与冲突预检（domain/packages.rs）、挂载与 QEMU 参数的派生（domain/guest.rs）。
//! 联动：由 service.rs 持有（R12）；实现是 detail/fs_packages.rs 与 detail/qemu.rs。

use crate::capabilities::vm::domain::guest::{GuestRun, GuestSpec};
use crate::capabilities::vm::domain::packages::Library;
use crate::kernel::api::ToolOutcome;

/// 目的：运行包库来源端口——扫描依赖文件夹（runtimes/）里的包清单。
/// 约束：「清单即事实」，每次调用重扫，放入即出现；清单校验、去重与冲突预检在本能力（packages::Library::build），
///   目录遍历与 yaml 解析在适配层。只有 `service.rs` 持有它（R12）。
pub trait PackageSource {
    fn scan(&self) -> Library;
    /// 包库所在目录（配置界面要把"把包放哪儿"如实告诉用户）。
    fn dir(&self) -> std::path::PathBuf;
}

/// 目的：guest 后端端口——按规格拉起一个 guest。
/// 约束：只做机制；挂哪些根、给不给网卡由调用方派生后传入。只有 `service.rs` 持有它（R12）。
#[allow(dead_code)]
pub trait GuestHost: Send + Sync {
    fn start(&self, spec: &GuestSpec) -> Result<Box<dyn Guest>, String>;
}

/// 目的：一个已拉起的 guest——在它里面跑命令、销毁它。
#[allow(dead_code)]
pub trait Guest: Send {
    fn exec(&mut self, run: &GuestRun) -> ToolOutcome;
    fn stop(&mut self);
}
