//! 目的：`work_reset` 的请求与报告形状——把沙箱按工作线头恢复，或把工作线移回某个提交。
//! 管：`ResetRequest` / `ResetReport` 的纯数据。
//! 不管：铺盘与备份（在 `service/reset.rs`）、回执文案（在 `domain/work_tools.rs`）。
//! 联动：`workspace::api` 重导出；`Service` 的 `work_reset` 消费。

/// 目的：一次 `work_reset`：只恢复 `paths`（同 `git restore`），或把工作线移到 `to`（同 `git reset`）。
/// 约束：`paths` 与 `to` 不能同时给；两个都不给 = 调用方漏参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetRequest {
    /// 目的：只把这些路径恢复成工作线头的内容（沙箱里的其他文件不动）。
    pub paths: Vec<String>,
    /// 目的：把工作线移到这条提交（必须在这条线上）。
    pub to: Option<u64>,
    /// 目的：只移工作线头、不改沙箱（同 `git reset --soft`）；`paths` 形态不看这一项。
    pub keep_changes: bool,
}

/// 目的：一次 `work_reset` 的结果：新工作线头 + 哪些路径被铺回 / 删除 + 备份了几份旧内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetReport {
    /// 目的：移动后的工作线头（`paths` 形态 = 原头，不动）。
    pub moved_to: Option<u64>,
    /// 目的：铺回沙箱的路径。
    pub restored: Vec<String>,
    /// 目的：从沙箱删掉的路径（工作线里已经没有）。
    pub removed: Vec<String>,
    /// 目的：被覆盖前备份到 `lost/` 的份数（0 = 没有内容被丢弃）。
    pub backed_up: usize,
}
