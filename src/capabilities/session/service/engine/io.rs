//! 目的：回合引擎的端口与结论——一次回合要用的 IO 槽位（`TurnIo`）与两类回合结果（`MemberTurn` / `CompactedTurn`）。
//! 管：`TurnIo` 的三个槽位（调用参数 / 裁决队 / 提问端口）；`MemberTurn` 的结论形态与构造；`CompactedTurn` 的压缩结果。
//! 不管：会话状态（在 `domain/`）；模型调用与工具执行机制（在 `llm` / `tools` 的端口后）；取消开关与短暂事件出口（按调用形参传，不进结构）。
//! 联动：`driver` / `round` / `tools` 读 `TurnIo`；协作能力与 turn 的生成流程消费 `MemberTurn`；`CompactedTurn` 由 `driver::compact_session` 产出。

use crate::capabilities::llm::api::{LlmOpts, Verb};
use crate::capabilities::session::api::{ContextUsage, LineView};
use crate::kernel::ports::AskUser;
use std::sync::Arc;

/// 目的：一次回合要用的 IO 槽位：调用参数 / 本会话的裁决队 / 工具层提问端口。
/// 约束：`emit`（短暂事件出口）与 `cancel`（停止开关）**不进结构**：它们按调用形参传，
///   结构本身于是可克隆、可跨线程共享，也不用为借用期写生命周期。
#[derive(Clone)]
pub struct TurnIo {
    /// 目的：本回合的调用参数（流式与预算，来自全局设置）。
    pub llm: LlmOpts,
    /// 目的：本会话的裁决队（工具级确认这类"等在工作线程上"的关进它）。
    ///   `None` = 这一趟不接工具级确认（照常执行）——测试、讨论席与没有交互前端的生成都走它。
    pub decisions: Option<Arc<dyn crate::capabilities::session::api::Gates>>,
    /// 目的：本会话的**提问端口**（工具执行层与围栏经它请用户裁决）：走同一条裁决队、**阻塞**等回答。
    ///   `None` = 这一趟没有可回答的前端（CLI 非交互、端到端夹具、测试、讨论席）——按 fail-closed 拒绝。
    pub ask: Option<Arc<dyn AskUser>>,
}

/// 目的：一个成员回合的结果：两套通道**统一形态**（上层不必再关心是哪条通道）。
pub struct MemberTurn {
    /// 目的：这一回合的表态；**None = 没表态**（散文 / 空回执 / 越权 / 到顶）。
    ///   没表态**不投影主会话**：原文只落进它自己的会话；提醒由核心在边界注入。
    pub verb: Option<Verb>,
    pub text: String,
    pub degraded: bool,
    /// 目的：这一轮的输出被供应商按长度截断了（如实标注，不假装完整）。
    pub truncated: bool,
    /// 目的：这一回合里**核实**留下的行（只读工具调用）。
    ///   内存通道把它并进讨论转录；核心驱动时由核心逐轮落进**该 agent 自己的会话**。
    #[cfg(test)]
    pub lines: Vec<LineView>,
}

impl MemberTurn {
    /// 目的：一回合的**结论**：表态 / 正文 + 两个如实标记。
    ///   思维链与核实行不在这里——它们随行落进**各自的会话**（内存测试通道另走它自己的构造点）。
    pub fn verdict(
        verb: Option<Verb>,
        text: String,
        degraded: bool,
        truncated: bool,
    ) -> MemberTurn {
        MemberTurn {
            verb,
            text,
            degraded,
            truncated,
            #[cfg(test)]
            lines: Vec::new(),
        }
    }
}

/// 目的：一次压缩的完整结果（记录行 + 用量）。
/// 约束：调用方负责推事件 / 落盘。
pub struct CompactedTurn {
    /// 目的：压缩记录行（摘要 + up_to）。
    pub line: LineView,
    /// 目的：压完后的发送视图估算用量。
    pub usage: ContextUsage,
}
