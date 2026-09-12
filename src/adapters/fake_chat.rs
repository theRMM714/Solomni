//! 演示/测试通道：脚本假模型（无网络、无密钥）。
//! 双用途：核心回落演示（无供应商时，如实告知）+ 单元测试脚本回放。
//! 只实现 core 的 Chat/ChatGateway 端口，不做装配决策。

use crate::core::ports::{BoxedChat, Chat, ChatGateway, Msg, Raw};
use crate::core::providers::Provider;

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
    fn complete(&mut self, messages: &[Msg]) -> Raw {
        self.calls.push(messages.to_vec());
        self.next()
    }
}

/// 演示网关：一律发演示通道（无供应商回落 / 测试装配两用）。
pub struct DemoGateway;

impl ChatGateway for DemoGateway {
    fn member_channel(&self, _provider: Option<&Provider>, module_id: &str) -> (BoxedChat, Option<String>) {
        (
            Box::new(FakeChat::new(vec![format!(
                "{{\"type\":\"say\",\"text\":\"（演示）{} 收到。\"}}",
                module_id
            )])),
            Some(format!("[{}] 未配置供应商，使用内置假模型演示", module_id)),
        )
    }

    fn core_channel(&self, _provider: Option<&Provider>) -> (BoxedChat, bool) {
        (
            Box::new(FakeChat::new(vec![
                "{\"type\":\"say\",\"text\":\"（演示）核心通道。\"}".to_string(),
                "[{\"item\":\"演示项\",\"status\":\"pass\",\"evidence\":\"演示\"}]".to_string(),
            ])),
            true,
        )
    }
}