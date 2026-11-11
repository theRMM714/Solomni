//! 目的：存储空间的机器可读报告（隐藏模式 --storage）。
//! 管：参数解析与把能力面报告打成 JSON。
//! 不管：盘面盘点与回收策略（在 storage 能力）；报告的人类可读打印（在 gc 探针）。
//! 联动：由 main 在 --storage 分支调用；装配在 entry::assembly。

use crate::capabilities::storage::api::Storage as _;

/// 目的：存储空间的机器可读报告（--storage）。
pub fn storage(root: &std::path::Path) -> i32 {
    match crate::entry::assembly::storage_from_settings(root).report() {
        Ok(report) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&report).unwrap_or_default()
            );
            0
        }
        Err(e) => {
            eprintln!("[存储] {}", e);
            1
        }
    }
}
