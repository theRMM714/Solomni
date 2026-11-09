//! 目的：依赖补齐能力——按清单把不预置的依赖下载并解包进产品内，并管一次补齐作业。
//! 管：清单 / 台账解析、本机状态、下载缓存、解包落盘与作业状态。
//! 不管：虚拟机档的能力语义（在 vm）；界面怎么展示（在 presentation）。
//! 联动：清单与台账的唯一真相在仓库根，产品里编译期内嵌；呈现层经 api 调，组合根注入实现。

pub mod api;
pub mod detail;
pub mod domain;
pub(crate) mod guest;
pub mod ports;
pub(crate) mod service;

pub use service::ProvisionService;

/// 目的：按本机适配器装配一份补齐实现——组合根只给产品根与取数端口，其余端口在这里落成。
/// 约束：只由组合根（main / diagnostics / 用例）调用；guest-runtime 会被如实拒绝，需要时用 `assemble_with_vm`。
pub fn assemble(
    root: &std::path::Path,
    fetch: std::sync::Arc<dyn ports::Fetcher>,
) -> ProvisionService {
    ProvisionService::new(
        root,
        std::sync::Arc::new(detail::host::RealHost),
        std::sync::Arc::new(detail::ledger::FileLedger::new(root)),
        std::sync::Arc::new(detail::install::HostInstallers),
        std::sync::Arc::new(detail::guest_image::Ext4Images::new(
            root,
            std::sync::Arc::clone(&fetch),
        )),
        std::sync::Arc::new(detail::procs::BookProcs::new(root)),
        fetch,
    )
}

/// 目的：带 vm 能力面的装配（组合根 / --provision 入口用）；guest-runtime 编排需要它。
pub fn assemble_with_vm(
    root: &std::path::Path,
    fetch: std::sync::Arc<dyn ports::Fetcher>,
    vm: std::sync::Arc<dyn crate::capabilities::vm::api::Vm + Send + Sync>,
) -> ProvisionService {
    ProvisionService::with_vm(
        root,
        std::sync::Arc::new(detail::host::RealHost),
        std::sync::Arc::new(detail::ledger::FileLedger::new(root)),
        std::sync::Arc::new(detail::install::HostInstallers),
        std::sync::Arc::new(detail::guest_image::Ext4Images::new(
            root,
            std::sync::Arc::clone(&fetch),
        )),
        std::sync::Arc::new(detail::procs::BookProcs::new(root)),
        fetch,
        vm,
    )
}
#[cfg(test)]
pub(crate) use detail::ledger::sync_toolchain_active;
/// 目的：给跨层用例提供 kind: toolchain 的清单条目形状与登记入口（只在测试构建里出现）。
#[cfg(test)]
pub(crate) use domain::{Dep, ToolchainSpec};
