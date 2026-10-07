//! 目的：机制端口——全项目共享的机制接口。
//! 管：`Log` / `ToolHandler` / `HostProbe` / `AskUser` 等端口的 trait 定义与它们的纯数据形态。
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

use crate::kernel::domain::types::{Ask, ToolOutcome};
use std::path::Path;

/// 目的：**提问端口**——需要用户裁决的机制（围栏、工具执行层，今后任何 yes/no）经它推一条问题，
///   并**阻塞**等回答（走会话的**统一裁决通道**：同一条队、同一张卡、同一条回答命令）。
/// 约束：它是全项目共享的机制接口（R12 的例外，与 `Log` / `ToolHandler` 同一类）；
///   实现方负责**阻塞**、把用户选中的**选项 id** 原样带回，并在没有可回答的前端时按 fail-closed 收场。
pub trait AskUser: Send + Sync {
    /// 目的：把这条问题推给用户并**阻塞**等他答（不设超时——不点不继续）。
    /// 参数：`ask` = 谁在问 + 消息三段 + 选项集；选项集**不得为空**（契约禁止置灰）。
    /// 返回：用户选中的**选项 id**；拒绝、没人答（停会话解成拒绝）、以及**构不出可用选项**
    ///   （空选项集，端口已按契约停掉这个会话并落一条警告）一律 `None`——调用方按 fail-closed 处置。
    fn ask(&self, ask: &Ask) -> Option<String>;

    /// 目的：这一环**构不出可用选项**时的收场：**不发起裁决**，改为**停掉这个会话 + 落一条警告**。
    /// 参数：`why` = 哪一环做不下去、要补什么（请求方给的原话，写进那条警告）。
    fn halt(&self, why: &str);
}

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
