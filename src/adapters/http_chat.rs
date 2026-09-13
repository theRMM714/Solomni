//! 真实通道：OpenAI 兼容 /chat/completions（ureq）。
//! 网关只做机制：给啥通道（供应商 + 模型）建啥会话；channel = None 时回落演示并如实告知。
//! 端点补全/回落规则见 endpoint 模块：无版本段先直连，404/405 再试 /v1。
//! 密钥只在出站调用里使用，永不落提示词/转录/日志；错误信息经脱敏（红线）。

use super::endpoint::{chat_candidates, memo_get, memo_set, resolve_candidates, retryable_status, Attempt, Memo};
use super::fake_chat::DemoGateway;
use crate::core::ports::{BoxedChat, Chat, ChatGateway, Chunk, Msg, Raw};
use crate::core::providers::Channel;

/// 真实会话通道：拥有通道副本（含密钥；密钥不出适配层）。
/// resolved = 本会话首次命中的有效端点，后续轮次直接复用，不再重复探测。
pub struct HttpChat {
    pub channel: Channel,
    pub provider_id: String,
    pub log: std::sync::Arc<dyn crate::core::ports::Log + Send + Sync>,
    resolved: Option<String>,
    /// 进程内共享的端点记忆：一旦通了就固定，后续会话不再探测。
    memo: Memo,
    memo_key: String,
}

impl HttpChat {
    /// 组合根/网关内部构造：通道 + 日志 + 共享端点记忆。
    fn new(channel: Channel, log: std::sync::Arc<dyn crate::core::ports::Log + Send + Sync>, memo: Memo) -> HttpChat {
        let memo_key = format!("{}|chat", channel.provider.base_url);
        let resolved = memo_get(&memo, &memo_key);
        HttpChat { provider_id: channel.provider.base_url.clone(), channel, log, resolved, memo, memo_key }
    }
}

impl Chat for HttpChat {
    fn complete(&mut self, messages: &[Msg], stream: bool, on: &mut dyn FnMut(Chunk) -> bool) -> Raw {
        let body = serde_json::json!({
            "model": self.channel.model,
            "stream": stream,
            "messages": messages
                .iter()
                .map(|m| serde_json::json!({ "role": m.role, "content": m.content }))
                .collect::<Vec<_>>(),
        })
        .to_string();
        let candidates: Vec<String> = match &self.resolved {
            Some(url) => vec![url.clone()],
            None => chat_candidates(&self.channel.provider.base_url),
        };
        let key = self.channel.provider.api_key.clone();
        let outcome = if stream {
            resolve_candidates(
                &candidates,
                |url| stream_once(url, &key, &body, on),
                |url, err, next| {
                    self.log.warn("http_chat::stream", &format!("端点 {} 不可用（{}），改试 {}", url, err, next));
                },
            )
        } else {
            resolve_candidates(
                &candidates,
                |url| attempt(url, &key, &body),
                |url, err, next| {
                    self.log.warn("http_chat::complete", &format!("端点 {} 不可用（{}），改试 {}", url, err, next));
                },
            )
        };
        match outcome {
            Ok((url, text)) => {
                // 通了就定死：进程内共享，后续会话直接用它，不再探测候选。
                memo_set(&self.memo, &self.memo_key, &url);
                self.resolved = Some(url);
                text
            }
            Err(e) => {
                self.log.error("http_chat::complete", &format!("通道 {} 调用失败：{}", self.provider_id, e));
                format!("模型调用失败：{}", e)
            }
        }
    }
}

/// 一次流式 POST：读 SSE，逐片回调正文与思维链；返回完整正文。
/// 已经吐出过内容后不再换候选（避免重复输出）。
/// 兼容两类供应商：发「增量」的、以及发「累积快照」的（此处统一归一成增量）。
/// 中止与容积双保险：on 返回 false、或正文/思维链超过上限，立即停止读取。
fn stream_once(url: &str, key: &str, body: &str, on: &mut dyn FnMut(Chunk) -> bool) -> Attempt<String> {
    use std::io::BufRead;
    let agent = super::http_agent::agent(10, 300);
    let resp = match agent
        .post(url)
        .set("Authorization", &format!("Bearer {}", key))
        .set("Content-Type", "application/json")
        .send_string(body)
    {
        Ok(r) => r,
        Err(ureq::Error::Status(code, resp)) => {
            let snippet = resp.into_string().unwrap_or_default();
            let snippet: String = snippet.chars().take(200).collect();
            let msg = redact(format!("供应商返回 {}：{}", code, snippet), key);
            return if retryable_status(code) { Attempt::Retry(msg) } else { Attempt::Fatal(msg) };
        }
        Err(other) => return Attempt::Retry(redact(format!("网络错误：{}", other), key)),
    };
    const MAX_STREAM_CHARS: usize = 200_000;
    // 新一轮开始：让调用方清空本轮流式占位（工具多轮各成一段）
    if !on(Chunk::Start) {
        return Attempt::Ok(String::new());
    }
    let mut content = String::new();
    let mut reasoning_acc = String::new();
    let mut got_any = false;
    let mut cancelled = false;
    for line in std::io::BufReader::new(resp.into_reader()).lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                let msg = redact(format!("读取流失败：{}", e), key);
                return if got_any { Attempt::Fatal(msg) } else { Attempt::Retry(msg) };
            }
        };
        let Some(data) = line.strip_prefix("data:") else { continue };
        let data = data.trim();
        if data == "[DONE]" {
            break;
        }
        if data.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else { continue };
        let Some(delta) = v.get("choices").and_then(|c| c.get(0)).and_then(|c| c.get("delta")) else { continue };
        // 正文：累积快照 → 只取新增部分；正常增量 → 原样
        if let Some(t) = delta.get("content").and_then(|c| c.as_str()) {
            if !t.is_empty() {
                got_any = true;
                // 只有「比已积累的更长且以它为前缀」才算累积快照；等长/重复的按普通增量追加，
                // 保证每个分片都会回调 on（否则中止检查会被跳过，流会卡死）。
                let piece = if t.len() > content.len() && t.starts_with(content.as_str()) {
                    t[content.len()..].to_string()
                } else {
                    t.to_string()
                };
                content = if piece.is_empty() { content } else { format!("{}{}", content, piece) };
                if !piece.is_empty() && !on(Chunk::Text(piece)) {
                    cancelled = true;
                    break;
                }
            }
        }
        // 思维链：同样归一化
        let reasoning = delta
            .get("reasoning_content")
            .and_then(|c| c.as_str())
            .or_else(|| delta.get("reasoning").and_then(|c| c.as_str()));
        if let Some(r) = reasoning {
            if !r.is_empty() {
                got_any = true;
                let piece = if r.len() > reasoning_acc.len() && r.starts_with(reasoning_acc.as_str()) {
                    r[reasoning_acc.len()..].to_string()
                } else {
                    r.to_string()
                };
                reasoning_acc = if piece.is_empty() { reasoning_acc } else { format!("{}{}", reasoning_acc, piece) };
                if !piece.is_empty() && !on(Chunk::Reasoning(piece)) {
                    cancelled = true;
                    break;
                }
            }
        }
        // 容积上限：防供应商跑飞把内存与界面拖死
        if content.len() + reasoning_acc.len() > MAX_STREAM_CHARS {
            break;
        }
    }
    // 用户主动中止：原样收尾，绝不换候选重开（否则「停止」会把流重新拉起来）
    if cancelled {
        return Attempt::Ok(content);
    }
    if content.is_empty() && reasoning_acc.is_empty() {
        return Attempt::Retry("流式响应没有正文内容".to_string());
    }
    Attempt::Ok(content)
}

/// 单次 POST：请求与解析都在此；失败按「可换候选 / 立即报」归类。
fn attempt(url: &str, key: &str, body: &str) -> Attempt<String> {
    let agent = super::http_agent::agent(10, 120);
    let resp = match agent
        .post(url)
        .set("Authorization", &format!("Bearer {}", key))
        .set("Content-Type", "application/json")
        .send_string(body)
    {
        Ok(r) => r,
        // 红线：ureq 部分错误会回显请求头，密钥必须先脱敏再出适配层。
        Err(ureq::Error::Status(code, resp)) => {
            let snippet = resp.into_string().unwrap_or_default();
            let snippet: String = snippet.chars().take(200).collect();
            let msg = redact(format!("供应商返回 {}：{}", code, snippet), key);
            return if retryable_status(code) { Attempt::Retry(msg) } else { Attempt::Fatal(msg) };
        }
        Err(other) => return Attempt::Retry(redact(format!("网络错误：{}", other), key)),
    };
    let text = match resp.into_string() {
        Ok(t) => t,
        Err(e) => return Attempt::Retry(redact(e.to_string(), key)),
    };
    match parse_content(&text) {
        Ok(c) => Attempt::Ok(c),
        // 2xx 但不是补全形状（SPA 回落的 HTML 首页等）：这里不是该 API，换下一个候选。
        Err(e) => Attempt::Retry(e),
    }
}

/// 解析补全响应：取 choices[0].message.content。
fn parse_content(text: &str) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("响应不是 JSON：{}", e))?;
    v.get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "响应缺少 choices[0].message.content".to_string())
}

/// 出站错误里的密钥一律替换掉再出适配层。
fn redact(s: String, key: &str) -> String {
    if key.is_empty() {
        s
    } else {
        s.replace(key, "***")
    }
}

fn real_or_demo(channel: Option<&Channel>, log: &std::sync::Arc<dyn crate::core::ports::Log + Send + Sync>, memo: &Memo) -> (BoxedChat, bool) {
    match channel {
        Some(c) => (Box::new(HttpChat::new(c.clone(), std::sync::Arc::clone(log), std::sync::Arc::clone(memo))), false),
        None => {
            let (chat, demo) = DemoGateway.core_channel(None);
            (chat, demo)
        }
    }
}

/// 真实网关：机制only。回落演示是如实告知的兜底，不是选择策略。
pub struct HttpGateway {
    log: std::sync::Arc<dyn crate::core::ports::Log + Send + Sync>,
    memo: Memo,
}

impl HttpGateway {
    /// 组合根注入日志端口（异常路径落盘）与共享端点记忆。
    pub fn with_log(log: std::sync::Arc<dyn crate::core::ports::Log + Send + Sync>, memo: Memo) -> HttpGateway {
        HttpGateway { log, memo }
    }
}

impl ChatGateway for HttpGateway {
    fn member_channel(&self, channel: Option<&Channel>, module_id: &str) -> (BoxedChat, Option<String>) {
        match channel {
            Some(c) => {
                self.log.info(
                    "gateway::member_channel",
                    &format!("模块 {} → 供应商 {}（模型 {}）", module_id, c.provider.base_url, c.model),
                );
                (Box::new(HttpChat::new(c.clone(), std::sync::Arc::clone(&self.log), std::sync::Arc::clone(&self.memo))), None)
            }
            // 回落告知（含模块 id）复用演示网关的话术。
            None => {
                self.log.warn("gateway::member_channel", &format!("模块 {} 无可用模型通道，回落演示通道", module_id));
                DemoGateway.member_channel(None, module_id)
            }
        }
    }

    fn core_channel(&self, channel: Option<&Channel>) -> (BoxedChat, bool) {
        let (chat, demo) = real_or_demo(channel, &self.log, &self.memo);
        if demo {
            self.log.warn("gateway::core_channel", "核心通道未配置模型，使用演示通道");
        } else {
            self.log.info("gateway::core_channel", "核心通道建立（真实供应商）");
        }
        (chat, demo)
    }
}
