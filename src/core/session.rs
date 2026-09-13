//! 直连与全能会话：上下文历史自有，通道来自注入的网关。
//! 直连支持工具循环（联动 engine::converse）：声明了工具的模块在对话中可申请执行。

use crate::core::engine::{MemberTools, MAX_TOOL_CALLS};
use crate::core::envelope;
use crate::core::events::SessionEvent;
use crate::core::ports::{BoxedChat, Msg};

/// 模式一：单模块直连（历史含职责提示词首条）。
pub struct DirectSession {
    id: String,
    history: Vec<Msg>,
    chat: BoxedChat,
    note: Option<String>,
    /// 工具环境；None = 本模块未声明工具。
    tools: Option<MemberTools>,
}

impl DirectSession {
    pub fn new(id: &str, system: String, chat: BoxedChat, note: Option<String>, tools: Option<MemberTools>) -> DirectSession {
        DirectSession { id: id.to_string(), history: vec![Msg::system(system)], chat, note, tools }
    }

    /// 开场事件（通道回落告知）。
    pub fn open(&self) -> Vec<SessionEvent> {
        self.note.clone().map(|n| vec![SessionEvent::Notice(n)]).unwrap_or_default()
    }

    /// 发言：走工具循环直到最终答复；工具轨迹以 Notice 如实呈现，全程并入历史。
    pub fn say(&mut self, text: &str) -> Vec<SessionEvent> {
        self.history.push(Msg::user(text.to_string()));
        // 拆字段借用：chat 可变与 history/tools 只读互不冲突。
        let DirectSession { id, history, chat, tools, .. } = self;
        let system = history[0].content.clone();
        let (reply, trace, exchange) =
            crate::core::engine::converse(&system, chat.as_mut(), tools.as_ref(), history.last().cloned().expect("刚压入用户消息"));
        // 交换全程并入历史（assistant 原文 + 工具结果），最终答复补压入，上下文完整可追溯。
        for msg in exchange.into_iter().skip(2) {
            history.push(msg);
        }
        history.push(Msg::assistant(reply.clone()));
        let mut out: Vec<SessionEvent> = trace
            .into_iter()
            .map(|t| SessionEvent::Notice(format!("[{}:工具] {}", id, t)))
            .collect();
        out.push(SessionEvent::Transcript(vec![format!("[{}] {}", id, reply)]));
        out
    }
}

/// 模式三：全能（历史含拼装职责提示词首条；核心声部，不接工具循环）。
pub struct OmniSession {
    history: Vec<Msg>,
    chat: BoxedChat,
    note: Option<String>,
}

impl OmniSession {
    pub fn new(system: String, chat: BoxedChat, note: Option<String>) -> OmniSession {
        OmniSession { history: vec![Msg::system(system)], chat, note }
    }

    pub fn open(&self) -> Vec<SessionEvent> {
        self.note.clone().map(|n| vec![SessionEvent::Notice(n)]).unwrap_or_default()
    }

    pub fn say(&mut self, text: &str) -> SessionEvent {
        self.history.push(Msg::user(text.to_string()));
        let raw = self.chat.complete(&self.history);
        let reply = envelope::parse(&raw);
        // tool 信封在此形态无执行机制：parse 已让 text = 原始输出，如实收录。
        self.history.push(Msg::assistant(reply.text.clone()));
        SessionEvent::Transcript(vec![format!("[全能] {}", reply.text)])
    }
}
// 测试访问器：验证职责提示词已入历史首条（回归：直连/全能曾丢失 system 提示词）。
#[cfg(test)]
impl DirectSession {
    pub fn history(&self) -> &[Msg] { &self.history }
}

#[cfg(test)]
impl OmniSession {
    pub fn history(&self) -> &[Msg] { &self.history }
}

// MAX_TOOL_CALLS 供引擎循环使用；此处引用以保持常量归属清晰（直连上限与执行一致）。
const _: () = { let _ = MAX_TOOL_CALLS; };
