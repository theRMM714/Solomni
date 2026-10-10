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

use std::path::PathBuf;
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // 守门模式（内部协议，用户不用）：把围栏装好再跑模块声明的命令，退出码即工具退出码。
    if let Some(i) = args
        .iter()
        .position(|a| a == kernel::detail::confine::FENCE_FLAG)
    {
        std::process::exit(guard::fence_run(&args, i));
    }
    // 自检（机器可读）：把"这台机器能承载哪些测试"如实交出来——测试入口据此判定，不靠猜（见 TESTING.md）。
    if args.iter().any(|a| a == "--doctor") {
        std::process::exit(diagnostics::doctor());
    }
    // 隐藏模式：走**产品自己那套**出站链路打一次最小 HTTPS 请求，三态如实回报
    // （ok / no-net / tls-fail|fail）——CI 三平台据此验本构建的 TLS 栈，不需要任何密钥。
    if let Some(i) = args.iter().position(|a| a == "--https-check") {
        std::process::exit(diagnostics::https_check(&args, i));
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
    // 隐藏模式：按条处置围栏台账（列清单 / 还原一条 / 撤一条 / 删一个 profile）——比整体收尾更细，
    // 所以排在 --fence-clean 之前；两者共用同一份台账与同一套 ACE 读法（见 docs/tools/README.md）。
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
    let source = capabilities::workspace::detail::FsModules::new(
        root.join("modules"),
        capabilities::tools::api::names(),
    );
    // 运行包库：依赖文件夹 runtimes/（一个包 = 一个文件夹 + package.yaml）。
    let packages = capabilities::workspace::detail::FsPackages::new(root.join("runtimes"));
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
    let registry = match capabilities::registry::service::RegistryService::new(
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
    // 两张表归**工具能力**（加载器在它自己的 detail 里；装载后由它的 service 持有）。
    let systools_source =
        capabilities::tools::detail::yaml_systools::YamlSystools::new(root.join("systools"));

    // 围栏是否允许在本机写权限：设置里授权过、或环境变量显式指定（SOLOMNI_FENCE_WRITE=1/0 可取反）。
    // 默认不准——没经过用户同意，本程序不动本机任何权限项。
    let write_allowed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    // 启动报告已经算过的同一个事实：Web 概览要按它区分"本机能力"与"本次实际"（下面那块必定赋值）。
    let allow_fence_write;
    // 工具执行：外层拉起的守门进程就是本程序自己（围栏在它里面装）。
    let tools = kernel::detail::process::ProcTools::new(
        std::env::current_exe().unwrap_or_default(),
        std::sync::Arc::new(capabilities::tools::detail::PromptProcessTexts::new(
            prompt.tools(),
        )),
        home.clone(),
        std::sync::Arc::clone(&write_allowed),
    );
    // 内置文件工具：纯 Rust 直接读写，不经过外部进程（编码问题不进本程序）。
    let io = capabilities::tools::detail::FsSysIo::default();
    // **工具能力**：两张表由加载器读进来，三个出站端口（外部执行 / 内置读写 / 围栏释放）
    // 只由它的 service 持有（R12）。装配期就挡下"悬空引用 / 缺能力"。
    let tools_svc = match capabilities::tools::service::ToolsService::new(
        &systools_source,
        Arc::new(tools),
        Arc::new(io),
        Arc::new(kernel::detail::confine::FenceHostAdapter::new(home)),
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

    // **工作区能力**：三个出站端口（清单 / 运行包库 / 目录布局）只由它的 service 持有（R12）。
    let workspace: Arc<dyn capabilities::workspace::api::Workspace + Send + Sync> =
        Arc::new(capabilities::workspace::service::WorkspaceService::new(
            Arc::new(source),
            Arc::new(packages),
            Arc::new(workspace),
            Arc::new(workstore),
        ));

    let mut conductor = capabilities::conductor::service::Conductor::new(
        Box::new(registry),
        Arc::new(capabilities::session::service::SessionService::new(
            Arc::new(history),
        )),
        workspace,
        Arc::clone(&llm),
        toolexec,
        prompt,
        systools,
        std::sync::Arc::clone(&log),
        Arc::new(kernel::detail::HostProbeAdapter),
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

    // 写权限开关定稿：环境变量优先（测试/CI 用得到），否则看设置。
    {
        let env_flag = std::env::var("SOLOMNI_FENCE_WRITE").ok();
        let allow = match env_flag.as_deref() {
            Some("1") => true,
            Some("0") => false,
            _ => conductor.registry().app_settings().fence_write,
        };
        write_allowed.store(allow, std::sync::atomic::Ordering::Relaxed);
        allow_fence_write = allow;
        let cap = kernel::detail::confine::capability();
        // 能力与本次实际**分开报**：授权与否决定路径级围栏装不装，但进程树围栏、资源上限与环境白名单
        // 在两种时段都生效（未授权不等于无围栏）。只说"本机能力"会让用户以为未授权时什么都没有。
        let usable = |ok: bool| if ok { "可用" } else { "不可用" };
        let (fs, net) = if allow {
            (cap.fs, cap.net)
        } else {
            (false, false)
        };
        println!(
            "[围栏] 本机能力：文件系统={} 断网={} 进程树={}（{}）",
            usable(cap.fs),
            usable(cap.net),
            usable(cap.tree),
            cap.note
        );
        let ro = conductor.registry().app_settings().fence_read.len();
        println!(
            "[围栏] 本次实际：文件系统={} 断网={} 进程树={}；容器授权={}；只读根={} 个（未授权时只放行进程树与资源上限，不写本机任何权限项；要启用：设置里打开，或 .home/settings.yaml 写 fence_write: true / fence_read: [路径…]）",
            usable(fs),
            usable(net),
            usable(cap.tree),
            if allow { "已授权" } else { "未授权" },
            ro
        );
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
    let ops = capabilities::conductor::api::Ops::from_handle(&handle);

    // 工具级确认要有地方被作答：网页有裁决卡，CLI 在生成中就地按选项答——两边都接上了。
    handle.allow_tool_cards();
    if web {
        serve_web(ops, port_flag(&args), allow_fence_write);
    } else {
        // CLI 里输入 webui 可直接转入 Web，无需重启进程（能力面可克隆，两份呈现共用同一个核心）。
        // 在 Web 里按 Ctrl+C = **回到 CLI**（web::serve 收到中断就正常返回）；在 CLI 提示符下再按 = 退出。
        loop {
            match presentation::cli::run(ops.clone(), presentation::web::DEFAULT_PORT) {
                presentation::cli::CliExit::Exit => break,
                presentation::cli::CliExit::Web(port) => {
                    serve_web(ops.clone(), port, allow_fence_write);
                }
            }
        }
    }
}

fn serve_web(ops: capabilities::conductor::api::Ops, port: u16, write_allowed: bool) {
    let cap = kernel::detail::confine::capability();
    // 能力与本次实际**分开报**（与启动报告同一套说法）：未授权时路径级围栏是关的，
    // 但进程树与资源上限照旧生效——概览里必须让用户看到这个区别，不能只看"本机能力"。
    let (fs, net) = if write_allowed {
        (cap.fs, cap.net)
    } else {
        (false, false)
    };
    let fence = presentation::web::FenceInfo {
        fs: cap.fs,
        net: cap.net,
        tree: cap.tree,
        note: cap.note,
        effective_fs: fs,
        effective_net: net,
        write_allowed,
        read_only_roots: ops
            .registry
            .settings()
            .map(|s| s.fence_read.len())
            .unwrap_or(0),
    };
    if let Err(e) = presentation::web::serve(ops, port, fence) {
        eprintln!("[Web 服务异常] {}", e);
        std::process::exit(1);
    }
}
