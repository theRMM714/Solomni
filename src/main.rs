//! Solomni 组合根 + 入口。
//! 组合根职责：创建各适配器实例 → 注入 Core 门面 → 交给呈现层（CLI 或 Web）。
//! 依赖方向：main → adapters / core / presentation；core 不知道后两者存在。

mod adapters;
mod core;
mod presentation;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn main() {
    let args: Vec<String> = std::env::args().collect();
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
    let store = adapters::YamlSettingsStore::new(
        root.join(".home").join("providers.yaml"),
        root.join(".home").join("models.yaml"),
        root.join(".home").join("settings.yaml"),
        root.join(".home").join("agents.yaml"),
    );
    let history = adapters::FsHistory::new(root.join("session"));
    let workspace = adapters::FsWorkspace::new(root.join("session"));
    let source = adapters::FsModules::new(root.join("modules"));
    // 端点记忆：谁先通了就固定谁，后续会话不再反复探测候选。
    let memo = adapters::endpoint::memo_new();
    let gateway = adapters::HttpGateway::with_log(std::sync::Arc::clone(&log), std::sync::Arc::clone(&memo));
    let catalog = adapters::HttpModelCatalog::with_log(std::sync::Arc::clone(&log), std::sync::Arc::clone(&memo));
    let tools = adapters::ProcTools::default();
    // 内置文件工具：纯 Rust 直接读写，不经过外部进程（编码问题不进本程序）。
    let io = adapters::FsSysIo::default();
    let prompts = adapters::YamlPrompts::new(root.join("prompts.yaml"));

    let core = match core::Core::new(
        Arc::new(store),
        Arc::new(history),
        Arc::new(workspace),
        Arc::new(source),
        Arc::new(gateway),
        Arc::new(catalog),
        Arc::new(tools),
        Arc::new(io),
        Box::new(prompts),
        std::sync::Arc::clone(&log),
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[装配失败] {}", e);
            std::process::exit(1);
        }
    };

    let port_flag = |args: &[String]| {
        args.iter()
            .position(|a| a == "--web-port")
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or(presentation::web::DEFAULT_PORT)
    };

    if web {
        serve_web(core, port_flag(&args), std::sync::Arc::clone(&log));
    } else {
        // CLI 里输入 webui 可直接转入 Web，无需重启进程。
        let (core, exit) = presentation::cli::run(core);
        if let presentation::cli::CliExit::Web(port) = exit {
            serve_web(core, port, std::sync::Arc::clone(&log));
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

fn serve_web(core: core::Core, port: u16, log: std::sync::Arc<dyn core::ports::Log + Send + Sync>) {
    let shared = Arc::new(Mutex::new(core));
    if let Err(e) = presentation::web::serve(shared, port, log) {
        eprintln!("[Web 服务异常] {}", e);
        std::process::exit(1);
    }
}