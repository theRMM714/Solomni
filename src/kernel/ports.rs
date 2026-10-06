//! 目的：机制端口——全项目共享的机制接口。
//! 管：`Log` / `ToolHandler` / `HostProbe` 等端口的 trait 定义与它们的纯数据形态。
//! 不管：机制实现（文件、时间戳、目录在 `detail/`）；谁在什么时机调用（策略在调用方）；某个能力私有的端口（那是它自己的 `ports.rs`）。
//! 联动：实现见 `src/kernel/detail/`；按 R12 的例外全项目共享，不属于任何能力。

/// 目的：运行日志端口（三级）。
pub trait Log: Send + Sync {
    fn info(&self, at: &str, msg: &str);
    fn warn(&self, at: &str, msg: &str);
    fn error(&self, at: &str, msg: &str);
}

/// 目的：测试与纯逻辑场景的无声日志（不落任何盘）。
pub struct NoopLog;
impl Log for NoopLog {
    fn info(&self, _at: &str, _msg: &str) {}
    fn warn(&self, _at: &str, _msg: &str) {}
    fn error(&self, _at: &str, _msg: &str) {}
}

use crate::kernel::domain::types::ToolOutcome;
use std::path::Path;

/// 目的：一类工具的执行者——**按名字认领**，不靠“内置 / 模块”的两分法。
/// 约束：成员循环只问“这一回合的工具面里有没有它、谁认领它”，三类工具走同一条派发路径；它是 R12 的例外（全项目共享）。
pub trait ToolHandler: Send + Sync {
    /// 这个名字归不归我（只看名字；模块归属另由模块工具那条路判）。
    fn owns(&self, name: &str) -> bool;
    /// 跑一次调用；上下文见 `ToolCtx`。
    fn run(&self, ctx: &ToolCtx, name: &str, args_json: &str) -> ToolOutcome;
}

/// 目的：核心自有工具的执行上下文——这一席是谁、属于哪个工作、**提交锚在哪一行**。
/// 约束：行锚让共享区提交能按转录行精确定位，回档才能把共享区物化回那一刻。
pub struct ToolCtx<'a> {
    /// 目的：顶层工作名（共享区与版本库的归属）。
    pub work: &'a str,
    /// 目的：这一席的 agent 实例名。
    pub agent: &'a str,
    /// 目的：下一条转录行的 id。
    pub line: u64,
}

pub trait HostProbe: Send + Sync {
    /// 这个路径存在且是文件。
    fn is_file(&self, path: &Path) -> bool;
    /// 这个路径存在且是目录。
    fn is_dir(&self, path: &Path) -> bool;
    /// PATH 上有没有这个可执行文件（只查存在性，不执行它；平台扩展名由适配层处理）。
    fn has_exe(&self, name: &str) -> bool;
    /// 本机能不能起硬件虚拟化（只问事实，不起任何虚拟机）。
    fn hypervisor_available(&self) -> bool;
}
