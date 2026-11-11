//! 目的：组合根的共用装配——把产品根变成能力面，供启动程序、探针与用例复用。
//! 管：各能力的端口选型、适配器构造与依赖顺序；同一份能力面只在这里装一次。
//! 不管：业务用例、参数解析、回执打印（在 main / entry::probes）；任何业务语义。
//! 联动：消费者是 `src/main.rs`、`src/entry/probes/*` 与测试；契约见 ARCHITECTURE.md 的「根」。

use std::path::Path;
use std::sync::Arc;

use crate::capabilities::builds::service::BuildsService;
use crate::capabilities::provision::api::Provision;
use crate::capabilities::provision::detail::fetch::HttpFetch;
use crate::capabilities::provision::detail::guest_image::Ext4Images;
use crate::capabilities::provision::detail::host::RealHost;
use crate::capabilities::provision::detail::install::HostInstallers;
use crate::capabilities::provision::detail::ledger::FileLedger;
use crate::capabilities::provision::detail::procs::BookProcs;
use crate::capabilities::provision::ports::Fetcher;
use crate::capabilities::provision::service::{ProvisionDeps, ProvisionService};
use crate::capabilities::storage::api::StoragePolicy;
use crate::capabilities::storage::service::StorageService;
use crate::capabilities::vm::api::Vm;

/// 目的：一次性取数适配器——补齐与探针共用同一组超时（连接 15s，整体 7200s）。
fn fetch() -> Arc<dyn Fetcher> {
    Arc::new(HttpFetch::new(15, 7200))
}

/// 目的：补齐能力的端口组合——根只给产品根与取数端口，其余适配器在这里落成。
fn provision_deps(root: &Path, fetch: Arc<dyn Fetcher>) -> ProvisionDeps {
    ProvisionDeps {
        host: Arc::new(RealHost),
        ledger: Arc::new(FileLedger::new(root)),
        installers: Arc::new(HostInstallers),
        images: Arc::new(Ext4Images::new(root, Arc::clone(&fetch))),
        procs: Arc::new(BookProcs::new(root)),
        fetch,
        probe: Arc::new(crate::kernel::detail::host_probe::HostProbeAdapter),
    }
}

/// 目的：不带 vm 的补齐能力面（只查状态、只装不需要 guest 的条目）。
/// 约束：guest-runtime 会被如实拒绝；需要时用 `provision_with_vm`。
pub(crate) fn provision(root: &Path) -> ProvisionService {
    ProvisionService::new(root, provision_deps(root, fetch()))
}

/// 目的：带 vm 的补齐能力面；guest-runtime 编排需要它。
pub(crate) fn provision_with_vm(root: &Path, vm: Arc<dyn Vm + Send + Sync>) -> ProvisionService {
    ProvisionService::with_vm(root, provision_deps(root, fetch()), vm)
}

/// 目的：带供给 vm 的补齐能力面——vm 只持内部 `none` 共享档，进程台账按产品根现场打开。
pub(crate) fn provision_with_guest_vm(root: &Path) -> ProvisionService {
    provision_with_vm(
        root,
        provision_vm(crate::kernel::detail::procs::ProcessBook::open(
            &root.join(".home"),
        )),
    )
}

/// 目的：供给编排专用的 vm 能力面：只持内部 `none` 共享档（只经 QGA，不挂载、不建可写层）。
/// 约束：与会话 vm 分开装配——`none` 不进用户候选链，避免 `clients` 为空的 guest 被静默当成无共享。
pub(crate) fn provision_vm(
    book: Arc<crate::kernel::detail::procs::ProcessBook>,
) -> Arc<dyn Vm + Send + Sync> {
    provision_vm_with(Arc::new(
        crate::capabilities::vm::detail::QemuEngine::new().with_book(book),
    ))
}

/// 目的：同上，但允许注入引擎替身（回归测试用）；生产入口只用 `provision_vm`。
pub(crate) fn provision_vm_with(
    engine: Arc<dyn crate::capabilities::vm::ports::VmEngine + Send + Sync>,
) -> Arc<dyn Vm + Send + Sync> {
    Arc::new(crate::capabilities::vm::service::VmService::new(
        engine,
        vec![Arc::new(crate::capabilities::vm::detail::NoneShare::new())],
    ))
}

/// 目的：按 settings.yaml 取存储策略；读不到用默认。
fn storage_policy(root: &Path) -> StoragePolicy {
    std::fs::read_to_string(root.join(".home").join("settings.yaml"))
        .ok()
        .and_then(|t| {
            yaml_serde::from_str::<crate::capabilities::registry::api::AppSettings>(&t).ok()
        })
        .map(|a| StoragePolicy::from(&a.storage))
        .unwrap_or_default()
}

/// 目的：用给定补齐面装一份存储能力（不另装核心）。
pub(crate) fn storage(
    root: &Path,
    provision: Arc<dyn Provision + Send + Sync>,
    policy: StoragePolicy,
) -> StorageService {
    StorageService::new(
        provision,
        Arc::new(crate::capabilities::storage::detail::survey::DiskSurvey::new(root.to_path_buf())),
        Arc::new(
            crate::capabilities::storage::detail::reclaim::DiskReclaim::new(root.to_path_buf()),
        ),
        Arc::new(crate::capabilities::storage::detail::guests::ProcessGuests::new(root)),
        Arc::new(crate::kernel::ports::NoopLog),
        policy,
    )
}

/// 目的：探针用的存储能力面——按产品根现场装补齐（含供给 vm）与存储策略。
pub(crate) fn storage_from_settings(root: &Path) -> StorageService {
    let provision: Arc<dyn Provision + Send + Sync> = Arc::new(provision_with_guest_vm(root));
    storage(root, provision, storage_policy(root))
}

/// 目的：虚拟机档构建用的 vm 能力面：**只持挂载式**共享档（virtiofs / 9p / smb）。
/// 约束：模块目录是读写挂进去的，产物才能落回宿主那份——构建只持挂载式用户档；
///   `copy` 是开发/应急通道，不装配；`none` 只给供给编排。
pub(crate) fn build_vm(root: &Path) -> Arc<dyn Vm + Send + Sync> {
    let shares: Vec<Arc<dyn crate::capabilities::vm::ports::ShareBackend + Send + Sync>> = {
        let book = crate::kernel::detail::procs::ProcessBook::open(&root.join(".home"));
        vec![
            Arc::new(
                crate::capabilities::vm::detail::VirtioFsShare::new().with_book(Arc::clone(&book)),
            ),
            Arc::new(crate::capabilities::vm::detail::NinePShare::new()),
            Arc::new(crate::capabilities::vm::detail::SmbShare::new().with_book(book)),
        ]
    };
    Arc::new(crate::capabilities::vm::service::VmService::new(
        Arc::new(
            crate::capabilities::vm::detail::QemuEngine::with_overlay_dir(
                root.join(".vm").join("overlays"),
            ),
        ),
        shares,
    ))
}

/// 目的：构建用的执行器——与外层工具执行同一套守门进程机制（围栏在它里面装）。
pub(crate) fn build_runner(
    root: &Path,
) -> Result<crate::kernel::detail::process::ProcTools, String> {
    let texts: Arc<dyn crate::kernel::ports::ProcessTexts> =
        Arc::new(crate::kernel::detail::process::CodeProcessTexts);
    Ok(crate::kernel::detail::process::ProcTools::new(
        std::env::current_exe().unwrap_or_default(),
        texts,
        root.join(".home"),
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    ))
}

/// 目的：命令行构建用的能力面——包库、虚拟机、补齐、执行器与基础工具链安装根按产品根现场装配。
pub(crate) fn build_service(root: &Path) -> Result<BuildsService, String> {
    let packages: Arc<dyn crate::capabilities::packages::api::Packages + Send + Sync> = Arc::new(
        crate::capabilities::packages::service::PackagesService::with_base(
            Arc::new(crate::capabilities::packages::detail::FsPackages::new(
                root.join("runtimes"),
            )),
            crate::entry::toolchains::base_facts(root),
        ),
    );
    let provision: Arc<dyn Provision + Send + Sync> = Arc::new(provision(root));
    let storage: Arc<dyn crate::capabilities::storage::api::Storage + Send + Sync> = Arc::new(
        storage(root, Arc::clone(&provision), StoragePolicy::default()),
    );
    let runner: Arc<dyn crate::kernel::ports::ProcessRunner + Send + Sync> =
        Arc::new(build_runner(root)?);
    let store = Arc::new(
        crate::capabilities::builds::detail::fingerprint::BuildStore::new(root.to_path_buf()),
    );
    Ok(BuildsService::new(
        root.to_path_buf(),
        packages,
        build_vm(root),
        provision,
        storage,
        store.clone(),
        store,
        Arc::new(crate::capabilities::builds::detail::cache::CacheOnDisk::new(root.to_path_buf())),
        runner,
        Arc::new(crate::kernel::detail::confine::FenceHostAdapter::new(
            root.join(".home"),
        )),
        Arc::new(crate::kernel::detail::host_probe::HostProbeAdapter),
        Arc::new(crate::kernel::ports::NoopLog),
        crate::entry::toolchains::active_roots(root),
    ))
}
