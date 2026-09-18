//! Solomni 组合根 + 入口。
//! 组合根职责：创建各适配器实例 → 注入 Core 门面 → 交给呈现层（CLI 或 Web）。
//! 依赖方向：main → adapters / core / presentation；core 不知道后两者存在。

mod adapters;
mod core;
mod presentation;

#[cfg(test)]
mod contract_tests;
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
    // 入站契约（机器可读）：HTTP 路由目录的唯一定义（见 ARCHITECTURE.md「呈现层入站契约」）。
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
    // 工具执行：外层拉起的守门进程就是本程序自己（围栏在它里面装）。
    let tools = adapters::ProcTools::new(
        std::env::current_exe().unwrap_or_default(),
        book.core.tool_texts.clone(),
        home.clone(),
        std::sync::Arc::clone(&write_allowed),
    );
    // 内置文件工具：纯 Rust 直接读写，不经过外部进程（编码问题不进本程序）。
    let io = adapters::FsSysIo::default();

    let core = match core::Core::new(
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
        Box::new(LoadedPrompts(book)),
        std::sync::Arc::clone(&log),
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[装配失败] {}", e);
            std::process::exit(1);
        }
    };

    // 写权限开关定稿：环境变量优先（测试/CI 用得到），否则看设置。
    {
        let env_flag = std::env::var("SOLOMNI_FENCE_WRITE").ok();
        let allow = match env_flag.as_deref() {
            Some("1") => true,
            Some("0") => false,
            _ => core.app_settings().fence_write,
        };
        write_allowed.store(allow, std::sync::atomic::Ordering::Relaxed);
        let cap = adapters::confine::capability();
        println!(
            "[围栏] 本机能力：文件系统={} 断网={} 进程树={}；写权限={}（{}）",
            cap.fs,
            cap.net,
            cap.tree,
            if allow { "已授权" } else { "未授权" },
            if allow { cap.note.as_str() } else { "未授权时段：外部工具按无围栏执行；要启用请设 fence_write: true" }
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
        serve_web(ops, port_flag(&args), std::sync::Arc::clone(&log));
    } else {
        // CLI 里输入 webui 可直接转入 Web，无需重启进程（能力面可克隆，两份呈现共用同一个核心）。
        if let presentation::cli::CliExit::Web(port) = presentation::cli::run(ops.clone()) {
            serve_web(ops, port, std::sync::Arc::clone(&log));
        }
    }
}

/// 精确回收：按台账撤掉围栏写过的权限项、删掉建过的容器 profile（隐藏模式，用户经文档知道它）。
fn fence_clean(root: &std::path::Path) -> i32 {
    let home = root.join(".home");
    match adapters::confine::clean(&home) {
        Ok(msg) => {
            println!("[围栏] 清理完成：{}", msg);
            0
        }
        Err(e) => {
            eprintln!("[围栏] 清理失败：{}", e);
            1
        }
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

/// 守门模式：读回围栏参数与命令，装围栏 → 跑命令 → 以工具退出码收场（失败如实报错，不静默）。
fn fence_run(args: &[String], flag: usize) -> i32 {
    let spec_json = args.get(flag + 1).cloned().unwrap_or_default();
    let command = match args.iter().position(|a| a == "--") {
        Some(j) => args.get(j + 1).cloned().unwrap_or_default(),
        None => String::new(),
    };
    match core::fence::FenceSpec::from_json(&spec_json) {
        Ok(spec) => adapters::confine::run_fenced(&spec, &command),
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

fn serve_web(ops: core::api::Ops, port: u16, log: std::sync::Arc<dyn core::ports::Log + Send + Sync>) {
    let cap = adapters::confine::capability();
    let fence = presentation::web::FenceInfo { fs: cap.fs, net: cap.net, tree: cap.tree, note: cap.note };
    if let Err(e) = presentation::web::serve(ops, port, log, fence) {
        eprintln!("[Web 服务异常] {}", e);
        std::process::exit(1);
    }
}