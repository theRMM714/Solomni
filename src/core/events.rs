//! 呈现侧契约：事件与介入请求。词汇定义在 core，前端按此渲染。
//! 呈现即上下文：转录行与核心记录完全一致。

/// 会话事件：驱动前端渲染；转录行为增量，前端按序累积。
/// 预留字段说明：DiscussionDone 的 round/over_cap 供 Web 前端做裁决确认页（CLI 暂不渲染）。
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum SessionEvent {
    /// 状态提示（通道回落、建组、返工、上限等）。
    Notice(String),
    /// 转录新增行。
    Transcript(Vec<String>),
    /// 讨论收敛（over_cap = 轮次超限，需用户裁决）。
    DiscussionDone { round: usize, over_cap: bool },
    /// 整理方案就绪。
    Plan(String),
    /// 成员执行回报（rework = 第几轮执行，0 为首轮）。
    Report { id: String, text: String, rework: usize },
    /// 验收清单（raw = 解析失败时的原文）。
    Review { items: Vec<CheckView>, raw: String },
    /// 交付结论（over_rework = 返工超限交用户裁决）。
    Delivery { ok: bool, over_rework: bool },
    /// 会话结束。
    Ended,
}

/// 验收条目的呈现视图。
#[derive(Debug, Clone)]
pub struct CheckView {
    pub item: String,
    pub status: String,
    pub note: String,
}

/// 用户介入请求：会话暂停，等前端回应。
#[derive(Debug, Clone)]
pub enum Pending {
    /// 模块请教用户（yes,allow 自裁模式下不会出现）。
    Ask { member: String, question: String },
    /// 核心代拟名单待确认。
    ConfirmSlate,
    /// 名单已定，等用户确认开始讨论（可授权自裁）。
    ConfirmBegin,
}