//! Solomni 组合根 + 入口。
//! 组合根职责：创建各适配器实例 → 注入 Core 门面 → 交给呈现层（CLI 或 Web）。
//! 依赖方向：main → adapters / core / presentation；core 不知道后两者存在。

mod adapters;
mod core;
mod presentation;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // 守门模式（内部协议，用户不用）：把围栏装好再跑模块声明的命令，退出码即工具退出码。
    if let Some(i) = args.iter().position(|a| a == adapters::confine::FENCE_FLAG) {
        std::process::exit(fence_run(&args, i));
    }
    // 自检（机器可读）：把"这台机器能承载哪些测试"如实交出来——测试入口据此判定，不靠猜（见 TESTING.md）。
    if args.iter().any(|a| a == "--doctor") {
        std::process::exit(doctor());
    }
    // 隐藏模式：走**产品自己那套**出站链路打一次最小 HTTPS 请求，三态如实回报
    // （ok / no-net / tls-fail|fail）——CI 三平台据此验本构建的 TLS 栈，不需要任何密钥。
    if let Some(i) = args.iter().position(|a| a == "--https-check") {
        std::process::exit(https_check(&args, i));
    }
    // 环境白名单（机器可读）：把运行期交给工具进程的环境逐行交出来——探针据此在**同一个环境**里驱动
    // 守门进程，不另抄一份（抄一份会漂移，也会漏掉只有真实环境才暴露的问题）。
    if let Some(i) = args.iter().position(|a| a == "--print-fence-env") {
        std::process::exit(print_fence_env(&args, i));
    }
    // 机制验证（机器可读，探针与测试驱动）：不装围栏、不写任何权限项，只如实报"这次能不能强制住"。
    // 未授权时段的拒绝执行（见 adapters/proc_tools.rs）就靠这一份结论。
    if let Some(i) = args.iter().position(|a| a == "--fence-verify") {
        std::process::exit(fence_verify(&args, i));
    }
    // 入站契约（机器可读）：HTTP 路由目录的唯一定义（见 docs/architecture/contracts.md）。
    if args.iter().any(|a| a == "--print-routes") {
        println!("{}", presentation::routes::catalog_json());
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
    let (root, root_note) = resolve_root(&raw_root);
    // 隐藏模式：精确回收围栏写过的权限项（不需要装配核心，也就不需要提示词册）。
    if args.iter().any(|a| a == "--fence-clean") {
        std::process::exit(fence_clean(&root));
    }

    // 组合根：唯一允许 new 具体适配器的地方（依赖注入）。
    let log: std::sync::Arc<dyn core::ports::Log + Send + Sync> = match adapters::FileLog::new(&root, "Solomni 运行日志") {
        Ok(l) => std::sync::Arc::new(l),
        Err(e) => {
            eprintln!("[日志系统异常] {}（进程继续，日志降级为 stderr）", e);
            std::sync::Arc::new(core::ports::NoopLog)
        }
    };
    if let Some(note) = &root_note {
        eprintln!("[根目录] {}", note);
        log.warn("main::root", note);
    }
    // 围栏能力如实告知（不强于实际：机制缺什么就说缺什么）。
    let fence_cap = adapters::confine::capability();
    log.info(
        "main::fence",
        &format!("围栏能力：文件系统={} 断网={} 进程树={}；{}", fence_cap.fs, fence_cap.net, fence_cap.tree, fence_cap.note),
    );
    println!("[围栏] {}", fence_cap.note);
    let store = adapters::YamlSettingsStore::new(
        root.join(".home").join("providers.yaml"),
        root.join(".home").join("models.yaml"),
        root.join(".home").join("settings.yaml"),
        root.join(".home").join("agents.yaml"),
    );
    let history = adapters::FsHistory::new(root.join("session"));
    let workspace = adapters::FsWorkspace::new(root.join("session"));
    let source = adapters::FsModules::new(root.join("modules"));
    // 运行包库：依赖文件夹 runtimes/（一个包 = 一个文件夹 + package.yaml）。
    let packages = adapters::FsPackages::new(root.join("runtimes"));
    // 端点记忆：谁先通了就固定谁，后续会话不再反复探测候选。
    let memo = adapters::endpoint::memo_new();
    let gateway = adapters::HttpGateway::with_log(std::sync::Arc::clone(&log), std::sync::Arc::clone(&memo));
    let catalog = adapters::HttpModelCatalog::with_log(std::sync::Arc::clone(&log), std::sync::Arc::clone(&memo));
    let prompts = adapters::YamlPrompts::new(root.join("prompts.yaml"));
    // 册子只读一次：core 与适配层（工具回执里的那些收尾标记）共用同一份。
    let book = match core::ports::PromptSource::load(&prompts) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[装配失败] {}", e);
            std::process::exit(1);
        }
    };
    // 围栏是否允许在本机写权限：设置里授权过、或环境变量显式指定（SOLOMNI_FENCE_WRITE=1/0 可取反）。
    // 默认不准——没经过用户同意，本程序不动本机任何权限项。
    let home = root.join(".home");
    let write_allowed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    // 启动报告已经算过的同一个事实：Web 概览要按它区分"本机能力"与"本次实际"（下面那块必定赋值）。
    let allow_fence_write;
    // 工具执行：外层拉起的守门进程就是本程序自己（围栏在它里面装）。
    let tools = adapters::ProcTools::new(
        std::env::current_exe().unwrap_or_default(),
        book.core.tool_texts.clone(),
        home.clone(),
        std::sync::Arc::clone(&write_allowed),
    );
    // 内置文件工具：纯 Rust 直接读写，不经过外部进程（编码问题不进本程序）。
    let io = adapters::FsSysIo::default();
    // 信封修复：只把字符串里的裸控制字符转义（无歧义才修，其余交给模型重发）。
    let repair = adapters::UnambiguousRepair;

    let mut core = match core::Core::new(
        Arc::new(store),
        Arc::new(history),
        Arc::new(workspace),
        Arc::new(source),
        Arc::new(packages),
        Arc::new(adapters::confine::FenceHostAdapter),
        Arc::new(gateway),
        Arc::new(catalog),
        Arc::new(tools),
        Arc::new(io),
        Arc::new(repair),
        Box::new(LoadedPrompts(book)),
        std::sync::Arc::clone(&log),
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[装配失败] {}", e);
            std::process::exit(1);
        }
    };

    // 隐藏模式：实测一条通道支不支持原生工具调用，并把确定结论写回 models.yaml（要真实网络）。
    if let Some(i) = args.iter().position(|a| a == "--probe-tools") {
        let Some(id) = args.get(i + 1) else {
            eprintln!("用法：solomni --probe-tools <模型 id>");
            std::process::exit(2);
        };
        match core.probe_model_tools(id) {
            Ok(core::providers::ProbeOutcome::Supported { detail }) => {
                println!("[探测] 模型 {}：支持原生工具调用（{}）", id, detail);
                println!("[探测] 已把 models.yaml 的 tools 写成 native");
                std::process::exit(0);
            }
            Ok(core::providers::ProbeOutcome::Unsupported { detail }) => {
                println!("[探测] 模型 {}：**不支持**原生工具调用（{}）", id, detail);
                println!("[探测] 已把 models.yaml 的 tools 写成 envelope（手写信封照旧可用，能力没有任何损失）");
                std::process::exit(0);
            }
            Ok(core::providers::ProbeOutcome::Unknown { detail }) => {
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
        match core.probe_replay_shape(id) {
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
                let names = |want: fn(&core::providers::ReplayShape) -> bool| -> String {
                    let got: Vec<&str> = report
                        .shapes
                        .iter()
                        .filter(|s| want(s))
                        .map(|s| s.name.as_str())
                        .collect();
                    if got.is_empty() { "（无）".to_string() } else { got.join(" / ") }
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
            _ => core.app_settings().fence_write,
        };
        write_allowed.store(allow, std::sync::atomic::Ordering::Relaxed);
        allow_fence_write = allow;
        let cap = adapters::confine::capability();
        // 能力与本次实际**分开报**：授权与否决定路径级围栏装不装，但进程树围栏、资源上限与环境白名单
        // 在两种时段都生效（未授权不等于无围栏）。只说"本机能力"会让用户以为未授权时什么都没有。
        let usable = |ok: bool| if ok { "可用" } else { "不可用" };
        let (fs, net) = if allow { (cap.fs, cap.net) } else { (false, false) };
        println!("[围栏] 本机能力：文件系统={} 断网={} 进程树={}（{}）", usable(cap.fs), usable(cap.net), usable(cap.tree), cap.note);
        println!(
            "[围栏] 本次实际：文件系统={} 断网={} 进程树={}；容器授权={}（未授权时只放行进程树与资源上限，不写本机任何权限项；要启用：设置里打开，或 .home/settings.yaml 写 fence_write: true）",
            usable(fs),
            usable(net),
            usable(cap.tree),
            if allow { "已授权" } else { "未授权" }
        );
    }

    let port_flag = |args: &[String]| {
        args.iter()
            .position(|a| a == "--web-port")
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or(presentation::web::DEFAULT_PORT)
    };

    // 核心搬到它自己的执行线程：此后呈现层只持有**入站能力面**——拿不到 Core，也拿不到任何核心锁。
    let handle = match core::api::CoreHandle::spawn(core) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[装配失败] {}", e);
            std::process::exit(1);
        }
    };
    let ops = core::api::Ops::from_handle(&handle);

    if web {
        serve_web(ops, port_flag(&args), std::sync::Arc::clone(&log), allow_fence_write);
    } else {
        // CLI 里输入 webui 可直接转入 Web，无需重启进程（能力面可克隆，两份呈现共用同一个核心）。
        if let presentation::cli::CliExit::Web(port) = presentation::cli::run(ops.clone()) {
            serve_web(ops, port, std::sync::Arc::clone(&log), allow_fence_write);
        }
    }
}

/// 精确回收：按台账撤掉围栏写过的权限项、删掉建过的容器 profile，再按名字前缀扫掉整族遗留 profile
/// （台账可能不存在：探针、夹具的台账被删、旧版本建的）——隐藏模式，用户经文档知道它。
fn fence_clean(root: &std::path::Path) -> i32 {
    let home = root.join(".home");
    let mut lines: Vec<String> = Vec::new();
    let mut failed = false;
    match adapters::confine::clean(&home) {
        Ok(msg) => lines.push(msg),
        Err(e) => {
            lines.push(format!("台账回收未完成：{}", e));
            failed = true;
        }
    }
    match adapters::confine::sweep_profiles() {
        Ok(n) => lines.push(format!("扫掉 {} 个本程序建过的容器 profile", n)),
        Err(e) => {
            lines.push(format!("容器 profile 清扫未完成：{}", e));
            failed = true;
        }
    }
    for line in &lines {
        if failed {
            eprintln!("[围栏] 清理未完成：{}", line);
        } else {
            println!("[围栏] 清理完成：{}", line);
        }
    }
    if failed {
        1
    } else {
        0
    }
}

/// 自检：本机事实（平台 + 围栏能力 + 外部解释器）。只报事实，不猜、不改任何东西（围栏自检那个临时目录除外）。
fn doctor() -> i32 {
    let cap = adapters::confine::capability();
    let doc = serde_json::json!({
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "fence": { "fs": cap.fs, "net": cap.net, "tree": cap.tree, "note": cap.note },
        "externals": {
            "python": find_exe("python"),
            "node": find_exe("node"),
            "curl": find_exe("curl"),
        },
    });
    println!("{}", doc);
    0
}

/// 隐藏模式：用**产品自己的出站代理**（含按平台装配的 TLS）打一次最小 HTTPS 请求，如实报结论。
/// 四态机器可读：ok（通）/ no-net（环境连不上外网）/ env-tls（本进程取不到系统 TLS 凭证，如沙箱挡住凭证存储）/
/// tls-fail|fail（我们链路坏了）。
/// 退出码恒 0：判定归调用方（测试按性质决定 env-skip 还是失败），这里只报事实。
fn https_check(args: &[String], i: usize) -> i32 {
    let url = args.get(i + 1).cloned().unwrap_or_default();
    if url.is_empty() {
        eprintln!("用法：solomni --https-check <https url>");
        return 2;
    }
    let backend = adapters::http_agent::tls_backend();
    let agent = adapters::http_agent::agent(10, 20);
    match agent.get(&url).call() {
        Ok(resp) => {
            println!("[HTTPS] ok {} {} {}", resp.status().as_u16(), backend, url);
            0
        }
        Err(e) => {
            println!("[HTTPS] {} {} {}", adapters::http_agent::classify(&e), backend, e);
            0
        }
    }
}

/// 在 PATH 里找一个可执行文件（找不到就是没有，不去别处翻）。
fn find_exe(name: &str) -> Option<String> {
    let path_var = std::env::var_os("PATH")?;
    let exts: Vec<String> = std::env::var("PATHEXT")
        .map(|v| v.split(';').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect())
        .unwrap_or_else(|_| vec![String::new()]);
    for dir in std::env::split_paths(&path_var) {
        for ext in &exts {
            let candidate = dir.join(format!("{}{}", name, ext.to_lowercase()));
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }
    None
}

/// 装配期已经读好的册子（同一份事实不再读第二遍）。
struct LoadedPrompts(core::prompt::Prompts);

impl core::ports::PromptSource for LoadedPrompts {
    fn load(&self) -> Result<core::prompt::Prompts, String> {
        Ok(self.0.clone())
    }
}

/// 隐藏模式：按 KEY=VALUE 逐行打出运行期给工具进程的环境白名单（入参 = 守门进程那份 JSON）。
fn print_fence_env(args: &[String], flag: usize) -> i32 {
    let raw = args.get(flag + 1).cloned().unwrap_or_default();
    match adapters::confine::FenceJob::from_json(&raw) {
        Ok(job) => {
            for (k, v) in adapters::confine::fence_env(&job.spec) {
                println!("{}={}", k.to_string_lossy(), v.to_string_lossy());
            }
            0
        }
        Err(e) => {
            eprintln!("[围栏] {}", e);
            adapters::confine::FENCE_FAILED
        }
    }
}

/// 隐藏模式：只做机制验证，如实报三态（enforced / env-unavailable / broken），恒退出 0——
/// 判定归调用方（探针按性质决定 env-skip 还是失败）。入参 = 守门进程那份 JSON，`--` 之后是命令。
fn fence_verify(args: &[String], flag: usize) -> i32 {
    let raw = args.get(flag + 1).cloned().unwrap_or_default();
    let command = match args.iter().position(|a| a == "--") {
        Some(j) => args.get(j + 1).cloned().unwrap_or_default(),
        None => String::new(),
    };
    let job = match adapters::confine::FenceJob::from_json(&raw) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("[围栏] {}", e);
            return adapters::confine::FENCE_FAILED;
        }
    };
    match adapters::confine::verify(&job.spec, &command) {
        adapters::confine::FenceVerdict::Enforced => {
            println!("enforced");
            0
        }
        adapters::confine::FenceVerdict::EnvUnavailable(why) => {
            println!("env-unavailable {}", why);
            0
        }
        adapters::confine::FenceVerdict::Broken(why) => {
            println!("broken {}", why);
            0
        }
    }
}

/// 守门模式：读回围栏参数与命令，装围栏 → 跑命令 → 以工具退出码收场（失败如实报错，不静默）。
fn fence_run(args: &[String], flag: usize) -> i32 {
    let raw_job = args.get(flag + 1).cloned().unwrap_or_default();
    let command = match args.iter().position(|a| a == "--") {
        Some(j) => args.get(j + 1).cloned().unwrap_or_default(),
        None => String::new(),
    };
    match adapters::confine::FenceJob::from_json(&raw_job) {
        Ok(job) => adapters::confine::run_fenced(&job, &command),
        Err(e) => {
            eprintln!("[围栏] {}", e);
            adapters::confine::FENCE_FAILED
        }
    }
}

/// 产品根 → 干净的绝对路径：用 current_dir 与传入的根做**纯词法**拼接（不去解析 ..、不碰盘符大小写、不碰盘）。
/// 只有"取不到 current_dir"这种异常情况才退回 canonicalize（并剥掉 Windows 的 \\?\ 扩展长度前缀）。
fn resolve_root(raw: &std::path::Path) -> (PathBuf, Option<String>) {
    match std::env::current_dir() {
        Ok(cwd) => {
            let joined = if raw.is_absolute() { raw.to_path_buf() } else { cwd.join(raw) };
            (lexical_abs(&joined), None)
        }
        Err(e) => match std::fs::canonicalize(raw) {
            Ok(p) => (
                strip_unc_prefix(p),
                Some(format!("取不到当前目录（{}）：改用 canonicalize 规范化产品根", e)),
            ),
            Err(e2) => (
                lexical_abs(raw),
                Some(format!("取不到当前目录（{}），canonicalize 也失败（{}）：产品根可能不是绝对路径", e, e2)),
            ),
        },
    }
}

/// 纯词法归一化：去掉 . 段、收掉重复与尾部分隔符（components 自带），保留盘符/根前缀与 .. 段（不解析）。
fn lexical_abs(p: &std::path::Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::Prefix(_) | std::path::Component::RootDir => out.push(c.as_os_str()),
            std::path::Component::Normal(s) => out.push(s),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => out.push(".."),
        }
    }
    out
}

/// Windows 上 canonicalize 会给出 \\?\C:\… 形式：它对多数工具可用但会污染提示词，去掉这个前缀。
#[cfg(windows)]
fn strip_unc_prefix(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy().into_owned();
    match s.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => p,
    }
}

#[cfg(not(windows))]
fn strip_unc_prefix(p: PathBuf) -> PathBuf {
    p
}

fn serve_web(
    ops: core::api::Ops,
    port: u16,
    log: std::sync::Arc<dyn core::ports::Log + Send + Sync>,
    write_allowed: bool,
) {
    let cap = adapters::confine::capability();
    // 能力与本次实际**分开报**（与启动报告同一套说法）：未授权时路径级围栏是关的，
    // 但进程树与资源上限照旧生效——概览里必须让用户看到这个区别，不能只看"本机能力"。
    let (fs, net) = if write_allowed { (cap.fs, cap.net) } else { (false, false) };
    let fence = presentation::web::FenceInfo {
        fs: cap.fs,
        net: cap.net,
        tree: cap.tree,
        note: cap.note,
        effective_fs: fs,
        effective_net: net,
        write_allowed,
    };
    if let Err(e) = presentation::web::serve(ops, port, log, fence) {
        eprintln!("[Web 服务异常] {}", e);
        std::process::exit(1);
    }
}