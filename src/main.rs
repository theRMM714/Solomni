//! 目的：Solomni 组合根与进程入口。
//! 管：启动序列——参数 / 根目录 / 工具链 / 日志 / 授权对账；唯一允许 new 具体适配器并装配各能力（含 conductor）的地方。
//! 不管：参数表与分派（entry::args）、各适配器与能力的内部实现、装配后的业务行为（在能力与呈现层）。
//! 联动：装配结果交给呈现层（presentation::cli / presentation::web），conductor 不知道后两者存在；测试构建外露启动门助手（entry::startup）。

mod capabilities;
mod entry;
mod guard;
mod kernel;
mod presentation;

#[cfg(test)]
mod tests;

// 测试点名的启动门助手：只在测试构建里外露，入口本身走 `entry::startup` 的路径。
#[cfg(test)]
pub(crate) use entry::startup::{
    FenceReport, GateOutcome, MissingCause, MissingPart, ask_yes_no, authorize,
    lower_enforce_to_best_effort, maybe_onboard_windows, missing_parts, run_gate,
    write_acl_setting,
};

use capabilities::registry::api::Registry as _;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // 参数表与分派在 entry::args：入口只决定"继续启动"还是"就此退出"。
    if let Some(code) = entry::args::before_root(&args) {
        std::process::exit(code);
    }
    // 旗语只有一个解析器：写错就报错并给最近建议，不带着错误参数继续启动。
    let flags = match presentation::cli::argv::parse(&args[1..]) {
        Ok(flags) => flags,
        Err(e) => {
            eprintln!("[错误] {}", e);
            std::process::exit(2);
        }
    };
    if flags.help {
        print!("{}", presentation::cli::catalog::help_text(flags.all));
        return;
    }
    if flags.version {
        println!("{}", presentation::cli::catalog::version_text());
        return;
    }
    // 启动形态：无参数 = CLI（默认）；--web = Web 转录中心。
    let web = flags.web;
    let raw_root = flags
        .root
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    // 产品根规范化成**干净的绝对路径**：提示词里给 AI 的、以及各适配器给出的根都是它。
    let (root, root_note) = entry::root::resolve_root(&raw_root);
    let home = root.join(".home");
    // 基础工具链：只写项目内 .tools；先应用 PATH，再跑诊断 / 构建 / 会话，模块工具与 build 因此都优先项目版本。
    let toolchains = entry::toolchains::apply(&root);
    // 这些是人读的启动事实；--doctor / --provision 的 stdout 是机器可读 JSON，所以走 stderr。
    // 基础 profile 缺失时**每次启动**都报：项目内 / 宿主回退 / 宿主也没有，并给补全命令。
    let base = entry::toolchains::base_facts(&root);
    for line in entry::toolchains::report_lines(&base) {
        eprintln!("[工具链] {}", line);
    }
    for w in &toolchains.warnings {
        eprintln!("[工具链] {}", w);
    }

    if let Some(code) = entry::args::after_root(&root, &args) {
        std::process::exit(code);
    }

    // 组合根：唯一允许 new 具体适配器的地方（依赖注入）。
    let log: std::sync::Arc<dyn kernel::ports::Log + Send + Sync> =
        match kernel::detail::FileLog::new(&root, "Solomni 运行日志") {
            Ok(l) => std::sync::Arc::new(l),
            Err(e) => {
                eprintln!("[日志系统异常] {}（进程继续，日志降级为 stderr）", e);
                std::sync::Arc::new(kernel::ports::NoopLog)
            }
        };
    if let Some(note) = &root_note {
        eprintln!("[根目录] {}", note);
        log.warn("main::root", note);
    }
    if !toolchains.active.is_empty() {
        log.info(
            "main::toolchains",
            &format!(
                "项目内工具链：{}",
                toolchains
                    .active
                    .iter()
                    .map(|(id, version)| format!("{} {}", id, version))
                    .collect::<Vec<_>>()
                    .join("、")
            ),
        );
    }
    for w in &toolchains.warnings {
        log.warn("main::toolchains", w);
    }
    // 启动对账（装配期，会话尚未开工）：按台账回收上一次运行被杀/崩溃留下的陈旧授权。
    // 只回收归属明确已死的条目；无法判定的报告后跳过；失败如实报、台账保留供重试（--fence-reconcile / --fence-clean）。
    {
        let rep = kernel::detail::confine::reconcile(&home);
        if rep.has_activity() {
            println!("[围栏] 启动对账：{}", rep.summary());
        }
        if !rep.errors.is_empty() {
            log.warn("main::fence_reconcile", &rep.summary());
        }
    }
    // 长驻子进程的账：按它回收上一次留下的孤儿（Unix 确认命令行标记才动手；Windows 有作业对象兜底）。
    let book = kernel::detail::procs::ProcessBook::open(&home);
    {
        let rep = book.reconcile();
        if rep.touched() {
            println!(
                "[进程] 启动对账：清掉 {} 条已不在的登记，收掉 {} 个孤儿，仍有 {} 个在跑",
                rep.dropped,
                rep.killed,
                rep.alive.len()
            );
        }
    }
    // 围栏能力如实告知（不强于实际：机制缺什么就说缺什么）。
    let fence_cap = kernel::detail::confine::capability();
    log.info(
        "main::fence",
        &format!(
            "围栏能力：文件系统={} 断网={} 进程树={}；{}",
            fence_cap.fs, fence_cap.net, fence_cap.tree, fence_cap.note
        ),
    );
    println!("[围栏] {}", fence_cap.note);
    let store = capabilities::registry::detail::yaml_settings::YamlSettingsStore::new(
        root.join(".home").join("providers.yaml"),
        root.join(".home").join("models.yaml"),
        root.join(".home").join("settings.yaml"),
        root.join(".home").join("agents.yaml"),
    );
    let history = capabilities::session::detail::fs_history::FsHistory::new(root.join("session"));
    let workspace = capabilities::workspace::detail::FsWorkspace::new(root.join("session"));
    let workstore = capabilities::workspace::detail::FsWorkStore::new();
    // 保留名表（内置工具名）由**组合根**问一次工具能力后交进去：清单校验归 workspace，
    // 名字空间归 tools，两边不互相依赖。
    let source = capabilities::modules::detail::FsModules::new(
        root.join("modules"),
        capabilities::tools::api::names(),
    );
    // 运行包能力：依赖文件夹 runtimes/（一个包 = 一个文件夹 + package.yaml）。
    let packages: Arc<dyn capabilities::packages::api::Packages + Send + Sync> =
        Arc::new(capabilities::packages::service::PackagesService::with_base(
            Arc::new(capabilities::packages::detail::FsPackages::new(
                root.join("runtimes"),
            )),
            base.clone(),
        ));
    // 端点记忆：谁先通了就固定谁，后续会话不再反复探测候选。
    let memo = capabilities::llm::detail::endpoint::memo_new();
    // **llm 能力**：三个出站端口（通道工厂 / 模型目录 / 信封修复）只由它的 service 持有（R12）；
    // 别人（登记处、协调业务、协作会话）只拿它的 `api::Llm` 面。
    let gateway: Arc<dyn capabilities::llm::ports::ChatGateway + Send + Sync> = Arc::new(
        capabilities::llm::detail::HttpGateway::with_log(Arc::clone(&log), Arc::clone(&memo)),
    );
    let catalog: Arc<dyn capabilities::llm::ports::ModelCatalog + Send + Sync> = Arc::new(
        capabilities::llm::detail::HttpModelCatalog::with_log(Arc::clone(&log), Arc::clone(&memo)),
    );
    let llm: Arc<dyn capabilities::llm::api::Llm + Send + Sync> =
        Arc::new(capabilities::llm::service::LlmService::new(
            gateway,
            catalog,
            Arc::new(capabilities::llm::detail::UnambiguousRepair),
        ));
    // **登记处能力**：四份 yaml 的状态与用例都在它里面（conductor 只按 `Registry` 用它，看不见字段）。
    let mut registry = match capabilities::registry::service::RegistryService::new(
        Arc::new(store),
        Arc::clone(&llm),
        Arc::clone(&log),
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[装配失败] {}", e);
            std::process::exit(1);
        }
    };
    // 依赖补齐后的**自动发现**：设置里没登记时，把装到项目内的可运行包直接当登记值（显式登记优先）。
    // 只填内存、不写用户的 settings.yaml；同进程内新装的包要下次启动才生效（Web 的 setup 作业即属此列）。
    let installed =
        capabilities::provision::api::Provision::status(&entry::assembly::provision(&root))
            .unwrap_or_default();
    // 清单里的 probe 是**相对产品根**的写法：这里拼成绝对路径再当登记值用（否则只在 CWD = 产品根时才成立）。
    let picked = |prefix: &str| {
        installed
            .iter()
            .find(|d| d.present && d.id.starts_with(prefix))
            .map(|d| root.join(&d.probe).to_string_lossy().into_owned())
    };
    registry.apply_discovered(
        picked("vm.qemu."),
        picked("vm.virtiofsd."),
        picked("vm.smbd."),
    );
    // 全局沙箱模式：要求 vs 本机能力 vs 本机授权。授权只影响 Windows 的容器围栏；
    // 环境变量优先于设置，且只读一次——预检、启动报告与工具进程必须用同一个值。
    let sandbox_cfg = registry.app_settings().sandbox.clone();
    let fence_write_env = std::env::var("SOLOMNI_FENCE_WRITE")
        .ok()
        .filter(|v| v == "1" || v == "0");
    let authorized = entry::startup::authorize(
        fence_write_env.as_deref(),
        sandbox_cfg.policy.windows.write_acl,
    );
    let requirements = kernel::api::SandboxRequirements {
        filesystem: sandbox_cfg.policy.enforce.filesystem
            == capabilities::registry::api::SandboxEnforceLevel::Require,
        network: sandbox_cfg.policy.enforce.network
            == capabilities::registry::api::SandboxEnforceLevel::Require
            && !sandbox_cfg.policy.network,
        process_tree: sandbox_cfg.policy.enforce.process_tree
            == capabilities::registry::api::SandboxEnforceLevel::Require,
        env_whitelist: sandbox_cfg.policy.enforce.env_whitelist
            == capabilities::registry::api::SandboxEnforceLevel::Require,
    };
    let sandbox_mode = Arc::new(kernel::api::SandboxMode::new(
        requirements,
        fence_cap.fs && (!cfg!(windows) || authorized),
        fence_cap.net && (!cfg!(windows) || authorized),
        fence_cap.tree,
        true,
        sandbox_cfg.policy.on_unavailable == capabilities::registry::api::SandboxUnavailable::Ask,
    ));
    // 授权开关：工具进程与启动报告共用这一份（环境变量只在这里生效一次）。
    let write_allowed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(authorized));
    // 先报事实，再决定要不要问：用户必须先看清是“未授权”还是“本机不支持”。
    let fence_report =
        entry::startup::FenceReport::new(fence_cap.clone(), authorized, Arc::clone(&sandbox_mode));
    entry::startup::print_fence_report(&fence_report, sandbox_cfg.policy.host_read_roots.len());

    let missing = entry::startup::missing_parts(&requirements, &fence_cap, authorized);
    for part in &missing {
        println!("[沙箱] {}：{}", part.name, part.cause.label());
    }

    // 测试/CI 的进程级注入：只认 '1' 降级、'0' 严格；其它值不作为覆盖。
    let forced_degraded = std::env::var("SOLOMNI_SANDBOX_DEGRADED")
        .ok()
        .filter(|v| v == "1" || v == "0");
    match forced_degraded.as_deref() {
        Some("1") if sandbox_mode.ask_allowed => sandbox_mode.set_degraded(true),
        Some("0") => sandbox_mode.set_degraded(false),
        _ => {}
    }

    let tty = std::io::stdin().is_terminal();
    // 提问用完就把 stdin 锁放掉：后面的 CLI 还要从同一个 stdin 读命令。
    {
        let mut stdin = std::io::stdin().lock();
        let mut stdout = std::io::stdout();
        if !missing.is_empty() {
            // 首次启动：没有 settings.yaml 时先给一次授权机会（只 Windows、只 TTY、无环境变量覆盖）。
            match entry::startup::maybe_onboard_windows(
                &missing,
                home.join("settings.yaml").exists(),
                tty,
                fence_write_env.as_deref(),
                forced_degraded.as_deref(),
                &fence_cap,
                &mut registry,
                &mut stdin,
                &mut stdout,
            ) {
                Ok(true) => {
                    println!(
                        "[沙箱] 已写入 .home/settings.yaml；启动器会把本进程整体重启（直接运行二进制时请重新启动）。"
                    );
                    std::process::exit(presentation::web::RESTART_EXIT_CODE);
                }
                Ok(false) => {}
                Err(e) => {
                    eprintln!("[沙箱] 写入授权失败：{}", e);
                    std::process::exit(1);
                }
            }
        }

        if !missing.is_empty() && !sandbox_mode.degraded() {
            match entry::startup::run_gate(
                &missing,
                sandbox_mode.ask_allowed,
                tty,
                forced_degraded.as_deref(),
                fence_write_env.as_deref(),
                &mut registry,
                &mut stdin,
                &mut stdout,
            ) {
                entry::startup::GateOutcome::Proceed => sandbox_mode.set_degraded(true),
                entry::startup::GateOutcome::Restart => {
                    println!(
                        "[沙箱] 已写入 .home/settings.yaml；启动器会把本进程整体重启（直接运行二进制时请重新启动）。"
                    );
                    std::process::exit(presentation::web::RESTART_EXIT_CODE);
                }
                entry::startup::GateOutcome::Exit(code) => std::process::exit(code),
            }
        }
    }
    if sandbox_mode.degraded() {
        println!("[沙箱] 本进程已降级：缺失的强制项按如实降级执行；重启后重新评估");
    }
    let prompt_source =
        capabilities::prompt::detail::yaml_prompts::YamlPrompts::new(root.join("prompts"));
    // **提示词册能力**：册子只在这里装载一次、也只被它持有；conductor 与工具执行
    // （回执里的那些收尾标记）要的"段"都经它的能力面拿——不再各存一份拷贝。
    let prompt = match capabilities::prompt::service::load(&prompt_source) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[装配失败] {}", e);
            std::process::exit(1);
        }
    };
    // 工具总表与角色表必须自洽（悬空引用 / 缺能力都是装配错误）：装配期就挡下，不拖到运行期。
    // 三张表归**工具能力**（加载器在它自己的 detail 里；装载后由它的 service 持有）。
    let systools_source =
        capabilities::tools::detail::yaml_systools::YamlSystools::new(root.join("systools"));

    // 进程机制共用的文案（一次执行与长驻会话同一份）。
    let process_texts: std::sync::Arc<dyn kernel::ports::ProcessTexts> =
        std::sync::Arc::new(kernel::detail::process::CodeProcessTexts);
    // 工具执行：外层拉起的守门进程就是本程序自己（围栏在它里面装）。
    let tools = kernel::detail::process::ProcTools::new(
        std::env::current_exe().unwrap_or_default(),
        std::sync::Arc::clone(&process_texts),
        home.clone(),
        std::sync::Arc::clone(&write_allowed),
    )
    .with_mode(std::sync::Arc::clone(&sandbox_mode));
    // 内置文件工具：纯 Rust 直接读写，不经过外部进程（编码问题不进本程序）。
    let io = capabilities::tools::detail::FsSysIo::default();
    // **工具能力**：三张表由加载器读进来，三个出站端口（外部执行 / 内置读写 / 围栏释放）
    // 只由它的 service 持有（R12）。装配期就挡下"悬空引用 / 缺能力"。
    let fence_host: Arc<dyn kernel::ports::FenceHost + Send + Sync> =
        Arc::new(kernel::detail::confine::FenceHostAdapter::new(home.clone()));
    let tools_svc = match capabilities::tools::service::ToolsService::new(
        &systools_source,
        Arc::new(tools),
        Arc::new(io),
        Arc::clone(&fence_host),
    ) {
        Ok(svc) if capabilities::tools::api::Tools::problems(&svc).is_empty() => Arc::new(svc),
        Ok(svc) => {
            eprintln!(
                "[装配失败] 系统工具与角色表不自洽：{}",
                capabilities::tools::api::Tools::problems(&svc).join("；")
            );
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("[装配失败] {}", e);
            std::process::exit(1);
        }
    };
    let toolexec: Arc<dyn capabilities::tools::api::ToolExec + Send + Sync> = tools_svc.clone();
    let systools: Arc<dyn capabilities::tools::api::Tools + Send + Sync> = tools_svc;

    // **模块能力**：清单来源与启停覆盖落盘只由它的 service 持有（R12）；它是模块事实的唯一上游。
    let module_state =
        capabilities::modules::detail::YamlModuleState::new(home.join("modules.yaml"));
    let modules: Arc<dyn capabilities::modules::api::Modules + Send + Sync> =
        match capabilities::modules::service::ModulesService::new(
            Arc::new(source),
            Arc::new(module_state),
        ) {
            Ok(m) => Arc::new(m),
            Err(e) => {
                eprintln!("[装配失败] {}", e);
                std::process::exit(1);
            }
        };

    // **虚拟机档能力**：运行包库来源、VM 引擎与共享后端这些出站端口只由它的 service 持有（R12）；用户共享后端按平台用户链装配，起 guest 时选。
    // copy 不在用户链；只有进程级开发/应急开关才补进持有列表（仍要显式候选）。
    let order = capabilities::vm::domain::exec::default_combos(std::env::consts::OS);
    let mut shares: Vec<Arc<dyn capabilities::vm::ports::ShareBackend + Send + Sync>> = Vec::new();
    for combo in order.iter().filter(|c| c.engine == "qemu") {
        let share: Arc<dyn capabilities::vm::ports::ShareBackend + Send + Sync> = match combo.share
        {
            "smb" => Arc::new(
                capabilities::vm::detail::SmbShare::new().with_book(std::sync::Arc::clone(&book)),
            ),
            "virtiofs" => Arc::new(
                capabilities::vm::detail::VirtioFsShare::new()
                    .with_book(std::sync::Arc::clone(&book)),
            ),
            "9p" => Arc::new(capabilities::vm::detail::NinePShare::new()),
            _ => continue,
        };
        if !shares.iter().any(|s| s.id() == share.id()) {
            shares.push(share);
        }
    }
    if capabilities::vm::api::copy_share_enabled() {
        shares.push(Arc::new(capabilities::vm::detail::CopyShare::new()));
        let note = format!(
            "{}={}：copy 共享仅用于开发/应急，性能不可用；本进程的 VM 会话会强制走它",
            capabilities::vm::api::COPY_SHARE_ENV,
            "copy"
        );
        log.warn("main::vm_share", &note);
        eprintln!("[虚拟机] {}", note);
    }
    let vm: Arc<dyn capabilities::vm::api::Vm + Send + Sync> =
        Arc::new(capabilities::vm::service::VmService::new(
            Arc::new(
                capabilities::vm::detail::QemuEngine::with_overlay_dir(
                    root.join(".vm").join("overlays"),
                )
                .with_book(std::sync::Arc::clone(&book))
                .with_log(log.clone())
                .with_texts(std::sync::Arc::clone(&process_texts)),
            ),
            shares,
        ));

    // **工作区能力**：目录布局与版本库两个出站端口只由它的 service 持有（R12）。
    let workspace: Arc<dyn capabilities::workspace::api::Workspace + Send + Sync> =
        Arc::new(capabilities::workspace::service::WorkspaceService::new(
            Arc::new(workspace),
            Arc::new(workstore),
        ));

    // **隐秘字段能力**：值只落 .home/secrets.yaml；加载失败 = 装配失败（不静默空表）。
    let secrets: Arc<dyn capabilities::secrets::api::SecretOps + Send + Sync> =
        match capabilities::secrets::service::SecretsService::new(
            Arc::clone(&modules),
            Arc::new(capabilities::secrets::detail::YamlSecrets::new(
                home.join("secrets.yaml"),
            )),
        ) {
            Ok(s) => Arc::new(s),
            Err(e) => {
                eprintln!("[装配失败] {}", e);
                std::process::exit(1);
            }
        };

    // **常驻服务能力**：真实协议适配器（MCP stdio 走守门进程的长驻会话）+ 模块能力面 + 隐秘字段面。
    let residents: Arc<dyn capabilities::residents::api::ResidentOps + Send + Sync> = Arc::new(
        capabilities::residents::service::ResidentsService::new(
            vec![Arc::new(capabilities::residents::detail::McpAdapter::new(
                Arc::new(
                    kernel::detail::ProcessSessions::new(
                        std::env::current_exe().unwrap_or_default(),
                        std::sync::Arc::clone(&process_texts),
                        home.clone(),
                        std::sync::Arc::clone(&write_allowed),
                        std::sync::Arc::clone(&book),
                    )
                    .with_mode(std::sync::Arc::clone(&sandbox_mode)),
                ),
                std::sync::Arc::clone(&log),
            ))
                as Arc<
                    dyn capabilities::residents::ports::ServiceAdapter + Send + Sync,
                >],
            Arc::clone(&modules),
            Arc::clone(&secrets),
        )
        .with_network(registry.app_settings().sandbox.policy.network)
        .with_host_runtime(
            Arc::clone(&packages),
            Arc::new(kernel::detail::HostProbeAdapter),
        ),
    );

    // 依赖补齐面先建出来：Conductor（选虚拟根要读清单）与呈现层（setup 路由）各持同一份。
    // vm 用**补齐面专用**的那份：运行期编排显式点名内部 none，不能拿会话 vm 顶替（那边只有用户档）。
    let provision: Arc<dyn capabilities::provision::api::Provision + Send + Sync> =
        Arc::new(entry::assembly::provision_with_guest_vm(&root));
    // **存储空间**：回收策略来自 settings；构建与呈现层共用同一份。
    // 存储的三个端口各一个适配器：盘点（只读）、回收（锁与删除）、guest 存活（进程台账）。
    let storage_service = Arc::new(capabilities::storage::service::StorageService::new(
        Arc::clone(&provision),
        Arc::new(capabilities::storage::detail::survey::DiskSurvey::new(
            root.clone(),
        )),
        Arc::new(capabilities::storage::detail::reclaim::DiskReclaim::new(
            root.clone(),
        )),
        Arc::new(capabilities::storage::detail::guests::ProcessGuests::new(
            &root,
        )),
        std::sync::Arc::clone(&log),
        capabilities::storage::api::StoragePolicy::from(&registry.app_settings().storage),
    ));
    let storage: Arc<dyn capabilities::storage::api::Storage + Send + Sync> =
        storage_service.clone();
    // **构建能力**：创建会话前补编模块产物（本机档宿主、虚拟机档 guest）；执行器与 `--build` 同一条路——
    // **不申请容器身份**（构建是系统动作）：AppContainer 的临时目录写不了文件；可达范围仍由围栏规格给出。
    let build_runner = kernel::detail::process::ProcTools::new(
        std::env::current_exe().unwrap_or_default(),
        std::sync::Arc::clone(&process_texts),
        home.clone(),
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    );
    // 项目内基础工具链的只读安装根：构建与 shell 共用同一份（不按 agent 重复注入）。
    let base_roots = entry::toolchains::active_roots(&root);
    // 构建的三个端口各一个适配器：源指纹与构建状态共用一份本机档（BuildStore），缓存卷落盘（CacheOnDisk）。
    let build_store = Arc::new(capabilities::builds::detail::fingerprint::BuildStore::new(
        root.clone(),
    ));
    let builds: Arc<dyn capabilities::builds::api::Builds + Send + Sync> =
        Arc::new(capabilities::builds::service::BuildsService::new(
            root.clone(),
            Arc::clone(&packages),
            Arc::clone(&vm),
            Arc::clone(&provision),
            Arc::clone(&storage),
            build_store.clone(),
            build_store.clone(),
            Arc::new(capabilities::builds::detail::cache::CacheOnDisk::new(
                root.clone(),
            )),
            Arc::new(build_runner),
            Arc::new(capabilities::tools::service::fence::ToolFenceGate),
            Arc::clone(&fence_host),
            Arc::new(kernel::detail::HostProbeAdapter),
            std::sync::Arc::clone(&log),
            base_roots.clone(),
        ));
    // 启动时要按有效启停拉起常驻服务：留一份手柄（进程级租约，不属于任何会话）。
    let residents_boot = Arc::clone(&residents);
    let mut conductor = capabilities::conductor::service::Conductor::new(
        root.clone(),
        Box::new(registry),
        Arc::new(capabilities::session::service::SessionService::new(
            Arc::new(history),
        )),
        workspace,
        Arc::clone(&vm),
        Arc::clone(&packages),
        Arc::clone(&provision),
        Arc::clone(&modules),
        Arc::clone(&builds),
        Arc::clone(&llm),
        toolexec,
        prompt,
        systools,
        std::sync::Arc::clone(&log),
        Arc::new(kernel::detail::HostProbeAdapter),
        residents,
        secrets,
    )
    .with_base_roots(base_roots);

    // 隐藏模式：实测一条通道支不支持原生工具调用，并把确定结论写回 models.yaml（要真实网络）。
    if let Some(i) = args.iter().position(|a| a == "--probe-tools") {
        let Some(id) = args.get(i + 1) else {
            eprintln!("用法：solomni --probe-tools <模型 id>");
            std::process::exit(2);
        };
        match conductor.registry_mut().probe_model_tools(id) {
            Ok(capabilities::conductor::api::ProbeOutcome::Supported { detail }) => {
                println!("[探测] 模型 {}：支持原生工具调用（{}）", id, detail);
                println!("[探测] 已把 models.yaml 的 tools 写成 native");
                std::process::exit(0);
            }
            Ok(capabilities::conductor::api::ProbeOutcome::Unsupported { detail }) => {
                println!("[探测] 模型 {}：**不支持**原生工具调用（{}）", id, detail);
                println!(
                    "[探测] 已把 models.yaml 的 tools 写成 envelope（手写信封照旧可用，能力没有任何损失）"
                );
                std::process::exit(0);
            }
            Ok(capabilities::conductor::api::ProbeOutcome::Unknown { detail }) => {
                println!("[探测] 模型 {}：无法判定（{}）", id, detail);
                println!("[探测] 登记处**没有改动**：请自行决定填 native 还是 envelope");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("[探测] 失败：{}", e);
                std::process::exit(1);
            }
        }
    }

    // 隐藏模式：探测"回放形状"——把上一轮的工具调用发回供应商时，哪种写法被接受。
    // 它**不改任何登记处**（只有 --probe-tools 带写回策略）：探测结论是事实，采不采用由人定。
    if let Some(i) = args.iter().position(|a| a == "--probe-replay") {
        let Some(id) = args.get(i + 1) else {
            eprintln!("用法：solomni --probe-replay <模型 id>");
            std::process::exit(2);
        };
        match conductor.registry().probe_replay_shape(id) {
            Ok(report) => {
                println!("[回放形状] 模型 {}（只报事实，不改登记处）：", id);
                for s in &report.shapes {
                    let verdict = if !s.accepted {
                        "被拒  "
                    } else if s.understood {
                        "收+读懂"
                    } else {
                        "收未懂 "
                    };
                    println!("  {}  {:<16} {}", verdict, s.name, s.detail);
                }
                let names = |want: fn(&capabilities::llm::api::ReplayShape) -> bool| -> String {
                    let got: Vec<&str> = report
                        .shapes
                        .iter()
                        .filter(|s| want(s))
                        .map(|s| s.name.as_str())
                        .collect();
                    if got.is_empty() {
                        "（无）".to_string()
                    } else {
                        got.join(" / ")
                    }
                };
                println!("[回放形状] 被接受的写法：{}", names(|s| s.accepted));
                println!(
                    "[回放形状] 模型真的读到了历史（回答里带回本次编号）的写法：{}",
                    names(|s| s.understood)
                );
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("[回放形状] 失败：{}", e);
                std::process::exit(1);
            }
        }
    }

    // 核心搬到它自己的执行线程：此后呈现层只持有**入站能力面**——拿不到 Conductor，也拿不到任何核心锁。
    let handle = match capabilities::conductor::api::ConductorHandle::spawn(conductor) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[装配失败] {}", e);
            std::process::exit(1);
        }
    };
    let mut ops = capabilities::conductor::api::Ops::from_handle(&handle);
    // 依赖补齐面：组合根注入（能力面默认未接入，这里换成真实现）。
    ops.provision = Arc::clone(&provision);
    ops.storage = Arc::clone(&storage);
    storage_service.start_auto();
    // 基础工具链事实：Web 端横幅与 CLI 启动报告读同一份。
    ops.toolchains = entry::toolchains::json(&base);

    // 启动拉起：把所有**已启用**的常驻服务拉起来（进程级租约，不属于任何会话）。
    for line in residents_boot.start_enabled() {
        log.info("main::residents", &line);
    }

    // 工具级确认要有地方被作答：网页有裁决卡，CLI 在生成中就地按选项答——两边都接上了。
    handle.allow_tool_cards();
    if web {
        entry::web::serve_web(
            ops,
            flags.port.unwrap_or(presentation::web::DEFAULT_PORT),
            &fence_report,
        );
    } else {
        // CLI 里输入 web 可直接转入 Web，无需重启进程（能力面可克隆，两份呈现共用同一个核心）。
        // 在 Web 里按 Ctrl+C = **回到 CLI**（web::serve 收到中断就正常返回）；在 CLI 提示符下再按 = 退出。
        loop {
            match presentation::cli::run(
                ops.clone(),
                presentation::web::DEFAULT_PORT,
                flags.plain,
                !flags.no_anim,
            ) {
                presentation::cli::CliExit::Exit => break,
                presentation::cli::CliExit::Web(port) => {
                    entry::web::serve_web(ops.clone(), port, &fence_report);
                }
            }
        }
    }
}
