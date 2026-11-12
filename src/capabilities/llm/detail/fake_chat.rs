//! 目的：测试专用的脚本假模型通道（无网络、无密钥）。
//! 管：按脚本回放的 Chat 与 ChatGateway 实现；记录收到的消息与用量。
//! 不管：生产装配（组合根不装配）；真实网络、密钥与通道发现。
//! 联动：只被单元测试与测试替身引用；实现 api::Chat 与 ports::ChatGateway；detail/mod.rs 以 cfg(test) 声明。

use crate::capabilities::llm::api::Channel;
use crate::capabilities::llm::api::{
    BoxedChat, Chat, Chunk, CompleteOpts, Completion, Msg, TokenUsage,
};
use crate::capabilities::llm::ports::ChatGateway;

/// 目的：脚本假模型：按调用次序回放脚本（最后一个条目重复兜底）；记录调用供测试断言。
pub struct FakeChat {
    script: Vec<String>,
    pub calls: Vec<Vec<Msg>>,
    /// 脚本替身要验“精确用量”时注入；None = 这条通道拿不到 usage。
    usage: Option<TokenUsage>,
}

impl FakeChat {
    pub fn new(script: Vec<String>) -> FakeChat {
        FakeChat {
            script,
            calls: Vec::new(),
            usage: None,
        }
    }

    /// 目的：让脚本替身每次调用都报同一份 usage（T1/T2 验状态栏与压缩阈值用）。
    pub fn with_usage(mut self, usage: TokenUsage) -> FakeChat {
        self.usage = Some(usage);
        self
    }

    fn done(&self, text: String) -> Completion {
        let mut done = Completion::text(text);
        done.usage = self.usage;
        done
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
    fn complete(
        &mut self,
        messages: &[Msg],
        opts: CompleteOpts<'_>,
        on: &mut dyn FnMut(Chunk) -> bool,
    ) -> Completion {
        self.calls.push(messages.to_vec());
        let stream = opts.stream;
        let text = self.next();
        if !stream {
            return self.done(text);
        }
        if !on(Chunk::Start) {
            return self.done(String::new());
        }
        if !text.is_empty() {
            let _ = on(Chunk::Text(text.clone()));
        }
        self.done(text)
    }

    fn last_usage(&self) -> Option<TokenUsage> {
        self.usage
    }
}

/// 目的：测试网关：忽略传入的通道，一律发脚本假模型（只用于测试装配）。
pub struct DemoGateway;

impl ChatGateway for DemoGateway {
    /// 脚本替身没有真实供应商可测：如实说"测不了"，不假装测过。
    fn probe_tools(
        &self,
        _channel: &Channel,
    ) -> Result<crate::capabilities::llm::api::ProbeOutcome, String> {
        Err("测试通道没有真实供应商，测不了工具调用支持".to_string())
    }

    fn member_channel(&self, _channel: &Channel, module_id: &str) -> BoxedChat {
        Box::new(FakeChat::new(vec![format!(
            "{{\"type\":\"say\",\"text\":\"（演示）{} 收到。\"}}",
            module_id
        )]))
    }

    fn core_channel(&self, _channel: &Channel) -> BoxedChat {
        // 核心操作走**工具调用**（见 docs/tools/tools-and-roles.md）：演示通道没有原生能力，
        // 所以发手写信封——它是这条通道唯一能用的形态。载荷形状与真实通道完全一致。
        let env = |tool: &str, args: &str| {
            format!(
                "{{\"type\":\"tool\",\"name\":\"{}\",\"args\":{}}}",
                tool, args
            )
        };
        Box::new(FakeChat::new(vec![
            "{\"type\":\"say\",\"text\":\"（演示）核心通道。\"}".to_string(),
            env(
                "checklist",
                "{\"items\":[{\"item\":\"演示项\",\"status\":\"pass\",\"evidence\":\"演示\"}]}",
            ),
            env(
                "plan",
                "{\"plan\":\"（演示）方案\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做\",\"objective\":\"做\",\"assignee\":\"a\",\"deps\":[]}]}",
            ),
        ]))
    }
}
