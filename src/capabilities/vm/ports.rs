//! 目的：vm 能力的出站端口。
//! 管：PackageSource（扫描 runtimes/ 包清单）、VmEngine / GuestVm / GuestChannel（起 / 跑 / 销毁 guest）与 ShareBackend（宿主↔guest 共享）。
//! 不管：清单校验与冲突预检（domain/packages.rs）、镜像判定与挂载清单（domain/guest.rs）、引擎参数（detail/）。
//! 联动：由 service.rs 持有（R12）；实现是 detail/fs_packages.rs、detail/qemu.rs 与 detail/share_ninep.rs。

use crate::capabilities::vm::domain::guest::{GuestImage, GuestMount, GuestRun, GuestSpec};
use crate::capabilities::vm::domain::packages::Library;
use crate::kernel::api::ToolOutcome;

/// 目的：运行包库来源端口——扫描依赖文件夹（runtimes/）里的包清单。
/// 约束：「清单即事实」，每次调用重扫，放入即出现；清单校验、去重与冲突预检在本能力（packages::Library::build），
///   目录遍历与 yaml 解析在适配层。只有 service.rs 持有它（R12）。
pub trait PackageSource {
    fn scan(&self) -> Library;
    /// 包库所在目录（配置界面要把"把包放哪儿"如实告诉用户）。
    fn dir(&self) -> std::path::PathBuf;
}

/// 目的：共享计划——把一次启动要挂的宿主目录翻成引擎参数与 guest 端挂载命令。
/// 约束：与引擎解耦是"尽量"：engine_args 由后端按调用方给的引擎 id 生成（同一后端可服务多引擎）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharePlan {
    /// 目的：后端稳定短名（"9p" 等），日志与诊断用。
    pub kind: &'static str,
    /// 目的：交给引擎的额外参数（QEMU 的 -fsdev / -device 等）。
    pub engine_args: Vec<String>,
    /// 目的：接上命令通道后在 guest 里逐条执行的挂载命令。
    pub guest_scripts: Vec<String>,
    /// 目的：本后端是否需要 guest 有网卡（guestfwd / SMB 这类隧道要）；9p / virtiofs 不需要。
    pub needs_net: bool,
}

/// 目的：一次共享装配的凭据——后端可能要在 guest 存活期间持有外部资源（如 virtiofsd 进程），
///   由 service 与 guest 同生命周期持有，销毁 guest 时一起释放。
pub trait ShareGuard: Send {
    fn release(&mut self);
}

/// 目的：没有外部资源的共享凭据（9p / QEMU 内置 virtiofs 这类）。
pub struct NoShareGuard;

impl ShareGuard for NoShareGuard {
    fn release(&mut self) {}
}

/// 目的：宿主↔guest 的共享后端（virtiofs / 9p / SMB / copy）。只有 service.rs 持有它（R12）。
pub trait ShareBackend: Send + Sync {
    /// 目的：后端稳定短名（诊断与日志用）。
    fn id(&self) -> &'static str;
    /// 目的：本后端能不能把共享落到这台引擎 / 这份 guest 上（纯事实）。
    fn supports(&self, engine: &str, guest: &GuestImage) -> bool;
    /// 目的：装配共享——产出计划与（可能的）外部资源凭据。
    fn prepare(
        &self,
        engine: &str,
        mounts: &[GuestMount],
    ) -> Result<(SharePlan, Box<dyn ShareGuard>), String>;
    /// 目的：guest 起来后、每次执行前的同步（copy 这类后端在这里拷进去）；默认无操作。
    fn sync_in(
        &self,
        _channel: &mut dyn GuestChannel,
        _mounts: &[GuestMount],
    ) -> Result<(), String> {
        Ok(())
    }
    /// 目的：每次执行后的回拷（copy 这类后端在这里拷回来）；默认无操作。
    fn sync_out(
        &self,
        _channel: &mut dyn GuestChannel,
        _mounts: &[GuestMount],
    ) -> Result<(), String> {
        Ok(())
    }
}

/// 目的：VM 引擎端口——探测承载、拉起 / 销毁一台 guest。只有 service.rs 持有它（R12）。
pub trait VmEngine: Send + Sync {
    /// 目的：引擎稳定短名（"qemu" / "vz"）。
    fn id(&self) -> &'static str;
    /// 目的：本引擎能不能承载这份 guest 镜像（纯事实）。
    fn supports(&self, guest: &GuestImage) -> bool;
    /// 目的：按规格与共享计划拉起一台 guest。
    fn boot(&self, spec: &GuestSpec, share: &SharePlan) -> Result<Box<dyn GuestVm>, String>;
}

/// 目的：一台已拉起的 guest——给出命令通道并销毁它。
pub trait GuestVm: Send {
    fn channel(&mut self) -> &mut dyn GuestChannel;
    fn stop(&mut self);
}

/// 目的：在 guest 里执行命令的通道（QGA / 自研 agent）。
pub trait GuestChannel: Send {
    fn exec(&mut self, run: &GuestRun) -> ToolOutcome;
}
