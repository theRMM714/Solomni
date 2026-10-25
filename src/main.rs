//! Solomni 组合根 + 入口。
//! 组合根职责：创建各适配器实例 → 装配各能力（含协调业务 `conductor`）→ 交给呈现层（CLI 或 Web）。
//! 依赖方向：main → adapters / capabilities / presentation；协调业务不知道后两者存在。

mod capabilities;
mod diagnostics;
mod entry;
mod guard;
mod kernel;
mod presentation;

#[cfg(test)]
mod tests;

use capabilities::registry::api::Registry as _;
use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;

/// 目的：补齐面专用的 vm 能力面：只持内部 `none` 共享档（运行期编排只经 QGA，不挂载、不建可写层）。
/// 约束：与会话 vm 分开装配——`none` 不进用户候选链，避免 `clients` 为空的 guest 被静默当成无共享。
/// 约束：网页补齐与 `--provision` 都经这里，两处不各装一份。
pub(crate) fn provision_vm(
    book: Arc<kernel::detail::procs::ProcessBook>,
) -> Arc<dyn capabilities::vm::api::Vm + Send + Sync> {
    provision_vm_with(Arc::new(
        capabilities::vm::detail::QemuEngine::new().with_book(book),
    ))
}

/// 目的：同上，但允许注入引擎替身（回归测试用）；生产入口只用 `provision_vm`。
pub(crate) fn provision_vm_with(
    engine: Arc<dyn capabilities::vm::ports::VmEngine + Send + Sync>,
) -> Arc<dyn capabilities::vm::api::Vm + Send + Sync> {
    Arc::new(capabilities::vm::service::VmService::new(
        engine,
        vec![Arc::new(capabilities::vm::detail::NoneShare::new())],
    ))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // 长驻子进程守门模式（内部协议，用户不用）：macOS 没有 PR_SET_PDEATHSIG，由本程序自己盯核心，
    // 核心一死就把子进程组收掉（见 kernel::detail::procs::watch）。
    if let Some(i) = args
        .iter()
        .position(|a| a == kernel::detail::procs::WATCHDOG_FLAG)
    {
        std::process::exit(guard::proc_watch(&args, i));
    }
    // 守门模式（内部协议，用户不用）：把围栏装好再跑模块声明的命令，退出码即工具退出码。
    if let Some(i) = args
        .iter()
        .position(|a| a == kernel::detail::confine::FENCE_FLAG)
    {
        std::process::exit(guard::fence_run(&args, i));
    }
    // 自检（机器可读）：把"这台机器能承载哪些测试"如实交出来——测试入口据此判定，不靠猜（见 TESTING.md）。
    // 放在根解析之后：它要按**产品根**读"项目内已装了什么"（与就绪判据同一份来源）。
    let doctor = args.iter().any(|a| a == "--doctor");
    // 隐藏模式：走**产品自己那套**出站链路打一次最小 HTTPS 请求，三态如实回报
    // （ok / no-net / tls-fail|fail）——CI 三平台据此验本构建的 TLS 栈，不需要任何密钥。
    if let Some(i) = args.iter().position(|a| a == "--https-check") {
        std::process::exit(diagnostics::https_check(&args, i));
    }
    // 隐藏模式：把归档解到目录（跨平台纯 Rust）——provision 能力用它解归档，不依赖用户装了什么解压工具。
    if let Some(i) = args.iter().position(|a| a == "--unpack") {
        std::process::exit(diagnostics::unpack_cli(&args, i));
    }
    // 环境白名单（机器可读）：把运行期交给工具进程的环境逐行交出来——探针据此在**同一个环境**里驱动
    // 守门进程，不另抄一份（抄一份会漂移，也会漏掉只有真实环境才暴露的问题）。
    if let Some(i) = args.iter().position(|a| a == "--print-fence-env") {
        std::process::exit(diagnostics::print_fence_env(&args, i));
    }
    // 机制验证（机器可读，探针与测试驱动）：不装围栏、不写任何权限项，只如实报"这次能不能强制住"。
    // 未授权时段的拒绝执行（见 capabilities/tools/detail/proc_tools.rs）就靠这一份结论。
    if let Some(i) = args.iter().position(|a| a == "--fence-verify") {
        std::process::exit(diagnostics::fence_verify(&args, i));
    }
    // 入站契约（机器可读）：HTTP 路由目录的唯一定义（见 docs/presentation/contracts.md）。
    if args.iter().any(|a| a == "--print-routes") {
        diagnostics::print_routes();
        std::process::exit(0);
    }
    // 启动形态：无参数 = CLI（默认）；-webUI = Web 转录中心。
    let web = args.iter().any(|a| a == "-webUI");
    let raw_root = args
        .iter()
        .position(|a| a == "--root")
        .and_then(|i| args.get(i + 1))
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
    if doctor {
        std::process::exit(diagnostics::doctor(&root));
    }
    // 隐藏模式：依赖补齐的状态（机器可读，只报事实）——缺什么、装到哪、从哪来。
    if args.iter().any(|a| a == "--provision") {
        std::process::exit(diagnostics::provision(&root, &args));
    }
    // 维护动作：显式回收一个工作里不再可达的内容对象（默认 dry-run，--yes 才删）。
    if let Some(i) = args
        .iter()
        .position(|a| a == "--work-gc" || a.starts_with("--work-gc="))
    {
        std::process::exit(diagnostics::work_gc(&root, &args, i));
    }
    // 隐藏模式：构建一个模块声明的产物（**系统动作**：在目标环境里跑 module.yaml 的 build.command）。
    if let Some(i) = args
        .iter()
        .position(|a| a == "--build" || a.starts_with("--build="))
    {
        std::process::exit(diagnostics::build(&root, &args, i));
    }
    // 隐藏模式：按条处置围栏台账（列清单 / 还原一条 / 撤一条 / 删一个 profile）——比整体收尾更细，
    // 所以排在 --fence-clean 之前；两者共用同一份台账与同一套 ACE 读法（见 docs/kernel/README.md 的「围栏落点：必要与可选」）。
    if let Some(code) = guard::fence_grant(&args, &root) {
        std::process::exit(code);
    }
    // 隐藏模式：精确回收围栏写过的权限项（不需要装配核心，也就不需要提示词册）。
    if args.iter().any(|a| a == "--fence-clean") {
        std::process::exit(guard::fence_clean(&root));
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
    let installed = capabilities::provision::api::Provision::status(
        &capabilities::provision::ProvisionService::new(&root),
    )
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
    let authorized = authorize(
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
    let fence_report = FenceReport::new(fence_cap.clone(), authorized, Arc::clone(&sandbox_mode));
    print_fence_report(&fence_report, sandbox_cfg.policy.host_read_roots.len());

    let missing = missing_parts(&requirements, &fence_cap, authorized);
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
            match maybe_onboard_windows(
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
            match run_gate(
                &missing,
                sandbox_mode.ask_allowed,
                tty,
                forced_degraded.as_deref(),
                fence_write_env.as_deref(),
                &mut registry,
                &mut stdin,
                &mut stdout,
            ) {
                GateOutcome::Proceed => sandbox_mode.set_degraded(true),
                GateOutcome::Restart => {
                    println!(
                        "[沙箱] 已写入 .home/settings.yaml；启动器会把本进程整体重启（直接运行二进制时请重新启动）。"
                    );
                    std::process::exit(presentation::web::RESTART_EXIT_CODE);
                }
                GateOutcome::Exit(code) => std::process::exit(code),
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
    let tools_svc = match capabilities::tools::service::ToolsService::new(
        &systools_source,
        Arc::new(tools),
        Arc::new(io),
        Arc::new(kernel::detail::confine::FenceHostAdapter::new(home.clone())),
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
                .with_book(std::sync::Arc::clone(&book)),
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
    let provision: Arc<dyn capabilities::provision::api::Provision + Send + Sync> = Arc::new(
        capabilities::provision::ProvisionService::with_vm(&root, provision_vm(Arc::clone(&book))),
    );
    // **构建能力**：创建会话前补编模块产物（本机档在宿主、虚拟机档在 guest）。
    // 执行器与 `--build` 同一条路：**不申请容器身份**（构建是系统动作，不是 agent 工具）——
    // 否则编译器在 AppContainer 的临时目录里写不了文件；可达范围仍由围栏规格给出（模块目录 + 状态目录）。
    let build_runner = kernel::detail::process::ProcTools::new(
        std::env::current_exe().unwrap_or_default(),
        std::sync::Arc::clone(&process_texts),
        home.clone(),
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    );
    let builds: Arc<dyn capabilities::builds::api::Builds + Send + Sync> =
        Arc::new(capabilities::builds::service::BuildsService::new(
            root.clone(),
            Arc::clone(&packages),
            Arc::clone(&vm),
            Arc::clone(&provision),
            Arc::new(build_runner),
            Arc::new(kernel::detail::HostProbeAdapter),
            std::sync::Arc::clone(&log),
            entry::toolchains::active_roots(&root),
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
    );

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
                println!("[探测] 已把 models.yaml 的 tools 写成 envelope（手写信封照旧可用，能力没有任何损失）");
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

    let port_flag = |args: &[String]| {
        args.iter()
            .position(|a| a == "--web-port")
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or(presentation::web::DEFAULT_PORT)
    };

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
    // 基础工具链事实：Web 端横幅与 CLI 启动报告读同一份。
    ops.toolchains = entry::toolchains::json(&base);

    // 启动拉起：把所有**已启用**的常驻服务拉起来（进程级租约，不属于任何会话）。
    for line in residents_boot.start_enabled() {
        log.info("main::residents", &line);
    }

    // 工具级确认要有地方被作答：网页有裁决卡，CLI 在生成中就地按选项答——两边都接上了。
    handle.allow_tool_cards();
    if web {
        serve_web(ops, port_flag(&args), &fence_report);
    } else {
        // CLI 里输入 webui 可直接转入 Web，无需重启进程（能力面可克隆，两份呈现共用同一个核心）。
        // 在 Web 里按 Ctrl+C = **回到 CLI**（web::serve 收到中断就正常返回）；在 CLI 提示符下再按 = 退出。
        loop {
            match presentation::cli::run(ops.clone(), presentation::web::DEFAULT_PORT) {
                presentation::cli::CliExit::Exit => break,
                presentation::cli::CliExit::Web(port) => {
                    serve_web(ops.clone(), port, &fence_report);
                }
            }
        }
    }
}

/// 目的：环境变量优先于设置里的写 ACL 授权；只认 1/0，其它值不覆盖。
fn authorize(env: Option<&str>, setting: bool) -> bool {
    match env {
        Some("1") => true,
        Some("0") => false,
        _ => setting,
    }
}

/// 目的：一项强制要求没满足的**原因**——未授权能靠授权修好，能力缺失不能。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MissingCause {
    Unsupported,
    Unauthorized,
}

impl MissingCause {
    fn label(self) -> &'static str {
        match self {
            MissingCause::Unsupported => "本机能力不可用",
            MissingCause::Unauthorized => "Windows 容器围栏未授权（本机能力可用）",
        }
    }
}

/// 目的：一项“要求但没满足”的围栏与原因；启动门据此决定能不能修。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MissingPart {
    name: &'static str,
    cause: MissingCause,
}

/// 目的：把“要求但没满足”的项按原因拆开。授权只影响 Windows 的路径级围栏；
///   Linux / macOS 的围栏在守门进程里自足，未授权不等于无围栏。
fn missing_parts(
    requirements: &kernel::api::SandboxRequirements,
    cap: &kernel::detail::confine::Capability,
    authorized: bool,
) -> Vec<MissingPart> {
    let paths_ok = !cfg!(windows) || authorized;
    let mut out = Vec::new();
    if requirements.filesystem {
        if !cap.fs {
            out.push(MissingPart {
                name: "文件系统围栏",
                cause: MissingCause::Unsupported,
            });
        } else if !paths_ok {
            out.push(MissingPart {
                name: "文件系统围栏",
                cause: MissingCause::Unauthorized,
            });
        }
    }
    if requirements.network {
        if !cap.net {
            out.push(MissingPart {
                name: "网络围栏",
                cause: MissingCause::Unsupported,
            });
        } else if !paths_ok {
            out.push(MissingPart {
                name: "网络围栏",
                cause: MissingCause::Unauthorized,
            });
        }
    }
    if requirements.process_tree && !cap.tree {
        out.push(MissingPart {
            name: "进程树围栏",
            cause: MissingCause::Unsupported,
        });
    }
    // 环境白名单由进程机制自足（组合根传 env_ok=true），没有“能力缺失”这一路。
    out
}

/// 目的：把指定原因的缺失项拼成人类可读清单。
fn missing_names(parts: &[MissingPart], cause: MissingCause) -> String {
    parts
        .iter()
        .filter(|p| p.cause == cause)
        .map(|p| p.name)
        .collect::<Vec<_>>()
        .join("、")
}

/// 目的：启动报告先把「本机能力」与「本次实际」分开说清，再让启动门提问。
fn print_fence_report(report: &FenceReport, read_only_roots: usize) {
    let usable = |ok: bool| if ok { "可用" } else { "不可用" };
    println!(
        "[围栏] 本机能力：文件系统={} 断网={} 进程树={}（{}）",
        usable(report.cap.fs),
        usable(report.cap.net),
        usable(report.cap.tree),
        report.cap.note
    );
    println!(
        "[围栏] 本次实际：文件系统={} 断网={} 进程树={}；容器授权={}；只读根={} 个（{}）",
        usable(report.fs),
        usable(report.net),
        usable(report.cap.tree),
        fence_auth_label(report.write_allowed),
        read_only_roots,
        fence_auth_note(report.write_allowed)
    );
}

/// 目的：可注入读写的 y/n 提问——启动门与测试用同一套默认值（空行 = 默认项）。
fn ask_yes_no<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    question: &str,
    default_yes: bool,
) -> bool {
    let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
    let _ = write!(writer, "{} {} ", question, hint);
    let _ = writer.flush();
    let mut answer = String::new();
    if reader.read_line(&mut answer).is_err() {
        return default_yes;
    }
    match answer.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => true,
        "n" | "no" => false,
        _ => default_yes,
    }
}

/// 目的：把 Windows 的写 ACL 授权落进设置；调用方负责在成功后整体重启。
fn write_acl_setting(
    registry: &mut dyn capabilities::registry::api::Registry,
    value: bool,
) -> Result<(), String> {
    let mut app = registry.app_settings();
    app.sandbox.policy.windows.write_acl = value;
    registry.set_app_settings(app)
}

/// 目的：能力缺失时的持久收口——把四项强制等级都降为 best_effort。
///   只降“要求等级”，不关闭已有的机制：装得上时仍照常强制，只是缺了不再拒绝启动。
fn lower_enforce_to_best_effort(
    registry: &mut dyn capabilities::registry::api::Registry,
) -> Result<(), String> {
    use capabilities::registry::api::SandboxEnforceLevel::BestEffort;
    let mut app = registry.app_settings();
    app.sandbox.policy.enforce.filesystem = BestEffort;
    app.sandbox.policy.enforce.network = BestEffort;
    app.sandbox.policy.enforce.process_tree = BestEffort;
    app.sandbox.policy.enforce.env_whitelist = BestEffort;
    registry.set_app_settings(app)
}

/// 目的：首次启动（没有 settings.yaml）时先给一次授权机会；只 Windows、只 TTY、无环境变量覆盖。
/// 返回 true = 已写盘，调用方应以 75 整体重启；false = 没写（用户暂不授权/不适用），继续走启动门。
#[allow(clippy::too_many_arguments)]
fn maybe_onboard_windows<R: BufRead, W: Write>(
    missing: &[MissingPart],
    settings_exists: bool,
    tty: bool,
    fence_write_env: Option<&str>,
    forced_degraded: Option<&str>,
    cap: &kernel::detail::confine::Capability,
    registry: &mut dyn capabilities::registry::api::Registry,
    reader: &mut R,
    writer: &mut W,
) -> Result<bool, String> {
    let needs_authorization = missing
        .iter()
        .any(|p| p.cause == MissingCause::Unauthorized);
    let has_unsupported = missing.iter().any(|p| p.cause == MissingCause::Unsupported);
    if !cfg!(windows)
        || !needs_authorization
        || has_unsupported
        || settings_exists
        || !tty
        || fence_write_env.is_some()
        || forced_degraded.is_some()
        || !cap.fs
        || !cap.net
    {
        return Ok(false);
    }
    let _ = writeln!(
        writer,
        "[围栏] Windows 的容器围栏需要授权 Solomni 在工具执行时给 agent 沙箱目录写 ACL。"
    );
    let _ = writeln!(
        writer,
        "[围栏] 授权会记进台账，可用 --fence-clean 撤销；程序默认不写。"
    );
    if !ask_yes_no(reader, writer, "[围栏] 现在授权并启用容器围栏？", true) {
        return Ok(false);
    }
    write_acl_setting(registry, true)?;
    Ok(true)
}

/// 目的：启动门的处置结论——继续（已同意降级）、写盘后重启、或退出。
enum GateOutcome {
    Proceed,
    Restart,
    Exit(i32),
}

/// 目的：环境不满足时的唯一出路：本次降级 / 重设 / 退出；不允许默认进入会话。
/// 约束：没有可回答的前端、或测试注入强制严格时，一律退出（不启动 CLI / Web）。
#[allow(clippy::too_many_arguments)]
fn run_gate<R: BufRead, W: Write>(
    missing: &[MissingPart],
    ask_allowed: bool,
    tty: bool,
    forced_degraded: Option<&str>,
    fence_write_env: Option<&str>,
    registry: &mut dyn capabilities::registry::api::Registry,
    reader: &mut R,
    writer: &mut W,
) -> GateOutcome {
    if forced_degraded == Some("0") {
        let _ = writeln!(
            writer,
            "[沙箱] SOLOMNI_SANDBOX_DEGRADED=0：按全局策略拒绝执行，不进入会话。"
        );
        return GateOutcome::Exit(1);
    }
    if forced_degraded == Some("1") {
        let _ = writeln!(
            writer,
            "[沙箱] SOLOMNI_SANDBOX_DEGRADED=1 不能覆盖 on_unavailable=refuse。"
        );
        return GateOutcome::Exit(1);
    }
    if !tty {
        let _ = writeln!(
            writer,
            "[沙箱] 没有可回答的前端：不进入会话（fail-closed）。"
        );
        let _ = writeln!(
            writer,
            "[沙箱] 要启用：在 .home/settings.yaml 写 sandbox.policy.windows.write_acl: true，或设置 SOLOMNI_FENCE_WRITE=1。"
        );
        return GateOutcome::Exit(1);
    }

    if ask_allowed {
        let _ = writeln!(writer, "[沙箱] 本次无围栏运行？（不落盘，重启后重新询问）");
        if ask_yes_no(reader, writer, "[沙箱] 继续？", false) {
            return GateOutcome::Proceed;
        }
    } else {
        let _ = writeln!(writer, "[沙箱] 全局策略不允许本次降级。");
    }

    if !ask_yes_no(reader, writer, "[沙箱] 现在重新设置？", true) {
        let _ = writeln!(writer, "[沙箱] 已退出，未进入会话。");
        return GateOutcome::Exit(0);
    }

    let unsupported = missing.iter().any(|p| p.cause == MissingCause::Unsupported);
    if unsupported {
        let _ = writeln!(
            writer,
            "[沙箱] 本机能力不足：{}。",
            missing_names(missing, MissingCause::Unsupported)
        );
        let _ = writeln!(
            writer,
            "[沙箱] 重新设置只能把强制等级降为 best_effort（持久接受无强制）。"
        );
        if !ask_yes_no(reader, writer, "[沙箱] 继续？", false) {
            let _ = writeln!(writer, "[沙箱] 已退出，未进入会话。");
            return GateOutcome::Exit(0);
        }
        return match lower_enforce_to_best_effort(registry) {
            Ok(()) => GateOutcome::Restart,
            Err(e) => {
                let _ = writeln!(writer, "[沙箱] 写入设置失败：{}", e);
                GateOutcome::Exit(1)
            }
        };
    }

    if fence_write_env.is_some() {
        let _ = writeln!(
            writer,
            "[沙箱] SOLOMNI_FENCE_WRITE 正在覆盖设置；改环境变量后重启，或退出。"
        );
        let _ = writeln!(writer, "[沙箱] 已退出，未进入会话。");
        return GateOutcome::Exit(0);
    }

    let _ = writeln!(
        writer,
        "[沙箱] 需要授权 Windows 写目录 ACL 才能启用容器围栏。"
    );
    if !ask_yes_no(reader, writer, "[沙箱] 现在授权并启用？", true) {
        let _ = writeln!(writer, "[沙箱] 已退出，未进入会话。");
        return GateOutcome::Exit(0);
    }
    match write_acl_setting(registry, true) {
        Ok(()) => GateOutcome::Restart,
        Err(e) => {
            let _ = writeln!(writer, "[沙箱] 写入设置失败：{}", e);
            GateOutcome::Exit(1)
        }
    }
}

/// 目的：启动报告与 Web 概览共用的一份围栏事实：本机能力 + 本次实际（授权后的路径级围栏）。
/// 约束：授权只影响 Windows 的目录 ACL；其它平台由 `Capability::effective_paths` 判成与本机能力相同。
struct FenceReport {
    cap: kernel::detail::confine::Capability,
    write_allowed: bool,
    fs: bool,
    net: bool,
    sandbox_mode: std::sync::Arc<kernel::api::SandboxMode>,
}

impl FenceReport {
    /// 目的：按本机能力与本次授权算出同一份事实；启动报告与 Web 不再各编一套。
    fn new(
        cap: kernel::detail::confine::Capability,
        write_allowed: bool,
        sandbox_mode: std::sync::Arc<kernel::api::SandboxMode>,
    ) -> Self {
        let (fs, net) = cap.effective_paths(write_allowed);
        Self {
            cap,
            write_allowed,
            fs,
            net,
            sandbox_mode,
        }
    }
}

/// 目的：本次授权状态的人类可读标签——Unix 没有外层写 ACL 这一步，不能显示成"未授权"。
fn fence_auth_label(write_allowed: bool) -> &'static str {
    #[cfg(windows)]
    {
        if write_allowed {
            "已授权"
        } else {
            "未授权"
        }
    }
    #[cfg(not(windows))]
    {
        let _ = write_allowed;
        "不适用"
    }
}

/// 目的：本次授权影响的说明——只讲 Windows 的目录 ACL，避免让 Unix 读者以为还有一步授权没做。
fn fence_auth_note(write_allowed: bool) -> &'static str {
    #[cfg(windows)]
    {
        if write_allowed {
            "容器围栏按本机能力装，写入的权限项逐条记台账"
        } else {
            "不写本机任何权限项，容器围栏不装，只放行进程树、资源上限与环境白名单；要启用：设置里打开，或 .home/settings.yaml 写 sandbox.policy.windows.write_acl: true"
        }
    }
    #[cfg(not(windows))]
    {
        let _ = write_allowed;
        "sandbox.policy.windows.write_acl 只影响 Windows 的目录 ACL；本平台围栏在守门进程里自足，与授权无关"
    }
}

fn serve_web(ops: capabilities::conductor::api::Ops, port: u16, report: &FenceReport) {
    // 与启动报告同一份事实：能力取本机，实际取授权后的路径级围栏。
    let fence = presentation::web::FenceInfo {
        fs: report.cap.fs,
        net: report.cap.net,
        tree: report.cap.tree,
        note: report.cap.note.clone(),
        effective_fs: report.fs,
        effective_net: report.net,
        write_allowed: report.write_allowed,
        read_only_roots: ops
            .registry
            .settings()
            .map(|s| s.sandbox.policy.host_read_roots.len())
            .unwrap_or(0),
        sandbox_missing: report
            .sandbox_mode
            .missing()
            .into_iter()
            .map(|s| s.to_string())
            .collect(),
        sandbox_degraded: report.sandbox_mode.degraded(),
    };
    if let Err(e) = presentation::web::serve(ops, port, fence) {
        eprintln!("[Web 服务异常] {}", e);
        std::process::exit(1);
    }
}
