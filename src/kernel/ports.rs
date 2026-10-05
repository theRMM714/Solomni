//! 机制端口：**全项目共享**的机制接口（R12 的例外——它们不属于某个能力，任何能力/适配器都可持有）。
//! 实现（机制）在 `kernel/detail/`；策略在调用方。

//! 只被调用；文件、时间戳、目录机制在 `kernel/detail/file_log.rs`。

/// 运行日志端口（三级）。
pub trait Log: Send + Sync {
    fn info(&self, at: &str, msg: &str);
    fn warn(&self, at: &str, msg: &str);
    fn error(&self, at: &str, msg: &str);
}

/// 测试与纯逻辑场景的无声日志（不落任何盘）。
pub struct NoopLog;
impl Log for NoopLog {
    fn info(&self, _at: &str, _msg: &str) {}
    fn warn(&self, _at: &str, _msg: &str) {}
    fn error(&self, _at: &str, _msg: &str) {}
}

use crate::kernel::domain::types::ToolOutcome;
use std::path::Path;

/// 一类工具的执行者：**按名字认领**，不靠“内置 / 模块”的两分法。
/// 成员循环因此只问“这一回合的工具面里有没有它、谁认领它”，内置、模块与核心自有工具
/// 走同一条派发路径——加一类工具不再改循环。为什么在 kernel：它是全项目共享的机制接口
/// （R12 的例外），kernel 不认识任何业务。
pub trait ToolHandler: Send + Sync {
    /// 这个名字归不归我（只看名字；模块归属另由模块工具那条路判）。
    fn owns(&self, name: &str) -> bool;
    /// 跑一次调用；上下文见 `ToolCtx`。
    fn run(&self, ctx: &ToolCtx, name: &str, args_json: &str) -> ToolOutcome;
}

/// 核心自有工具的执行上下文：这一席是谁、属于哪个工作、**提交锚在哪一行**。
/// 行锚让共享区提交能按转录行精确定位，回档才能把共享区物化回那一刻。
pub struct ToolCtx<'a> {
    /// 顶层工作名（共享区与版本库的归属）。
    pub work: &'a str,
    /// 这一席的 agent 实例名。
    pub agent: &'a str,
    /// 下一条转录行的 id。
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
