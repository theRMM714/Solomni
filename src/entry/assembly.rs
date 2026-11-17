//! 目的：组合根的共用装配——把产品根变成能力面，供启动程序、探针与用例复用。
//! 管：各能力的端口选型、适配器构造与依赖顺序；同一份能力面只在这里装一次。
//! 不管：业务用例、参数解析、回执打印（在 main / presentation/cli/boot.rs）；任何业务语义。
//! 联动：消费者是 `src/main.rs`、`src/presentation/cli/boot.rs` 与测试；契约见 ARCHITECTURE.md 的「根」。

use crate::kernel::api::layout;
use std::path::Path;
use std::sync::Arc;

use crate::capabilities::catalog::packages::api::BaseFacts;
use crate::capabilities::maintenance::builds::service::BuildsService;
use crate::capabilities::maintenance::storage::api::StoragePolicy;
use crate::capabilities::maintenance::storage::service::StorageService;
use crate::capabilities::provision::api::Provision;
use crate::capabilities::provision::detail::fetch::HttpFetch;
use crate::capabilities::provision::detail::guest_image::Ext4Images;
use crate::capabilities::provision::detail::host::RealHost;
use crate::capabilities::provision::detail::install::HostInstallers;
use crate::capabilities::provision::detail::ledger::FileLedger;
use crate::capabilities::provision::detail::procs::BookProcs;
use crate::capabilities::provision::ports::Fetcher;
use crate::capabilities::provision::service::{ProvisionDeps, ProvisionService};
use crate::capabilities::system::service::{SystemDeps, SystemService};
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
            &layout::home(root),
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
    std::fs::read_to_string(layout::home(root).join("settings.yaml"))
        .ok()
        .and_then(|t| {
            yaml_serde::from_str::<crate::capabilities::catalog::registry::api::AppSettings>(&t)
                .ok()
        })
        .map(|a| StoragePolicy::from(&a.storage))
        .unwrap_or_default()
}

/// 目的：四份登记处 yaml 的读取面——组合根按产品根装，启动与排障都走这一份。
pub(crate) fn settings_store(
    root: &Path,
) -> crate::capabilities::catalog::registry::detail::yaml_settings::YamlSettingsStore {
    crate::capabilities::catalog::registry::detail::yaml_settings::YamlSettingsStore::new(
        layout::home(root).join("providers.yaml"),
        layout::home(root).join("models.yaml"),
        layout::home(root).join("settings.yaml"),
        layout::home(root).join("agents.yaml"),
    )
}

/// 目的：用给定补齐面装一份存储能力（不另装核心）。
pub(crate) fn storage(
    root: &Path,
    provision: Arc<dyn Provision + Send + Sync>,
    policy: StoragePolicy,
) -> StorageService {
    StorageService::new(
        provision,
        Arc::new(
            crate::capabilities::maintenance::storage::detail::survey::DiskSurvey::new(
                root.to_path_buf(),
            ),
        ),
        Arc::new(
            crate::capabilities::maintenance::storage::detail::reclaim::DiskReclaim::new(
                root.to_path_buf(),
            ),
        ),
        Arc::new(
            crate::capabilities::maintenance::storage::detail::guests::ProcessGuests::new(root),
        ),
        Arc::new(crate::kernel::ports::NoopLog),
        policy,
    )
}

/// 目的：虚拟机档构建用的 vm 能力面：**只持挂载式**共享档（virtiofs / 9p / smb）。
/// 约束：模块目录是读写挂进去的，产物才能落回宿主那份——构建只持挂载式用户档；
///   `copy` 是开发/应急通道，不装配；`none` 只给供给编排。
pub(crate) fn build_vm(root: &Path) -> Arc<dyn Vm + Send + Sync> {
    let shares: Vec<Arc<dyn crate::capabilities::vm::ports::ShareBackend + Send + Sync>> = {
        let book = crate::kernel::detail::procs::ProcessBook::open(&layout::home(root));
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
                layout::vm(root).join("overlays"),
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
        layout::home(root),
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    ))
}

/// 目的：命令行构建用的能力面——包库、虚拟机、补齐、执行器与基础工具链安装根按产品根现场装配。
pub(crate) fn build_service(root: &Path) -> Result<BuildsService, String> {
    let packages: Arc<dyn crate::capabilities::catalog::packages::api::Packages + Send + Sync> =
        Arc::new(
            crate::capabilities::catalog::packages::service::PackagesService::with_base(
                Arc::new(
                    crate::capabilities::catalog::packages::detail::FsPackages::new(
                        layout::runtimes(root),
                    ),
                ),
                crate::entry::toolchains::base_facts(root),
            ),
        );
    let provision: Arc<dyn Provision + Send + Sync> = Arc::new(provision(root));
    let storage: Arc<dyn crate::capabilities::maintenance::storage::api::Storage + Send + Sync> =
        Arc::new(storage(
            root,
            Arc::clone(&provision),
            StoragePolicy::default(),
        ));
    let runner: Arc<dyn crate::kernel::ports::ProcessRunner + Send + Sync> =
        Arc::new(build_runner(root)?);
    let store = Arc::new(
        crate::capabilities::maintenance::builds::detail::fingerprint::BuildStore::new(
            root.to_path_buf(),
        ),
    );
    Ok(BuildsService::new(
        root.to_path_buf(),
        packages,
        build_vm(root),
        provision,
        storage,
        store.clone(),
        store,
        Arc::new(
            crate::capabilities::maintenance::builds::detail::cache::CacheOnDisk::new(
                root.to_path_buf(),
            ),
        ),
        runner,
        Arc::new(crate::capabilities::tools::service::fence::ToolFenceGate),
        Arc::new(crate::kernel::detail::confine::FenceHostAdapter::new(
            layout::home(root),
        )),
        Arc::new(crate::kernel::detail::host_probe::HostProbeAdapter),
        Arc::new(crate::kernel::ports::NoopLog),
        crate::entry::toolchains::active_roots(root),
    ))
}
/// 目的：系统维护能力面的端口：组合根里已经装好的同一份实例，启动相命令直接复用。
/// 约束：一处进程只要一套存储 / 补齐 / 构建——另装一套会让后台作业与日志互相看不见。
pub(crate) struct SystemPorts {
    pub storage: Arc<dyn crate::capabilities::maintenance::storage::api::Storage + Send + Sync>,
    pub provision: Arc<dyn Provision + Send + Sync>,
    pub builds: Arc<dyn crate::capabilities::maintenance::builds::api::Builds + Send + Sync>,
    pub workspace: Arc<dyn crate::capabilities::workspace::api::Workspace + Send + Sync>,
    pub modules: Arc<dyn crate::capabilities::catalog::modules::api::Modules + Send + Sync>,
}

/// 目的：启动相命令用的系统维护能力面——存储 / 补齐 / 构建 / 工作区 / 模块与工具链事实按产品根现场装。
/// 约束：只在这里装一次；能力自己不构造别的能力的适配器（依赖由本函数注入）。
pub(crate) fn system(root: &Path, base: &BaseFacts) -> Result<SystemService, String> {
    let provision: Arc<dyn Provision + Send + Sync> = Arc::new(provision_with_guest_vm(root));
    let storage: Arc<dyn crate::capabilities::maintenance::storage::api::Storage + Send + Sync> =
        Arc::new(storage(root, Arc::clone(&provision), storage_policy(root)));
    let builds: Arc<dyn crate::capabilities::maintenance::builds::api::Builds + Send + Sync> =
        Arc::new(build_service(root)?);
    let workspace: Arc<dyn crate::capabilities::workspace::api::Workspace + Send + Sync> = Arc::new(
        crate::capabilities::workspace::service::WorkspaceService::new(
            Arc::new(
                crate::capabilities::workspace::detail::fs_workspace::FsWorkspace::new(
                    root.join("session"),
                ),
            ),
            Arc::new(crate::capabilities::workspace::detail::fs_workstore::FsWorkStore::new()),
        ),
    );
    let modules: Arc<dyn crate::capabilities::catalog::modules::api::Modules + Send + Sync> =
        Arc::new(
            crate::capabilities::catalog::modules::service::ModulesService::new(
                Arc::new(
                    crate::capabilities::catalog::modules::detail::FsModules::new(
                        root.join("modules"),
                        crate::capabilities::tools::api::names(),
                    ),
                ),
                Arc::new(
                    crate::capabilities::catalog::modules::detail::YamlModuleState::new(
                        layout::home(root).join("modules.yaml"),
                    ),
                ),
            )
            .map_err(|e| format!("模块能力装配失败：{}", e))?,
        );
    system_with(
        root,
        base,
        SystemPorts {
            storage,
            provision,
            builds,
            workspace,
            modules,
        },
    )
}

/// 目的：用调用方已经装好的端口拼出系统维护能力面（主程序复用同一份存储 / 补齐 / 构建 / 工作区 / 模块）。
pub(crate) fn system_with(
    root: &Path,
    base: &BaseFacts,
    ports: SystemPorts,
) -> Result<SystemService, String> {
    // 目的：宿主事实端口在这里接上——自检里要直读宿主的那几项由 detail 实现，service 只认端口。
    let facts: Arc<dyn crate::capabilities::system::api::Facts + Send + Sync> =
        Arc::new(crate::capabilities::system::detail::HostFacts);
    // 目的：便携维护端口——用户 PATH 与围栏机器级预授的写入实现都在 detail，service 只认端口。
    let setup: Arc<dyn crate::capabilities::system::api::SetupOps + Send + Sync> = Arc::new(
        crate::capabilities::system::detail::setup::SetupHost::new(root),
    );
    Ok(SystemService::with_facts(
        SystemDeps {
            root: root.to_path_buf(),
            storage: ports.storage,
            provision: ports.provision,
            builds: ports.builds,
            workspace: ports.workspace,
            modules: ports.modules,
            toolchains: base.clone(),
            net: network_allowed(root),
        },
        facts,
        setup,
    ))
}

/// 目的：构建期的全局网络开关（与主程序同一份 settings.yaml；读不到 / 非法按拒绝）。
fn network_allowed(root: &Path) -> bool {
    std::fs::read_to_string(layout::home(root).join("settings.yaml"))
        .ok()
        .and_then(|t| {
            yaml_serde::from_str::<crate::capabilities::catalog::registry::api::AppSettings>(&t)
                .ok()
        })
        .map(|a| crate::capabilities::turn::env::api::network_allowed(&a))
        .unwrap_or(false)
}
