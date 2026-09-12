//! 直连与全能会话：上下文历史自有，通道来自注入的网关。

use crate::core::envelope;
use crate::core::events::SessionEvent;
use crate::core::ports::{BoxedChat, Msg};

/// 模式一：单模块直连（历史含职责提示词首条）。
pub struct DirectSession {
    id: String,
    history: Vec<Msg>,
    chat: BoxedChat,
    note: Option<String>,
}

impl DirectSession {
    pub fn new(id: &str, system: String, chat: BoxedChat, note: Option<String>) -> DirectSession {
        DirectSession { id: id.to_string(), history: vec![Msg::system(system)], chat, note }
    }

    /// 开场事件（通道回落告知）。
    pub fn open(&self) -> Vec<SessionEvent> {
        self.note.clone().map(|n| vec![SessionEvent::Notice(n)]).unwrap_or_default()
    }

    pub fn say(&mut self, text: &str) -> SessionEvent {
        self.history.push(Msg::user(text.to_string()));
        let raw = self.chat.complete(&self.history);
        let reply = envelope::parse(&raw);
        self.history.push(Msg::assistant(reply.text.clone()));
        SessionEvent::Transcript(vec![format!("[{}] {}", self.id, reply.text)])
    }
}

/// 模式三：全能（历史含拼装职责提示词首条）。
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