//! 目的：组合根的共用装配——把产品根变成入站面，供启动程序、探针与用例复用。
//! 管：各单元的端口选型、适配器构造与依赖顺序；同一份入站面只在这里装一次。
//! 不管：业务用例、参数解析、回执打印（在 main / presentation/cli/boot.rs）；任何业务语义。
//! 联动：消费者是 `src/main.rs`、`src/presentation/cli/boot.rs` 与测试；契约见 ARCHITECTURE.md 的「根」。

use crate::kernel::api::layout;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::units::catalog::packages::api::BaseFacts;
use crate::units::maintenance::builds::service::BuildsService;
use crate::units::maintenance::storage::api::StoragePolicy;
use crate::units::maintenance::storage::service::StorageService;
use crate::units::provision::api::Provision;
use crate::units::provision::detail::fetch::HttpFetch;
use crate::units::provision::detail::guest_image::Ext4Images;
use crate::units::provision::detail::host::RealHost;
use crate::units::provision::detail::install::HostInstallers;
use crate::units::provision::detail::ledger::FileLedger;
use crate::units::provision::detail::procs::BookProcs;
use crate::units::provision::ports::Fetcher;
use crate::units::provision::service::{ProvisionDeps, ProvisionService};
use crate::units::system::api::FencePolicy;
use crate::units::system::service::{SystemDeps, SystemService};
use crate::units::vm::api::Vm;

/// 目的：一次性取数适配器——补齐与探针共用同一组超时（连接 15s，整体 7200s）。
fn fetch() -> Arc<dyn Fetcher> {
    Arc::new(HttpFetch::new(15, 7200))
}

/// 目的：补齐单元的端口组合——根只给产品根与取数端口，其余适配器在这里落成。
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

/// 目的：不带 vm 的补齐入站面（只查状态、只装不需要 guest 的条目）。
/// 约束：guest-runtime 会被如实拒绝；需要时用 `provision_with_vm`。
pub(crate) fn provision(root: &Path) -> ProvisionService {
    ProvisionService::new(root, provision_deps(root, fetch()))
}

/// 目的：带 vm 的补齐入站面；guest-runtime 编排需要它。
pub(crate) fn provision_with_vm(root: &Path, vm: Arc<dyn Vm + Send + Sync>) -> ProvisionService {
    ProvisionService::with_vm(root, provision_deps(root, fetch()), vm)
}

/// 目的：带供给 vm 的补齐入站面——vm 只持内部 `none` 共享档，进程台账按产品根现场打开。
pub(crate) fn provision_with_guest_vm(root: &Path) -> ProvisionService {
    provision_with_vm(
        root,
        provision_vm(crate::kernel::detail::procs::ProcessBook::open(
            &layout::state(root),
        )),
    )
}

/// 目的：供给编排专用的 vm 入站面：只持内部 `none` 共享档（只经 QGA，不挂载、不建可写层）。
/// 约束：与会话 vm 分开装配——`none` 不进用户候选链，避免 `clients` 为空的 guest 被静默当成无共享。
pub(crate) fn provision_vm(
    book: Arc<crate::kernel::detail::procs::ProcessBook>,
) -> Arc<dyn Vm + Send + Sync> {
    provision_vm_with(Arc::new(
        crate::units::vm::detail::QemuEngine::new().with_book(book),
    ))
}

/// 目的：同上，但允许注入引擎替身（回归测试用）；生产入口只用 `provision_vm`。
pub(crate) fn provision_vm_with(
    engine: Arc<dyn crate::units::vm::ports::VmEngine + Send + Sync>,
) -> Arc<dyn Vm + Send + Sync> {
    Arc::new(crate::units::vm::service::VmService::new(
        engine,
        vec![Arc::new(crate::units::vm::detail::NoneShare::new())],
    ))
}

/// 目的：登记处读写面（`data/.home` 这块盘归 `home` 单元）——组合根只装一次，登记处 / 隐秘字段 / 模块覆盖共用。
pub(crate) fn home_docs(root: &Path) -> Arc<dyn crate::units::home::api::Home + Send + Sync> {
    Arc::new(crate::units::home::service::HomeService::new(Box::new(
        crate::units::home::detail::FsHome::new(layout::home(root)),
    )))
}

/// 目的：会话盘面（`data/session` 这块盘归 `sessions` 单元）——组合根只在这里装一次，转录与共享区适配器都从它取根。
pub(crate) fn sessions(root: &Path) -> Arc<dyn crate::units::sessions::api::Sessions + Send + Sync> {
    Arc::new(crate::units::sessions::service::SessionsService::new(
        layout::session(root),
        Box::new(crate::units::sessions::detail::FsSessionDirs::new()),
    ))
}

/// 目的：按 settings.yaml 取存储策略；读不到用默认。
fn storage_policy(root: &Path) -> StoragePolicy {
    read_settings(root)
        .map(|a| StoragePolicy::from(&a.storage))
        .unwrap_or_default()
}

/// 目的：读一份基本设置（读不到 / 非法 = None）——读经 `home` 单元，组合根不自己碰文件。
fn read_settings(root: &Path) -> Option<crate::units::catalog::registry::api::AppSettings> {
    let text = home_docs(root)
        .read(crate::units::home::api::Doc::Settings)
        .ok()??;
    yaml_serde::from_str(&text).ok()
}

/// 目的：四份登记处 yaml 的读取面——组合根按产品根装，启动与排障都走这一份。
pub(crate) fn settings_store(
    root: &Path,
) -> crate::units::catalog::registry::detail::yaml_settings::YamlSettingsStore {
    crate::units::catalog::registry::detail::yaml_settings::YamlSettingsStore::new(home_docs(root))
}

/// 目的：用给定补齐面装一份存储子业务（不另装核心）。
/// 参数：`log` = 日志端口；`active_log` = 当前进程正在写的日志路径（没有就 None），盘点时标为受保护。
pub(crate) fn storage(
    root: &Path,
    provision: Arc<dyn Provision + Send + Sync>,
    policy: StoragePolicy,
    log: Arc<dyn crate::kernel::ports::Log + Send + Sync>,
    active_log: Option<PathBuf>,
) -> StorageService {
    StorageService::new(
        provision,
        Arc::new(
            crate::units::maintenance::storage::detail::survey::DiskSurvey::new(
                root.to_path_buf(),
                active_log,
            ),
        ),
        Arc::new(
            crate::units::maintenance::storage::detail::reclaim::DiskReclaim::new(
                root.to_path_buf(),
            ),
        ),
        Arc::new(crate::units::maintenance::storage::detail::guests::ProcessGuests::new(root)),
        log,
        policy,
    )
}

/// 目的：虚拟机档构建用的 vm 入站面：**只持挂载式**共享档（virtiofs / 9p / smb）。
/// 约束：模块目录是读写挂进去的，产物才能落回宿主那份——构建只持挂载式用户档；
///   `copy` 是开发/应急通道，不装配；`none` 只给供给编排。
pub(crate) fn build_vm(root: &Path) -> Arc<dyn Vm + Send + Sync> {
    let shares: Vec<Arc<dyn crate::units::vm::ports::ShareBackend + Send + Sync>> = {
        let book = crate::kernel::detail::procs::ProcessBook::open(&layout::state(root));
        vec![
            Arc::new(crate::units::vm::detail::VirtioFsShare::new().with_book(Arc::clone(&book))),
            Arc::new(crate::units::vm::detail::NinePShare::new()),
            Arc::new(crate::units::vm::detail::SmbShare::new().with_book(book)),
        ]
    };
    Arc::new(crate::units::vm::service::VmService::new(
        Arc::new(crate::units::vm::detail::QemuEngine::with_overlay_dir(
            layout::vm(root).join("overlays"),
        )),
        shares,
    ))
}

/// 目的：构建用的执行器——与外层工具执行同一套守门进程机制（围栏在它里面装）。
pub(crate) fn build_runner(
    root: &Path,
) -> Result<crate::kernel::detail::process::ProcTools, String> {
    Ok(crate::kernel::detail::process::ProcTools::new(
        std::env::current_exe().unwrap_or_default(),
        root.to_path_buf(),
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    ))
}

/// 目的：命令行构建用的入站面——包库、虚拟机、补齐、执行器与基础工具链安装根按产品根现场装配。
pub(crate) fn build_service(root: &Path) -> Result<BuildsService, String> {
    let packages: Arc<dyn crate::units::catalog::packages::api::Packages + Send + Sync> = Arc::new(
        crate::units::catalog::packages::service::PackagesService::with_base(
            Arc::new(crate::units::catalog::packages::detail::FsPackages::new(
                layout::runtimes(root),
            )),
            crate::entry::toolchains::base_facts(root),
        ),
    );
    let provision: Arc<dyn Provision + Send + Sync> = Arc::new(provision(root));
    let storage: Arc<dyn crate::units::maintenance::storage::api::Storage + Send + Sync> =
        Arc::new(storage(
            root,
            Arc::clone(&provision),
            StoragePolicy::default(),
            Arc::new(crate::kernel::ports::NoopLog),
            None,
        ));
    let runner: Arc<dyn crate::kernel::ports::ProcessRunner + Send + Sync> =
        Arc::new(build_runner(root)?);
    let store = Arc::new(
        crate::units::maintenance::builds::detail::fingerprint::BuildStore::new(root.to_path_buf()),
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
            crate::units::maintenance::builds::detail::cache::CacheOnDisk::new(root.to_path_buf()),
        ),
        runner,
        Arc::new(crate::units::tools::service::fence::ToolFenceGate),
        Arc::new(crate::kernel::detail::confine::FenceHostAdapter::new(
            root.to_path_buf(),
        )),
        Arc::new(crate::kernel::detail::host_probe::HostProbeAdapter),
        Arc::new(crate::kernel::ports::NoopLog),
        crate::entry::toolchains::active_roots(root),
    ))
}
/// 目的：隐秘字段入站面的装配：清单事实 + 值存储端口；加载失败如实拒绝（不静默空表）。
/// 约束：值只经存储端口落盘；本函数不读环境变量、不碰命令行、不拼错误文案。
pub(crate) fn secrets_with(
    modules: Arc<dyn crate::units::catalog::modules::api::Modules + Send + Sync>,
    store: Arc<dyn crate::units::catalog::secrets::ports::SecretStore + Send + Sync>,
) -> Result<Arc<dyn crate::units::catalog::secrets::api::SecretOps + Send + Sync>, String> {
    Ok(Arc::new(
        crate::units::catalog::secrets::service::SecretsService::new(modules, store)?,
    ))
}

/// 目的：隐秘字段入站面（值落 `.home/secrets.yaml`）——主程序用它，路径选型只在这里定一次。
pub(crate) fn secrets(
    root: &Path,
    modules: Arc<dyn crate::units::catalog::modules::api::Modules + Send + Sync>,
) -> Result<Arc<dyn crate::units::catalog::secrets::api::SecretOps + Send + Sync>, String> {
    secrets_with(
        modules,
        Arc::new(crate::units::catalog::secrets::detail::YamlSecrets::new(home_docs(
            root,
        ))),
    )
}

/// 目的：常驻服务入站面的装配：适配器注册表 + 模块清单 + 隐秘字段面；生命周期与协议全在该单元内。
/// 约束：协议适配器只由组合根构造（核心不认识任何协议）；网络开关与运行包事实由调用方用它的 `with_*` 补。
pub(crate) fn residents(
    adapters: Vec<Arc<dyn crate::units::residents::ports::ServiceAdapter + Send + Sync>>,
    modules: Arc<dyn crate::units::catalog::modules::api::Modules + Send + Sync>,
    secrets: Arc<dyn crate::units::catalog::secrets::api::SecretOps + Send + Sync>,
) -> crate::units::residents::service::ResidentsService {
    crate::units::residents::service::ResidentsService::new(adapters, modules, secrets)
}

/// 目的：系统维护入站面的端口：组合根里已经装好的同一份实例，启动相命令直接复用。
/// 约束：一处进程只要一套存储 / 补齐 / 构建——另装一套会让后台作业与日志互相看不见。
pub(crate) struct SystemPorts {
    pub storage: Arc<dyn crate::units::maintenance::storage::api::Storage + Send + Sync>,
    pub provision: Arc<dyn Provision + Send + Sync>,
    pub builds: Arc<dyn crate::units::maintenance::builds::api::Builds + Send + Sync>,
    pub workspace: Arc<dyn crate::units::workspace::api::Workspace + Send + Sync>,
    pub modules: Arc<dyn crate::units::catalog::modules::api::Modules + Send + Sync>,
}

/// 目的：启动相命令用的系统维护入站面——存储 / 补齐 / 构建 / 工作区 / 模块与工具链事实按产品根现场装。
/// 约束：只在这里装一次；单元自己不构造别的单元的适配器（依赖由本函数注入）。
pub(crate) fn system(root: &Path, base: &BaseFacts) -> Result<SystemService, String> {
    let provision: Arc<dyn Provision + Send + Sync> = Arc::new(provision_with_guest_vm(root));
    let storage: Arc<dyn crate::units::maintenance::storage::api::Storage + Send + Sync> =
        Arc::new(storage(
            root,
            Arc::clone(&provision),
            storage_policy(root),
            Arc::new(crate::kernel::ports::NoopLog),
            None,
        ));
    let builds: Arc<dyn crate::units::maintenance::builds::api::Builds + Send + Sync> =
        Arc::new(build_service(root)?);
    let workspace: Arc<dyn crate::units::workspace::api::Workspace + Send + Sync> =
        Arc::new(crate::units::workspace::service::WorkspaceService::new(
            Arc::new(crate::units::workspace::detail::fs_workspace::FsWorkspace::new(
                sessions(root).root(),
            )),
            Arc::new(crate::units::workspace::detail::fs_workstore::FsWorkStore::new()),
        ));
    let modules: Arc<dyn crate::units::catalog::modules::api::Modules + Send + Sync> = Arc::new(
        crate::units::catalog::modules::service::ModulesService::new(
            Arc::new(crate::units::catalog::modules::detail::FsModules::new(
                layout::modules_src(root),
                crate::units::tools::api::names(),
                // 启动相没有工具单元可问：命令词直接取编译进来的命令表投影；
                // 名字表的 modules 块由核心装配在装载工具单元后再并入（启动相不派发模块动作）。
                crate::units::tools::api::CommandBook::builtin()
                    .map(|b| crate::units::tools::api::OpBook::from_commands(b).ids())
                    .unwrap_or_default(),
            )),
            Arc::new(
                crate::units::catalog::modules::detail::YamlModuleState::new(home_docs(root)),
            ),
        )
        .map_err(|e| format!("模块清单装配失败：{}", e))?,
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

/// 目的：预授成功后把"允许写 ACL"记进设置——只改 `sandbox.policy.windows.write_acl` 一项，走登记处同一套校验与写盘。
struct WriteAclPolicy {
    root: PathBuf,
}

impl FencePolicy for WriteAclPolicy {
    fn record_authorization(&self) -> Result<String, String> {
        crate::units::catalog::registry::service::grant_write_acl(&settings_store(&self.root))?;
        Ok("sandbox.policy.windows.write_acl = true（已记进设置）".to_string())
    }
}

/// 目的：把设置写入面接到便携维护端口上：`--setup fence` 预授成功后顺带记下授权。
pub(crate) fn write_acl_policy(root: &Path) -> Arc<dyn FencePolicy> {
    Arc::new(WriteAclPolicy {
        root: root.to_path_buf(),
    })
}

/// 目的：用调用方已经装好的端口拼出系统维护入站面（主程序复用同一份存储 / 补齐 / 构建 / 工作区 / 模块）。
pub(crate) fn system_with(
    root: &Path,
    base: &BaseFacts,
    ports: SystemPorts,
) -> Result<SystemService, String> {
    // 目的：宿主事实端口在这里接上——自检里要直读宿主的那几项由 detail 实现，service 只认端口。
    let facts: Arc<dyn crate::units::system::api::Facts + Send + Sync> =
        Arc::new(crate::units::system::detail::HostFacts);
    // 目的：便携维护端口——用户 PATH 与围栏机器级预授的写入实现都在 detail，service 只认端口。
    let setup: Arc<dyn crate::units::system::api::SetupOps + Send + Sync> = Arc::new(
        crate::units::system::detail::setup::SetupHost::new(root).with_policy(
            write_acl_policy(root),
            crate::kernel::api::platform::HAS_EXTERNAL_ACL_STEP,
        ),
    );
    Ok(SystemService::with_facts(
        SystemDeps {
            root: root.to_path_buf(),
            sessions: sessions(root),
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
    read_settings(root)
        .map(|a| crate::units::turn::env::api::network_allowed(&a))
        .unwrap_or(false)
}
