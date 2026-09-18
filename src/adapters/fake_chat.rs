//! 演示/测试通道：脚本假模型（无网络、无密钥）。
//! 双用途：核心回落演示（无可用模型通道时，如实告知）+ 单元测试脚本回放。
//! 只实现 core 的 Chat/ChatGateway 端口，不做装配决策。

use crate::core::ports::{BoxedChat, Chat, ChatGateway, Chunk, CompleteOpts, Completion, Msg};
use crate::core::providers::Channel;

/// 脚本假模型：按调用次序回放脚本（最后一个条目重复兜底）；记录调用供测试断言。
pub struct FakeChat {
    script: Vec<String>,
    pub calls: Vec<Vec<Msg>>,
}

impl FakeChat {
    pub fn new(script: Vec<String>) -> FakeChat {
        FakeChat { script, calls: Vec::new() }
    }

    fn next(&mut self) -> String {
        if self.script.len() > 1 {
            self.script.remove(0)
        } else {
            self.script.first().cloned().unwrap_or_default()
        }
    }
}

impl Chat for FakeChat {
    /// 兑现 Chat 端口的流式与中止契约（与 http_chat 同一套语义）：
    /// stream = false 不回调；stream = true 先发 Chunk::Start，再发一条 Chunk::Text（整条脚本）。
    /// on 返回 false = 调用方要求中止，立即停止回调并返回已产出的正文（Start 阶段中止则返回空串）。
    fn complete(&mut self, messages: &[Msg], opts: CompleteOpts<'_>, on: &mut dyn FnMut(Chunk) -> bool) -> Completion {
        self.calls.push(messages.to_vec());
        let stream = opts.stream;
        let text = self.next();
        if !stream {
            return Completion::text(text);
        }
        if !on(Chunk::Start) {
            return Completion::text("");
        }
        if !text.is_empty() {
            let _ = on(Chunk::Text(text.clone()));
        }
        Completion::text(text)
    }
}

/// 演示网关：一律发演示通道（无可用模型回落 / 测试装配两用）。
pub struct DemoGateway;

impl ChatGateway for DemoGateway {
    /// 演示通道没有真实供应商可测：如实说"测不了"，不假装测过。
    fn probe_tools(&self, _channel: &Channel) -> Result<crate::core::ports::ProbeOutcome, String> {
        Err("演示通道没有真实供应商，测不了工具调用支持".to_string())
    }

    fn member_channel(&self, _channel: Option<&Channel>, module_id: &str) -> (BoxedChat, Option<String>) {
        (
            Box::new(FakeChat::new(vec![format!(
                "{{\"type\":\"say\",\"text\":\"（演示）{} 收到。\"}}",
                module_id
            )])),
            Some(format!("[{}] 未配置模型通道，使用内置假模型演示", module_id)),
        )
    }

    fn core_channel(&self, _channel: Option<&Channel>) -> (BoxedChat, bool) {
        (
            Box::new(FakeChat::new(vec![
                "{\"type\":\"say\",\"text\":\"（演示）核心通道。\"}".to_string(),
                "[{\"item\":\"演示项\",\"status\":\"pass\",\"evidence\":\"演示\"}]".to_string(),
            ])),
            true,
        )
    }
}
