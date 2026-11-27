//! 目的：装配组合根——把替身装进各入站面与核心，和生产的接线同一条路。
//! 管：入站面工厂、核心装配的各种变体、agent 登记辅助。
//! 不管：替身实现（各 `*.rs`）。
//! 联动：`src/units/business/runtime/service/` 与 `docs/testing/doubles.md`。

use super::*;

/// 目的：登记处子业务的测试装配：内存登记处 + 指定模型目录 + 指定通道 + 空日志。生产里这些端口由组合根注入，测试这里用替身顶。
pub(crate) fn registry_service(
    store: InMemorySettings,
    llm: Arc<dyn crate::units::business::llm::api::Llm + Send + Sync>,
) -> Box<dyn crate::units::business::catalog::registry::api::Registry> {
    Box::new(
        crate::units::business::catalog::registry::service::RegistryService::new(
            Arc::new(store),
            llm,
            Arc::new(crate::kernel::ports::NoopLog),
        )
        .expect("内存登记处装配不应失败"),
    )
}

/// 目的：测试用的 **llm 入站面**：把通道工厂 + 模型目录装进 `LlmService`（修信封用 `NoRepair`）。与生产同一条路——组合根装 service，别人只拿 `api::Llm` 面（R12）。
pub(crate) fn test_llm(
    gateway: Arc<dyn crate::units::business::llm::ports::ChatGateway + Send + Sync>,
    catalog: Arc<FakeCatalog>,
) -> Arc<dyn crate::units::business::llm::api::Llm + Send + Sync> {
    Arc::new(crate::units::business::llm::service::LlmService::new(
        gateway,
        catalog,
        Arc::new(NoRepair),
    ))
}

/// 目的：同上，但指定信封修复器（"修信封"路径的用例用真修复器）。
pub(crate) fn test_llm_with_repair(
    repair: Arc<dyn crate::units::business::llm::ports::EnvelopeRepair + Send + Sync>,
) -> Arc<dyn crate::units::business::llm::api::Llm + Send + Sync> {
    Arc::new(crate::units::business::llm::service::LlmService::new(
        Arc::new(crate::units::business::llm::detail::fake_chat::DemoGateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        repair,
    ))
}

/// 目的：只要一个 llm 面（不关心通道与修复）的用例用它：演示通道 + 不修。
pub(crate) fn test_llm_demo() -> Arc<dyn crate::units::business::llm::api::Llm + Send + Sync> {
    test_llm_with_repair(Arc::new(NoRepair))
}

/// 目的：测试用的工作区对：布局面（`Workspace`）与版本面（`Versions`）由同一份目录替身造出，与生产同一条装配路（R12）。
/// 约束：布局方法在 `TestWork` 上直通，版本方法经 `Deref` 到 `dyn Versions`——用例照旧一处调两边。
pub(crate) struct TestWork {
    pub layout: Arc<dyn crate::units::business::workspace::api::Workspace + Send + Sync>,
    pub versions: Arc<dyn crate::units::business::versions::api::Versions + Send + Sync>,
}

impl std::ops::Deref for TestWork {
    type Target = dyn crate::units::business::versions::api::Versions + Send + Sync;

    fn deref(&self) -> &Self::Target {
        &*self.versions
    }
}

impl TestWork {
    pub(crate) fn bind(&self, name: &str, root: Option<&std::path::Path>) -> Result<(), String> {
        self.layout.bind(name, root)
    }

    pub(crate) fn ensure_layout(&self, name: &str, agents: &[String]) -> Result<(), String> {
        self.layout.ensure_layout(name, agents)
    }

    pub(crate) fn discard_layout(&self, name: &str, agents: &[String]) -> Result<(), String> {
        self.layout.discard_layout(name, agents)
    }

    pub(crate) fn work_has(&self, session: &str, name: &str) -> bool {
        self.layout.work_has(session, name)
    }

    pub(crate) fn files(
        &self,
        session: &str,
        agents: &[String],
    ) -> Result<crate::units::business::workspace::api::WorkFiles, String> {
        self.layout.files(session, agents)
    }

    pub(crate) fn usage(
        &self,
        session: &str,
        agents: &[String],
    ) -> Result<crate::units::business::workspace::api::WorkUsage, String> {
        self.layout.usage(session, agents)
    }

    pub(crate) fn roots(
        &self,
        name: &str,
        agents: &[String],
    ) -> Result<crate::units::business::workspace::api::WorkRoots, String> {
        self.layout.roots(name, agents)
    }

    pub(crate) fn is_external(&self, session: &str) -> bool {
        self.layout.is_external(session)
    }

    pub(crate) fn shared_lock_path(
        &self,
        session: &str,
    ) -> Result<Option<std::path::PathBuf>, String> {
        self.layout.shared_lock_path(session)
    }

    pub(crate) fn write_work(&self, session: &str, name: &str, bytes: &[u8]) -> Result<(), String> {
        self.layout.write_work(session, name, bytes)
    }
}

/// 目的：测试用的工作区面（布局 + 版本）：版本库默认用内存替身；要观察主副本/提交记录的用例用 `test_workspace_store`。
pub(crate) fn test_workspace(dirs: Arc<dyn Workdirs + Send + Sync>) -> TestWork {
    test_workspace_store(dirs, Arc::new(InMemoryWorkStore::new()))
}

pub(crate) fn test_workspace_store(
    dirs: Arc<dyn Workdirs + Send + Sync>,
    store: Arc<dyn WorkStore + Send + Sync>,
) -> TestWork {
    let layout: Arc<dyn crate::units::business::workspace::api::Workspace + Send + Sync> = Arc::new(
        crate::units::business::workspace::service::WorkspaceService::new(Arc::clone(&dirs)),
    );
    let versions: Arc<dyn crate::units::business::versions::api::Versions + Send + Sync> = Arc::new(
        crate::units::business::versions::service::VersionService::new(Arc::clone(&layout), store),
    );
    TestWork { layout, versions }
}

/// 目的：用指定的单文件上限装配工作区面（测试超限分支时避免造 64 MiB 数据）。
pub(crate) fn test_workspace_store_with_max(
    dirs: Arc<dyn Workdirs + Send + Sync>,
    store: Arc<dyn WorkStore + Send + Sync>,
    max: u64,
) -> TestWork {
    let layout: Arc<dyn crate::units::business::workspace::api::Workspace + Send + Sync> = Arc::new(
        crate::units::business::workspace::service::WorkspaceService::new(Arc::clone(&dirs)),
    );
    let versions: Arc<dyn crate::units::business::versions::api::Versions + Send + Sync> = Arc::new(
        crate::units::business::versions::service::VersionService::with_max_asset_bytes(
            Arc::clone(&layout),
            store,
            max,
        ),
    );
    TestWork { layout, versions }
}

impl InMemoryModuleState {
    /// 目的：空覆盖起步。
    pub(crate) fn new() -> InMemoryModuleState {
        InMemoryModuleState {
            state: Mutex::new(Default::default()),
        }
    }
}

impl crate::units::business::catalog::modules::ports::ModuleStateStore for InMemoryModuleState {
    fn load(
        &self,
    ) -> Result<crate::units::business::catalog::modules::domain::state::EnabledState, String> {
        Ok(self.state.lock().expect("锁").clone())
    }
    fn save(
        &self,
        state: &crate::units::business::catalog::modules::domain::state::EnabledState,
    ) -> Result<(), String> {
        *self.state.lock().expect("锁") = state.clone();
        Ok(())
    }
}

/// 目的：测试用的**模块入站面**——把清单来源与覆盖存储装进 `ModulesService`（与生产同一条路，R12）。
pub(crate) fn test_modules(
    source: Arc<dyn ModuleSource + Send + Sync>,
) -> Arc<dyn crate::units::business::catalog::modules::api::Modules + Send + Sync> {
    test_modules_store(source, Arc::new(InMemoryModuleState::new()))
}

/// 目的：测试用的模块入站面（指定覆盖存储；断言落盘/有效值的用例用它）。
pub(crate) fn test_modules_store(
    source: Arc<dyn ModuleSource + Send + Sync>,
    store: Arc<dyn crate::units::business::catalog::modules::ports::ModuleStateStore + Send + Sync>,
) -> Arc<dyn crate::units::business::catalog::modules::api::Modules + Send + Sync> {
    Arc::new(
        crate::units::business::catalog::modules::service::ModulesService::new(source, store)
            .expect("装模块"),
    )
}

/// 目的：登记一个 agent（测试装配用）：**校验用的模块清单由调用方取一份**交给登记处——清单归 modules，登记处只认事实（见 docs/architecture/boundaries.md 的「三分法」）。
pub(crate) fn agent_upsert(
    core: &mut Core,
    name: &str,
    modules: &[&str],
    model: &str,
    note: &str,
) -> Result<(), String> {
    agent_upsert_chara(core, name, modules, model, note, "")
}

/// 目的：登记一个带人设的 agent（测试装配用）。
pub(crate) fn agent_upsert_chara(
    core: &mut Core,
    name: &str,
    modules: &[&str],
    model: &str,
    note: &str,
    chara: &str,
) -> Result<(), String> {
    let roster = core.modules_handle().roster();
    let modules: Vec<String> = modules.iter().map(|m| m.to_string()).collect();
    core.registry_mut()
        .agent_upsert(name, &modules, model, note, chara, &roster)
}

/// 目的：注入指定常驻服务面与隐秘字段面的装配（动作面、会话回收、env 注入的端到端断言用）。
pub(crate) fn core_with_services(
    modules: Vec<Module>,
    gateway: ScriptGateway,
    runner: Arc<dyn crate::kernel::ports::ProcessRunner>,
    residents: Arc<dyn crate::units::business::residents::api::ResidentOps + Send + Sync>,
    secrets: Arc<dyn crate::units::business::catalog::secrets::api::SecretOps + Send + Sync>,
) -> Core {
    let gateway: Arc<dyn crate::units::business::llm::ports::ChatGateway + Send + Sync> =
        Arc::new(gateway);
    let llm = test_llm(
        Arc::clone(&gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    let work = test_workspace(Arc::new(InMemoryWorkspace::new()));
    let toolexec: Arc<dyn crate::units::business::tools::api::ToolExec + Send + Sync> =
        test_tools_svc_with(
            runner,
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        );
    Core::new(
        crate::tests::root_dir(),
        registry_service(InMemorySettings::new(), Arc::clone(&llm)),
        test_history(),
        work.layout.clone(),
        work.versions.clone(),
        test_vm(),
        test_policies_with(Arc::clone(&toolexec)),
        test_packages(Arc::new(InMemoryPackages::empty())),
        test_provision(),
        test_modules(Arc::new(VecSource(modules))),
        Arc::new(crate::units::business::maintenance::builds::api::NoBuilds),
        llm,
        toolexec,
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        test_host_probe(),
        residents,
        secrets,
        test_decls(),
    )
}

pub(crate) fn core_with(modules: Vec<Module>, gateway: ScriptGateway) -> Core {
    core_with_runner(modules, gateway, Arc::new(SilentRunner))
}

/// 目的：注入指定内存工作区的装配（断言 @ 文件清单取自会话 meta.agents）。
pub(crate) fn core_with_workspace(
    modules: Vec<Module>,
    gateway: ScriptGateway,
    ws: Arc<InMemoryWorkspace>,
) -> Core {
    let gateway: Arc<dyn crate::units::business::llm::ports::ChatGateway + Send + Sync> =
        Arc::new(gateway);
    let llm = test_llm(
        Arc::clone(&gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    let work = test_workspace(ws);
    let toolexec: Arc<dyn crate::units::business::tools::api::ToolExec + Send + Sync> =
        test_tools_svc_with(
            Arc::new(SilentRunner),
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        );
    Core::new(
        crate::tests::root_dir(),
        registry_service(InMemorySettings::new(), Arc::clone(&llm)),
        test_history(),
        work.layout.clone(),
        work.versions.clone(),
        test_vm(),
        test_policies_with(Arc::clone(&toolexec)),
        test_packages(Arc::new(InMemoryPackages::empty())),
        test_provision(),
        test_modules(Arc::new(VecSource(modules))),
        Arc::new(crate::units::business::maintenance::builds::api::NoBuilds),
        llm,
        toolexec,
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        test_host_probe(),
        Arc::new(crate::units::business::residents::api::NoResidents),
        Arc::new(crate::units::business::catalog::secrets::api::NoSecrets),
        test_decls(),
    )
}

/// 目的：注入指定工作区端口与**产品根**的装配（外部工作区用例：真实 FsWorkspace + 隔离产品根）。
pub(crate) fn core_with_workdirs(
    root: std::path::PathBuf,
    modules: Vec<Module>,
    gateway: ScriptGateway,
    dirs: Arc<dyn Workdirs + Send + Sync>,
) -> Core {
    let gateway: Arc<dyn crate::units::business::llm::ports::ChatGateway + Send + Sync> =
        Arc::new(gateway);
    let llm = test_llm(
        Arc::clone(&gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    let work = test_workspace(dirs);
    let toolexec: Arc<dyn crate::units::business::tools::api::ToolExec + Send + Sync> =
        test_tools_svc_with(
            Arc::new(SilentRunner),
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        );
    Core::new(
        root,
        registry_service(InMemorySettings::new(), Arc::clone(&llm)),
        test_history(),
        work.layout.clone(),
        work.versions.clone(),
        test_vm(),
        test_policies_with(Arc::clone(&toolexec)),
        test_packages(Arc::new(InMemoryPackages::empty())),
        test_provision(),
        test_modules(Arc::new(VecSource(modules))),
        Arc::new(crate::units::business::maintenance::builds::api::NoBuilds),
        llm,
        toolexec,
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        test_host_probe(),
        Arc::new(crate::units::business::residents::api::NoResidents),
        Arc::new(crate::units::business::catalog::secrets::api::NoSecrets),
        test_decls(),
    )
}

/// 目的：在**指定产品根**下造一格虚拟根（`layout::vm(root)/guest/<名>`），返回名字。
/// 约束：`crate::tests::test_guest` 只能造在当前目录（那正是 `Core` 的默认产品根）；这里给注入产品根的用例用。内容与它同形（占位磁盘镜像 + 内核 + 三平台共享客户端声明）；收尾由调用方连根删，不影响真实数据根。
pub(crate) fn test_guest_at(root: &std::path::Path, name: &str) -> String {
    let dir = crate::kernel::domain::layout::vm(root)
        .join("guest")
        .join(name);
    std::fs::create_dir_all(&dir).expect("建虚拟根目录");
    std::fs::write(dir.join("rootfs.ext4"), b"img").expect("写占位磁盘镜像");
    std::fs::write(dir.join("vmlinuz"), b"stub").expect("写占位内核");
    std::fs::write(
        dir.join("guest.yaml"),
        format!("id: {name}\nclients: [9p, virtiofs, cifs]\n"),
    )
    .expect("写 guest.yaml");
    name.to_string()
}

/// 目的：注入**产品根 + 工作区端口 + 可观察 VM 引擎**的装配（外部工作区 × 虚拟机档用例）。
/// 约束：`core_with_workdirs` 装的是本机档默认件（`test_vm()` 的引擎不可注入、探针不声明能起虚拟机），这里两样都接上：引擎用调用方给的 `FakeEngine`（`specs` 记挂载计划），探针按"本机能起虚拟机"作答。历史用**真实 FsHistory**（落 `<产品根>/session`）：删会话要真删目录，`InMemoryHistory` 给不出这条事实。
pub(crate) fn core_with_workdirs_vm(
    root: std::path::PathBuf,
    modules: Vec<Module>,
    gateway: ScriptGateway,
    dirs: Arc<dyn Workdirs + Send + Sync>,
    engine: FakeEngine,
) -> Core {
    let gateway: Arc<dyn crate::units::business::llm::ports::ChatGateway + Send + Sync> =
        Arc::new(gateway);
    let llm = test_llm(
        Arc::clone(&gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    let history: Arc<dyn crate::units::business::turn_state::api::History + Send + Sync> = Arc::new(
        crate::units::business::conversation::session::service::SessionService::new(Arc::new(
            crate::units::business::conversation::session::detail::fs_history::FsHistory::new(
                root.join("session"),
            ),
        )),
    );
    let work = test_workspace(dirs);
    let toolexec: Arc<dyn crate::units::business::tools::api::ToolExec + Send + Sync> =
        test_tools_svc_with(
            Arc::new(SilentRunner),
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        );
    let policies = test_policies_at_with(root.clone(), Arc::clone(&toolexec));
    Core::new(
        root,
        registry_service(InMemorySettings::new(), Arc::clone(&llm)),
        history,
        work.layout.clone(),
        work.versions.clone(),
        test_vm_with_engine(engine),
        policies,
        test_packages(Arc::new(InMemoryPackages::empty())),
        test_provision(),
        test_modules(Arc::new(VecSource(modules))),
        Arc::new(crate::units::business::maintenance::builds::api::NoBuilds),
        llm,
        toolexec,
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        vm_capable_host_probe(),
        Arc::new(crate::units::business::residents::api::NoResidents),
        Arc::new(crate::units::business::catalog::secrets::api::NoSecrets),
        test_decls(),
    )
}

/// 目的：注入指定内存文件系统的装配（断言内置文件工具真正落盘）。
pub(crate) fn core_with_io(
    modules: Vec<Module>,
    gateway: ScriptGateway,
    io: Arc<InMemorySysIo>,
) -> Core {
    core_with_all(
        modules,
        gateway,
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::new(InMemoryHistory::new()),
        io,
    )
}

/// 目的：注入**构建面**的装配——创建会话前补编的用例要断言"编完才建 / 编不成不建"。
/// 参数：`modules` = 清单事实；`builds` = 构建入站面（用例给真实现或替身）。
pub(crate) fn core_with_builds(
    modules: Vec<Module>,
    builds: Arc<dyn crate::units::business::maintenance::builds::api::Builds + Send + Sync>,
) -> Core {
    let gateway: Arc<dyn ChatGateway + Send + Sync> = Arc::new(gw(BTreeMap::new(), Vec::new()));
    let llm = test_llm(
        Arc::clone(&gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    let work = test_workspace(Arc::new(InMemoryWorkspace::new()));
    let toolexec: Arc<dyn crate::units::business::tools::api::ToolExec + Send + Sync> =
        test_tools_svc_with(
            Arc::new(SilentRunner),
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        );
    Core::new(
        crate::tests::root_dir(),
        registry_service(InMemorySettings::new(), Arc::clone(&llm)),
        test_history(),
        work.layout.clone(),
        work.versions.clone(),
        test_vm(),
        test_policies_with(Arc::clone(&toolexec)),
        test_packages(Arc::new(InMemoryPackages::empty())),
        test_provision(),
        test_modules(Arc::new(VecSource(modules))),
        builds,
        llm,
        toolexec,
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        test_host_probe(),
        Arc::new(crate::units::business::residents::api::NoResidents),
        Arc::new(crate::units::business::catalog::secrets::api::NoSecrets),
        test_decls(),
    )
}

pub(crate) fn core_with_runner(
    modules: Vec<Module>,
    gateway: ScriptGateway,
    runner: Arc<impl ProcessRunner + 'static>,
) -> Core {
    core_with_catalog(
        modules,
        gateway,
        runner,
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    )
}

/// 目的：指定模型目录的装配（断言编辑期密钥复用）。
pub(crate) fn core_with_catalog(
    modules: Vec<Module>,
    gateway: ScriptGateway,
    runner: Arc<impl ProcessRunner + 'static>,
    catalog: Arc<FakeCatalog>,
) -> Core {
    core_with_all(
        modules,
        gateway,
        runner,
        catalog,
        Arc::new(InMemoryHistory::new()),
        Arc::new(InMemorySysIo::new()),
    )
}

/// 目的：指定全部端口的装配（历史落盘断言用）：默认空包库。
pub(crate) fn core_with_all(
    modules: Vec<Module>,
    gateway: ScriptGateway,
    runner: Arc<impl ProcessRunner + 'static>,
    catalog: Arc<FakeCatalog>,
    history: Arc<InMemoryHistory>,
    io: Arc<InMemorySysIo>,
) -> Core {
    core_with_pkgs(
        modules,
        gateway,
        runner,
        catalog,
        history,
        io,
        Arc::new(InMemoryPackages::empty()),
    )
}

/// 目的：指定全部端口 + 运行包库的装配（断言缺包诊断与工具回执）。
pub(crate) fn core_with_pkgs(
    modules: Vec<Module>,
    gateway: ScriptGateway,
    runner: Arc<impl ProcessRunner + 'static>,
    catalog: Arc<FakeCatalog>,
    history: Arc<InMemoryHistory>,
    io: Arc<InMemorySysIo>,
    packages: Arc<InMemoryPackages>,
) -> Core {
    core_with_pkgs_probe(
        modules,
        gateway,
        runner,
        catalog,
        history,
        io,
        packages,
        test_host_probe(),
    )
}

/// 目的：指定全部端口 + 运行包库 + **宿主探测**的装配（断言本机档按生效 PATH 判运行能力）。
/// 约束：参数与 core_with_pkgs 同形，只多一个 probe；测试装配需要逐项控制端口。
#[allow(clippy::too_many_arguments)]
pub(crate) fn core_with_pkgs_probe(
    modules: Vec<Module>,
    gateway: ScriptGateway,
    runner: Arc<impl ProcessRunner + 'static>,
    catalog: Arc<FakeCatalog>,
    history: Arc<InMemoryHistory>,
    io: Arc<InMemorySysIo>,
    packages: Arc<InMemoryPackages>,
    probe: Arc<dyn crate::kernel::ports::HostProbe + Send + Sync>,
) -> Core {
    let gateway: Arc<dyn crate::units::business::llm::ports::ChatGateway + Send + Sync> =
        Arc::new(gateway);
    let llm = test_llm(Arc::clone(&gateway), catalog);
    // 测试里的宿主回退：probe 说有的基础能力就当作"围栏可用"，与生产里组合根给的事实同形。
    let mut base = crate::units::business::catalog::packages::api::BaseFacts::default();
    for id in ["python", "node", "cc"] {
        let names: &[&str] = match id {
            "python" => &["python", "python3"],
            "node" => &["node"],
            "cc" => &["cc", "gcc", "g++", "clang", "clang++"],
            _ => &[],
        };
        if names.iter().any(|n| probe.has_exe(n)) {
            base.fallback.insert(
                id.to_string(),
                crate::units::business::catalog::packages::api::HostFallback {
                    path: std::path::PathBuf::from(id),
                    fenced: true,
                    why: String::new(),
                },
            );
        }
    }
    let work = test_workspace(Arc::new(InMemoryWorkspace::new()));
    let toolexec: Arc<dyn crate::units::business::tools::api::ToolExec + Send + Sync> =
        test_tools_svc_with(runner, io, Arc::new(NoFenceHost));
    Core::new(
        crate::tests::root_dir(),
        registry_service(InMemorySettings::new(), Arc::clone(&llm)),
        test_history_of(history),
        work.layout.clone(),
        work.versions.clone(),
        test_vm(),
        test_policies_with(Arc::clone(&toolexec)),
        test_packages_with_base(packages, base),
        test_provision(),
        test_modules(Arc::new(VecSource(modules))),
        Arc::new(crate::units::business::maintenance::builds::api::NoBuilds),
        llm,
        toolexec,
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        probe,
        Arc::new(crate::units::business::residents::api::NoResidents),
        Arc::new(crate::units::business::catalog::secrets::api::NoSecrets),
        test_decls(),
    )
}

/// 目的：用**指定登记处**装配（断言"全局设置是流式的上限、预算全局通用"这类判据）。
pub(crate) fn core_with_settings(store: InMemorySettings) -> Core {
    core_with_settings_modules(store, Vec::new())
}

/// 目的：同上设置装配，但指定模块清单（建带模块的会话时用）。
pub(crate) fn core_with_settings_modules(store: InMemorySettings, modules: Vec<Module>) -> Core {
    let gateway: Arc<dyn crate::units::business::llm::ports::ChatGateway + Send + Sync> =
        Arc::new(gw(BTreeMap::new(), Vec::new()));
    let llm = test_llm(
        Arc::clone(&gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    let work = test_workspace(Arc::new(InMemoryWorkspace::new()));
    let toolexec: Arc<dyn crate::units::business::tools::api::ToolExec + Send + Sync> =
        test_tools_svc_with(
            Arc::new(SilentRunner),
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        );
    Core::new(
        crate::tests::root_dir(),
        registry_service(store, Arc::clone(&llm)),
        test_history(),
        work.layout.clone(),
        work.versions.clone(),
        test_vm(),
        test_policies_with(Arc::clone(&toolexec)),
        test_packages(Arc::new(InMemoryPackages::empty())),
        test_provision(),
        test_modules(Arc::new(VecSource(modules))),
        Arc::new(crate::units::business::maintenance::builds::api::NoBuilds),
        llm,
        toolexec,
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        test_host_probe(),
        Arc::new(crate::units::business::residents::api::NoResidents),
        Arc::new(crate::units::business::catalog::secrets::api::NoSecrets),
        test_decls(),
    )
}

pub(crate) fn gw(member: BTreeMap<String, Vec<String>>, core: Vec<String>) -> ScriptGateway {
    ScriptGateway {
        member,
        core: Arc::new(Mutex::new(core)),
        usage: None,
    }
}

/// 目的：指定任意网关 + 指定内存文件系统的装配（既换通道又要断言落盘的用例用它）。
pub(crate) fn core_with_io_gateway(
    modules: Vec<Module>,
    gateway: impl ChatGateway + Send + Sync + 'static,
    io: Arc<InMemorySysIo>,
) -> Core {
    let gateway: Arc<dyn crate::units::business::llm::ports::ChatGateway + Send + Sync> =
        Arc::new(gateway);
    let llm = test_llm(
        Arc::clone(&gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    let work = test_workspace(Arc::new(InMemoryWorkspace::new()));
    let toolexec: Arc<dyn crate::units::business::tools::api::ToolExec + Send + Sync> =
        test_tools_svc_with(Arc::new(SilentRunner), io, Arc::new(NoFenceHost));
    Core::new(
        crate::tests::root_dir(),
        registry_service(InMemorySettings::new(), Arc::clone(&llm)),
        test_history(),
        work.layout.clone(),
        work.versions.clone(),
        test_vm(),
        test_policies_with(Arc::clone(&toolexec)),
        test_packages(Arc::new(InMemoryPackages::empty())),
        test_provision(),
        test_modules(Arc::new(VecSource(modules))),
        Arc::new(crate::units::business::maintenance::builds::api::NoBuilds),
        llm,
        toolexec,
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        test_host_probe(),
        Arc::new(crate::units::business::residents::api::NoResidents),
        Arc::new(crate::units::business::catalog::secrets::api::NoSecrets),
        test_decls(),
    )
}

/// 目的：指定任意网关的装配（入站契约测试用：需要自定义时序的通道）。
pub(crate) fn core_with_gateway(
    modules: Vec<Module>,
    gateway: impl ChatGateway + Send + Sync + 'static,
) -> Core {
    core_with_gateway_probe(modules, gateway, test_host_probe())
}

/// 目的：同 core_with_gateway，但注入指定的宿主探测（机器能力/依赖在场与否要可控的用例用它）。
pub(crate) fn core_with_gateway_probe(
    modules: Vec<Module>,
    gateway: impl ChatGateway + Send + Sync + 'static,
    probe: Arc<dyn crate::kernel::ports::HostProbe + Send + Sync>,
) -> Core {
    let gateway: Arc<dyn crate::units::business::llm::ports::ChatGateway + Send + Sync> =
        Arc::new(gateway);
    let llm = test_llm(
        Arc::clone(&gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    let work = test_workspace(Arc::new(InMemoryWorkspace::new()));
    let toolexec: Arc<dyn crate::units::business::tools::api::ToolExec + Send + Sync> =
        test_tools_svc_with(
            Arc::new(SilentRunner),
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        );
    Core::new(
        crate::tests::root_dir(),
        registry_service(InMemorySettings::new(), Arc::clone(&llm)),
        test_history(),
        work.layout.clone(),
        work.versions.clone(),
        test_vm(),
        test_policies_with(Arc::clone(&toolexec)),
        test_packages(Arc::new(InMemoryPackages::empty())),
        test_provision(),
        test_modules(Arc::new(VecSource(modules))),
        Arc::new(crate::units::business::maintenance::builds::api::NoBuilds),
        llm,
        toolexec,
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        probe,
        Arc::new(crate::units::business::residents::api::NoResidents),
        Arc::new(crate::units::business::catalog::secrets::api::NoSecrets),
        test_decls(),
    )
}
