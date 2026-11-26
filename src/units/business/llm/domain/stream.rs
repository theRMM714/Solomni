//! 目的：流式一次调用的**累积状态**——协议实现往里写，流机制读它收尾。
//! 管：正文/思维链的增量与累积快照兼容、原生调用按下标累积、usage 分块按字段合并、结束原因。
//! 不管：SSE 分行与超时（在 detail/http_stream）；字段名与 JSON 形状（在 detail/protocol）；回执语义（在 api）。
//! 联动：detail/protocol 的实现调 absorb_* / call_slot；http_stream 读 is_blank / done_completion。

use crate::units::business::llm::api::{Completion, TokenUsage, ToolCall};

/// 目的：一次流式调用累积到现在的全部事实。
#[derive(Default)]
pub struct StreamAcc {
    pub content: String,
    pub reasoning: String,
    pub finish: String,
    pub calls: Vec<ToolCall>,
    pub usage: Option<TokenUsage>,
    /// 目的：这次流里到底来过东西没有（正文 / 思维链 / 调用都算）：决定超时与读失败该不该重试。
    pub got_any: bool,
}

impl StreamAcc {
    /// 目的：并入一段正文。供应商有两种写法：只给增量，或给"到目前为止的全部"——
    ///    后者去掉已知前缀，避免正文重复一遍（同一段话被拼两遍会让模型看到自相矛盾的上下文）。
    pub fn absorb_text(&mut self, raw: &str) -> Option<String> {
        self.absorb(&mut Field::Text, raw)
    }

    /// 目的：同上，思维链。
    pub fn absorb_reasoning(&mut self, raw: &str) -> Option<String> {
        self.absorb(&mut Field::Reasoning, raw)
    }

    /// 目的：并入一份用量：分块可能只报一半字段，新值**并进**已有值，不拿半份覆盖整份。
    pub fn absorb_usage(&mut self, usage: TokenUsage) {
        self.usage = Some(match self.usage {
            Some(known) => known.widen(usage),
            None => usage,
        });
    }

    /// 目的：取（必要时扩容）第 idx 个原生调用槽：流式分块按 id/name/arguments 三路各自到达。
    pub fn call_slot(&mut self, idx: usize) -> &mut ToolCall {
        while self.calls.len() <= idx {
            self.calls.push(ToolCall {
                id: String::new(),
                name: String::new(),
                args_json: String::new(),
            });
        }
        &mut self.calls[idx]
    }

    /// 目的：记下供应商给的结束原因（空串 = 没给，不覆盖已知值）。
    pub fn set_finish(&mut self, finish: &str) {
        if !finish.is_empty() {
            self.finish = finish.to_string();
        }
    }

    /// 目的：这次流一个字节的有效内容都没有（要按"没有正文"处理，而不是当成空回复）。
    pub fn is_blank(&self) -> bool {
        self.content.is_empty() && self.reasoning.is_empty() && self.calls.is_empty()
    }

    /// 目的：流读完了：交回一次补全。
    pub fn done_completion(self) -> Completion {
        Completion {
            raw: self.content,
            reasoning: self.reasoning,
            finish: self.finish,
            calls: self.calls,
            error: None,
            cancelled: false,
            usage: self.usage,
        }
    }

    /// 目的：用户停止了：已产出的部分原样带回，标记为取消（上层据此不执行任何工具）。
    pub fn cancelled_completion(self) -> Completion {
        Completion {
            raw: self.content,
            reasoning: self.reasoning,
            finish: self.finish,
            calls: self.calls,
            error: None,
            cancelled: true,
            usage: self.usage,
        }
    }

    fn absorb(&mut self, field: &mut Field, raw: &str) -> Option<String> {
        if raw.is_empty() {
            return None;
        }
        self.got_any = true;
        let known = match field {
            Field::Text => &mut self.content,
            Field::Reasoning => &mut self.reasoning,
        };
        let piece = if raw.len() > known.len() && raw.starts_with(known.as_str()) {
            raw[known.len()..].to_string()
        } else {
            raw.to_string()
        };
        known.push_str(&piece);
        (!piece.is_empty()).then_some(piece)
    }
}

/// 目的：absorb 往哪个字段写（两处逻辑一模一样，只差目标串）。
enum Field {
    Text,
    Reasoning,
}
