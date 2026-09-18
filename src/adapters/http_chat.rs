//! 真实通道：OpenAI 兼容 /chat/completions（ureq）。
//! 网关只做机制：给啥通道（供应商 + 模型）建啥会话；channel = None 时回落演示并如实告知。
//! 端点补全/回落规则见 endpoint 模块：无版本段先直连，404/405 再试 /v1。
//! 密钥只在出站调用里使用，永不落提示词/转录/日志；错误信息经脱敏（红线）。

use super::endpoint::{chat_candidates, memo_get, memo_set, resolve_candidates, retryable_status, Attempt, Memo};
use super::fake_chat::DemoGateway;
use crate::core::ports::{
    BoxedChat, Chat, ChatGateway, Chunk, CompleteOpts, Completion, Msg, ProbeOutcome, ToolCall, ToolDecl,
};
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

/// 组一次 /chat/completions 的请求体。**真实会话与探针共用同一份形状**——
/// 探针发出去的必须是线上真会发的东西，否则它测出来的结论代表不了线上行为。
fn request_body(
    model: &str,
    stream: bool,
    messages: serde_json::Value,
    tools: Option<&[ToolDecl]>,
) -> serde_json::Value {
    let mut body = serde_json::json!({ "model": model, "stream": stream, "messages": messages });
    // 原生工具调用：把工具声明带给供应商（"参数走结构化槽位"的全部秘密就在这一段）。
    // 不声明 tools = 手写信封模式：模型照旧在正文里写信封，核心自己解析。
    if let Some(tools) = tools {
        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(tools.iter().map(decl_json).collect());
            body["tool_choice"] = serde_json::json!("auto");
        }
    }
    body
}

/// 发一段**合成好的 messages**（探针专用）：回放形状探测要发的不是普通消息，
/// 而是含 tool_calls 与 role=tool 的历史。端点候选、脱敏与重试规则与真实请求完全一致。
pub(crate) fn attempt_raw(
    url: &str,
    key: &str,
    model: &str,
    messages: serde_json::Value,
    tools: Option<&[ToolDecl]>,
) -> Attempt<Completion> {
    let body = request_body(model, false, messages, tools).to_string();
    attempt(url, key, &body)
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
    fn complete(&mut self, messages: &[Msg], opts: CompleteOpts<'_>, on: &mut dyn FnMut(Chunk) -> bool) -> Completion {
        let stream = opts.stream;
        let wire: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| serde_json::json!({ "role": m.role, "content": m.content }))
            .collect();
        let body = request_body(
            &self.channel.model,
            stream,
            serde_json::Value::Array(wire),
            opts.tools,
        )
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
                Completion::text(format!("模型调用失败：{}", e))
            }
        }
    }
}

/// 一次流式 POST：读 SSE，逐片回调正文与思维链；返回完整正文。
/// 已经吐出过内容后不再换候选（避免重复输出）。
/// 兼容两类供应商：发「增量」的、以及发「累积快照」的（此处统一归一成增量）。
/// 中止与容积双保险：on 返回 false、或正文/思维链超过上限，立即停止读取。
fn stream_once(url: &str, key: &str, body: &str, on: &mut dyn FnMut(Chunk) -> bool) -> Attempt<Completion> {
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
        return Attempt::Ok(Completion::text(""));
    }
    let mut content = String::new();
    let mut reasoning_acc = String::new();
    // 供应商的结束原因（通常只在最后一个分片里给）：如实带回，核心据此分辨"写完停"还是"被截断"
    let mut finish = String::new();
    // 原生工具调用：按 index 分片来（id/name 一般在第一片，arguments 逐片拼接）——最常见的坑就在这里
    let mut calls: Vec<ToolCall> = Vec::new();
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
        let Some(choice) = v.get("choices").and_then(|c| c.get(0)) else { continue };
        if let Some(f) = choice.get("finish_reason").and_then(|f| f.as_str()) {
            if !f.is_empty() {
                finish = f.to_string();
            }
        }
        let Some(delta) = choice.get("delta") else { continue };
        // 原生工具调用的分片：按 index 归位，name/id 出现即记，arguments 追加
        if let Some(arr) = delta.get("tool_calls").and_then(|t| t.as_array()) {
            for item in arr {
                let idx = item.get("index").and_then(|i| i.as_u64()).unwrap_or(calls.len() as u64) as usize;
                while calls.len() <= idx {
                    calls.push(ToolCall { id: String::new(), name: String::new(), args_json: String::new() });
                }
                let slot = &mut calls[idx];
                if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                    if !id.is_empty() {
                        slot.id = id.to_string();
                    }
                }
                if let Some(f) = item.get("function") {
                    if let Some(n) = f.get("name").and_then(|n| n.as_str()) {
                        if !n.is_empty() {
                            slot.name = n.to_string();
                        }
                    }
                    if let Some(a) = f.get("arguments").and_then(|a| a.as_str()) {
                        slot.args_json.push_str(a);
                    }
                }
            }
        }
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
        return Attempt::Ok(Completion { raw: content, finish, calls });
    }
    // 纯工具调用轮的正文是空的：这不算"没内容"，不能因此换候选重试。
    if content.is_empty() && reasoning_acc.is_empty() && calls.is_empty() {
        return Attempt::Retry("流式响应没有正文内容".to_string());
    }
    Attempt::Ok(Completion { raw: content, finish, calls })
}

/// 工具声明 → OpenAI 兼容的 function 形状（唯一一处映射，探测与正式请求共用）。
fn decl_json(t: &ToolDecl) -> serde_json::Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": t.name,
            "description": t.description,
            "parameters": t.parameters,
        },
    })
}

/// 一条最小请求：单条用户消息 + 可选工具声明（探测用它；正式对话走 complete）。
/// 复用同一套端点解析、脱敏与响应解析，不另写一份。
pub(crate) fn attempt_with_tools(
    url: &str,
    key: &str,
    model: &str,
    user: &str,
    tools: Option<&[ToolDecl]>,
) -> Attempt<Completion> {
    let mut body = serde_json::json!({
        "model": model,
        "stream": false,
        "messages": [{ "role": "user", "content": user }],
    });
    if let Some(tools) = tools {
        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(tools.iter().map(decl_json).collect());
            body["tool_choice"] = serde_json::json!("auto");
        }
    }
    attempt(url, key, &body.to_string())
}

/// 单次 POST：请求与解析都在此；失败按「可换候选 / 立即报」归类。
fn attempt(url: &str, key: &str, body: &str) -> Attempt<Completion> {
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

/// 解析补全响应：取 choices[0].message.content、finish_reason 与原生 tool_calls（后两者可能没有）。
fn parse_content(text: &str) -> Result<Completion, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("响应不是 JSON：{}", e))?;
    let choice = v.get("choices").and_then(|c| c.get(0));
    // 有些供应商在纯工具调用时 content 是 null（不是缺字段）：按空串处理，不算错。
    let raw = choice
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or_default()
        .to_string();
    let finish = choice
        .and_then(|c| c.get("finish_reason"))
        .and_then(|f| f.as_str())
        .unwrap_or_default()
        .to_string();
    let calls = choice
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("tool_calls"))
        .and_then(|t| t.as_array())
        .map(|arr| arr.iter().filter_map(native_call).collect::<Vec<_>>())
        .unwrap_or_default();
    if raw.is_empty() && calls.is_empty() {
        return Err("响应缺少 choices[0].message.content（也没有 tool_calls）".to_string());
    }
    Ok(Completion { raw, finish, calls })
}

/// 从一段 OpenAI 形状的 tool_calls 元素里取（id, name, arguments）。
/// arguments 是**一段 JSON 文本**（供应商原样给），能不能解析留给上层如实报。
fn native_call(v: &serde_json::Value) -> Option<ToolCall> {
    let f = v.get("function")?;
    let name = f.get("name").and_then(|n| n.as_str())?.to_string();
    let id = v.get("id").and_then(|i| i.as_str()).unwrap_or_default().to_string();
    let args_json = f.get("arguments").and_then(|a| a.as_str()).unwrap_or("{}").to_string();
    Some(ToolCall { id, name, args_json })
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
    fn probe_tools(&self, channel: &Channel) -> Result<ProbeOutcome, String> {
        crate::adapters::http_probe::probe(channel, &self.log)
    }

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
