//! Solomni 组合根 + 启动器。
//! 组合根职责：创建各适配器实例 → 注入 Core 门面 → 交给呈现层。
//! 依赖方向：main → adapters / core / presentation；core 不知道后两者存在。

mod adapters;
mod core;
mod presentation;

#[cfg(test)]
mod tests;

use std::path::PathBuf;

fn main() {
    let root = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));

    // 组合根：唯一允许 new 具体适配器的地方（依赖注入）。
    let store = adapters::YamlRegistryStore::new(root.join(".home").join("providers.yaml"));
    let source = adapters::FsModules::new(root.join("modules"));
    let gateway = adapters::HttpGateway;
    let prompts = adapters::YamlPrompts::new(root.join("prompts.yaml"));

    let core = match core::Core::new(Box::new(store), Box::new(source), Box::new(gateway), Box::new(prompts)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[装配失败] {}", e);
            std::process::exit(1);
        }
    };
    presentation::cli::run(core);
}