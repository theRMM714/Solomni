//! 真实通道：OpenAI 兼容 /chat/completions（ureq）。
//! 网关只做机制：给啥供应商建啥通道；provider = None 时回落演示通道并如实告知。
//! 密钥只在出站调用里使用，永不落提示词/转录/日志；错误信息经脱敏（红线）。

use super::fake_chat::DemoGateway;
use crate::core::ports::{BoxedChat, Chat, ChatGateway, Msg, Raw};
use crate::core::providers::Provider;

/// 真实会话通道：拥有供应商副本（含密钥；密钥不出适配层）。
pub struct HttpChat {
    pub provider: Provider,
    pub model: String,
}

impl Chat for HttpChat {
    fn complete(&mut self, messages: &[Msg]) -> Raw {
        let body = serde_json::json!({
            "model": self.model,
            "messages": messages
                .iter()
                .map(|m| serde_json::json!({ "role": m.role, "content": m.content }))
                .collect::<Vec<_>>(),
        });
        let url = format!("{}/chat/completions", self.provider.base_url.trim_end_matches('/'));
        match ureq_do(&url, &self.provider.api_key, &body.to_string()) {
            Ok(t) => t,
            Err(e) => {
                // 不猜测：失败原文照出，信封解析层会按 say 降级收录（转录即内容）。
                format!("模型调用失败：{}", e)
            }
        }
    }
}

/// 真实 HTTP 出站；错误信息只含状态与响应摘要，永不携带密钥。
fn ureq_do(url: &str, key: &str, body: &str) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(120))
        .build();
    let resp = agent
        .post(url)
        .set("Authorization", &format!("Bearer {}", key))
        .set("Content-Type", "application/json")
        .send_string(body)
        .map_err(|e| {
            // 红线：ureq 部分错误会回显请求头，密钥必须先脱敏再出适配层。
            let redact = |s: String| s.replace(key, "***");
            match e {
                ureq::Error::Status(code, resp) => {
                    let snippet = resp.into_string().unwrap_or_default();
                    let snippet: String = snippet.chars().take(200).collect();
                    redact(format!("供应商返回 {}：{}", code, snippet))
                }
                other => redact(format!("网络错误：{}", other)),
            }
        })?;
    let text = resp.into_string().map_err(|e| e.to_string())?;
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("响应不是 JSON：{}", e))?;
    v.get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "响应缺少 choices[0].message.content".to_string())
}

fn real_or_demo(provider: Option<&Provider>) -> (BoxedChat, bool) {
    match provider {
        Some(p) => {
            let model = p.models.first().cloned().unwrap_or_else(|| "default".to_string());
            (Box::new(HttpChat { provider: p.clone(), model }), false)
        }
        None => {
            let (chat, demo) = DemoGateway.core_channel(None);
            (chat, demo)
        }
    }
}

/// 真实网关：机制only。回落演示是如实告知的兜底，不是选择策略。
pub struct HttpGateway;

impl ChatGateway for HttpGateway {
    fn member_channel(&self, provider: Option<&Provider>, module_id: &str) -> (BoxedChat, Option<String>) {
        match provider {
            Some(p) => {
                let model = p.models.first().cloned().unwrap_or_else(|| "default".to_string());
                (Box::new(HttpChat { provider: p.clone(), model }), None)
            }
            // 回落告知（含模块 id）复用演示网关的话术。
            None => DemoGateway.member_channel(None, module_id),
        }
    }

    fn core_channel(&self, provider: Option<&Provider>) -> (BoxedChat, bool) {
        real_or_demo(provider)
    }
}