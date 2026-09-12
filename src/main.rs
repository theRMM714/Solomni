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
    let root = args
        .iter()
        .position(|a| a == "--root")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    // 组合根：唯一允许 new 具体适配器的地方（依赖注入）。
    let log: std::sync::Arc<dyn core::ports::Log + Send + Sync> = match adapters::FileLog::new(&root, "Solomni 运行日志") {
        Ok(l) => std::sync::Arc::new(l),
        Err(e) => {
            eprintln!("[日志系统异常] {}（进程继续，日志降级为 stderr）", e);
            std::sync::Arc::new(core::ports::NoopLog)
        }
    };
    let store = adapters::YamlRegistryStore::new(root.join(".home").join("providers.yaml"));
    let source = adapters::FsModules::new(root.join("modules"));
    let gateway = adapters::HttpGateway::with_log(std::sync::Arc::clone(&log));
    let prompts = adapters::YamlPrompts::new(root.join("prompts.yaml"));

    let core = match core::Core::new(
        Arc::new(store),
        Arc::new(source),
        Arc::new(gateway),
        Box::new(prompts),
        std::sync::Arc::clone(&log),
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[装配失败] {}", e);
            std::process::exit(1);
        }
    };

    if web {
        let shared = Arc::new(Mutex::new(core));
        let port = args
            .iter()
            .position(|a| a == "--web-port")
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or(3081);
        if let Err(e) = presentation::web::serve(shared, port, std::sync::Arc::clone(&log)) {
            eprintln!("[Web 服务异常] {}", e);
            std::process::exit(1);
        }
    } else {
        presentation::cli::run(core);
    }
}