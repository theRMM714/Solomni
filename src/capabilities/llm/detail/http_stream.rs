//! 目的：可取消的 OpenAI 兼容流式 HTTP 适配器。
//! 管：一次模型调用的出站流、SSE 解析、分层超时与取消关闭。
//! 不管：端点候选与重试归类（endpoint）、通道构造（http_chat）、会话状态与行落盘（会话能力）。
//! 联动：http_chat 调它；预算来自 llm::api::CompleteOpts；TLS / 代理由 reqwest 的平台配置。

use super::endpoint::{retryable_status, Attempt};
use super::http_agent::redact;
use crate::capabilities::llm::api::{Chunk, CompleteOpts, Completion, ToolCall};
use futures_util::StreamExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 流已经收到过数据后，两个数据块之间最多等多久；收到任意字节（含 SSE keepalive）即重置。
const STREAM_IDLE_TIMEOUT_SECS: u64 = 120;
/// 取消位轮询间隔：停止请求不依赖供应商继续发数据。
const CANCEL_POLL: Duration = Duration::from_millis(50);
const MAX_STREAM_CHARS: usize = 200_000;

/// 目的：一次 LLM 调用用的 current-thread runtime；只用时间驱动，不把核心改成 async。
pub(crate) fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("建立流式运行时失败：{}", e))
}

fn client() -> Result<reqwest::Client, String> {
    let builder = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        // 与旧 ureq 通道一致：只走 HTTP/1.1，避免供应商在 h2 下缓冲 SSE。
        .http1_only();
    #[cfg(not(windows))]
    let builder = builder.use_preconfigured_tls(tls_config()?);
    builder
        .build()
        .map_err(|e| format!("建立 HTTP 客户端失败：{}", e))
}

/// 非 Windows：rustls + ring 提供者 + 系统证书验证器（与 ureq 的证书口径一致）。
#[cfg(not(windows))]
fn tls_config() -> Result<rustls::ClientConfig, String> {
    use rustls_platform_verifier::ConfigVerifierExt;
    let _ = rustls::crypto::ring::default_provider().install_default();
    rustls::ClientConfig::with_platform_verifier().map_err(|e| format!("TLS 配置失败：{}", e))
}

fn budget(secs: u64) -> Duration {
    Duration::from_secs(secs.max(1))
}

fn idle_budget(secs: u64) -> Duration {
    budget(secs.min(STREAM_IDLE_TIMEOUT_SECS))
}

fn is_cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(|c| c.load(Ordering::Relaxed))
}

async fn wait_cancel(cancel: Option<&AtomicBool>) {
    match cancel {
        Some(c) => {
            while !c.load(Ordering::Relaxed) {
                tokio::time::sleep(CANCEL_POLL).await;
            }
        }
        None => std::future::pending::<()>().await,
    }
}

fn cancelled() -> Attempt<Completion> {
    Attempt::Ok(Completion::cancelled(
        String::new(),
        String::new(),
        String::new(),
        Vec::new(),
    ))
}

/// 目的：一次流式 POST：读 SSE、逐片回调正文/思维链并返回完整正文。
/// 约束：兼容增量与累积快照；`on` 返回 false、取消位置位或超过容积上限时立即停止。
pub(crate) async fn stream_once(
    url: &str,
    key: &str,
    body: &str,
    opts: CompleteOpts<'_>,
    on: &mut dyn FnMut(Chunk) -> bool,
) -> Attempt<Completion> {
    if is_cancelled(opts.cancel.as_deref()) {
        return cancelled();
    }
    let client = match client() {
        Ok(c) => c,
        Err(e) => return Attempt::Fatal(redact(e, key)),
    };
    let first = budget(opts.timeout_secs);
    let idle = idle_budget(opts.timeout_secs);
    let request = client
        .post(url)
        .header("Authorization", &format!("Bearer {}", key))
        .header("Content-Type", "application/json")
        .body(body.to_string());
    let sent = tokio::select! {
        biased;
        _ = wait_cancel(opts.cancel.as_deref()) => return cancelled(),
        r = tokio::time::timeout(first, request.send()) => r,
    };
    let resp = match sent {
        Err(_) => {
            return Attempt::Retry(format!("等待响应超时（{} 秒）", first.as_secs()));
        }
        Ok(Err(e)) => {
            return Attempt::Retry(redact(format!("网络错误：{}", e), key));
        }
        Ok(Ok(r)) => r,
    };
    let status = resp.status();
    if !status.is_success() {
        let code = status.as_u16();
        let deadline = tokio::time::Instant::now() + first;
        let snippet = match read_text(resp, opts.cancel.as_deref(), deadline, key).await {
            Ok(t) => t.chars().take(200).collect::<String>(),
            Err(_) => String::new(),
        };
        if is_cancelled(opts.cancel.as_deref()) {
            return cancelled();
        }
        let msg = redact(format!("供应商返回 {}：{}", code, snippet), key);
        return if retryable_status(code) {
            Attempt::Retry(msg)
        } else {
            Attempt::Fatal(msg)
        };
    }
    if !on(Chunk::Start) {
        return cancelled();
    }
    let mut stream = resp.bytes_stream();
    let mut state = StreamState::default();
    let mut buf: Vec<u8> = Vec::new();
    let mut first_chunk = true;
    let mut done = false;
    while !done {
        let wait = if first_chunk { first } else { idle };
        let next = tokio::select! {
            biased;
            _ = wait_cancel(opts.cancel.as_deref()) => return state.cancelled(),
            r = tokio::time::timeout(wait, stream.next()) => r,
        };
        match next {
            // 首块超时 = 没拿到任何回复；空闲超时 = 流走到一半不再出数据。
            Err(_) if first_chunk => {
                return Attempt::Retry(format!("等待首个数据块超时（{} 秒）", first.as_secs()));
            }
            Err(_) => {
                let msg = format!("流空闲超时（{} 秒）", idle.as_secs());
                return if state.got_any {
                    Attempt::Fatal(msg)
                } else {
                    Attempt::Retry(msg)
                };
            }
            Ok(None) => break,
            Ok(Some(Err(e))) => {
                let msg = redact(format!("读取流失败：{}", e), key);
                return if state.got_any {
                    Attempt::Fatal(msg)
                } else {
                    Attempt::Retry(msg)
                };
            }
            Ok(Some(Ok(bytes))) => {
                first_chunk = false;
                buf.extend_from_slice(&bytes);
                match feed(&mut buf, &mut state, on, opts.cancel.as_deref()) {
                    Feed::Continue => {}
                    Feed::Done => done = true,
                    Feed::Cancelled => return state.cancelled(),
                }
                if state.content.len() + state.reasoning.len() > MAX_STREAM_CHARS {
                    done = true;
                }
            }
        }
    }
    // 最后一行可能没有换行符（有些供应商不发结尾空行）。
    if !buf.is_empty() {
        buf.push(b'\n');
        match feed(&mut buf, &mut state, on, opts.cancel.as_deref()) {
            Feed::Continue | Feed::Done => {}
            Feed::Cancelled => return state.cancelled(),
        }
    }
    if state.content.is_empty() && state.reasoning.is_empty() && state.calls.is_empty() {
        return Attempt::Retry("流式响应没有正文内容".to_string());
    }
    Attempt::Ok(Completion {
        raw: state.content,
        reasoning: state.reasoning,
        finish: state.finish,
        calls: state.calls,
        error: None,
        cancelled: false,
    })
}

enum Feed {
    Continue,
    Done,
    Cancelled,
}

#[derive(Default)]
struct StreamState {
    content: String,
    reasoning: String,
    finish: String,
    calls: Vec<ToolCall>,
    got_any: bool,
}

impl StreamState {
    fn cancelled(self) -> Attempt<Completion> {
        Attempt::Ok(Completion::cancelled(
            self.content,
            self.reasoning,
            self.finish,
            self.calls,
        ))
    }
}

fn feed(
    buf: &mut Vec<u8>,
    state: &mut StreamState,
    on: &mut dyn FnMut(Chunk) -> bool,
    cancel: Option<&AtomicBool>,
) -> Feed {
    while !is_cancelled(cancel) {
        let Some(pos) = buf.iter().position(|b| *b == b'\n') else {
            break;
        };
        let raw: Vec<u8> = buf.drain(..=pos).collect();
        let line = String::from_utf8_lossy(&raw[..raw.len() - 1])
            .trim_end_matches('\r')
            .to_string();
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data == "[DONE]" {
            return Feed::Done;
        }
        if data.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };
        let Some(choice) = v.get("choices").and_then(|c| c.get(0)) else {
            continue;
        };
        if let Some(f) = choice.get("finish_reason").and_then(|f| f.as_str()) {
            if !f.is_empty() {
                state.finish = f.to_string();
            }
        }
        let Some(delta) = choice.get("delta") else {
            continue;
        };
        if let Some(arr) = delta.get("tool_calls").and_then(|t| t.as_array()) {
            if !arr.is_empty() {
                state.got_any = true;
            }
            for item in arr {
                let idx = item
                    .get("index")
                    .and_then(|i| i.as_u64())
                    .unwrap_or(state.calls.len() as u64) as usize;
                while state.calls.len() <= idx {
                    state.calls.push(ToolCall {
                        id: String::new(),
                        name: String::new(),
                        args_json: String::new(),
                    });
                }
                let slot = &mut state.calls[idx];
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
        if let Some(t) = delta.get("content").and_then(|c| c.as_str()) {
            if !t.is_empty() {
                state.got_any = true;
                let piece =
                    if t.len() > state.content.len() && t.starts_with(state.content.as_str()) {
                        t[state.content.len()..].to_string()
                    } else {
                        t.to_string()
                    };
                if !piece.is_empty() {
                    state.content.push_str(&piece);
                    if !on(Chunk::Text(piece)) {
                        return Feed::Cancelled;
                    }
                }
            }
        }
        let reasoning = delta
            .get("reasoning_content")
            .and_then(|c| c.as_str())
            .or_else(|| delta.get("reasoning").and_then(|c| c.as_str()));
        if let Some(r) = reasoning {
            if !r.is_empty() {
                state.got_any = true;
                let piece =
                    if r.len() > state.reasoning.len() && r.starts_with(state.reasoning.as_str()) {
                        r[state.reasoning.len()..].to_string()
                    } else {
                        r.to_string()
                    };
                if !piece.is_empty() {
                    state.reasoning.push_str(&piece);
                    if !on(Chunk::Reasoning(piece)) {
                        return Feed::Cancelled;
                    }
                }
            }
        }
    }
    if is_cancelled(cancel) {
        Feed::Cancelled
    } else {
        Feed::Continue
    }
}

/// 目的：非流式 POST；没有中间分片可重置，整次调用共用一个截止点。
pub(crate) async fn attempt_async(
    url: &str,
    key: &str,
    body: &str,
    timeout_secs: u64,
    cancel: Option<&AtomicBool>,
) -> Attempt<Completion> {
    if is_cancelled(cancel) {
        return cancelled();
    }
    let client = match client() {
        Ok(c) => c,
        Err(e) => return Attempt::Fatal(redact(e, key)),
    };
    let budget = budget(timeout_secs);
    // 非流式没有中间数据可重置：整次调用（等响应头 + 读响应体）共用一个截止点。
    let deadline = tokio::time::Instant::now() + budget;
    let request = client
        .post(url)
        .header("Authorization", &format!("Bearer {}", key))
        .header("Content-Type", "application/json")
        .body(body.to_string());
    let sent = tokio::select! {
        biased;
        _ = wait_cancel(cancel) => return cancelled(),
        r = tokio::time::timeout_at(deadline, request.send()) => r,
    };
    let resp = match sent {
        Err(_) => return Attempt::Retry(format!("等待响应超时（{} 秒）", budget.as_secs())),
        Ok(Err(e)) => return Attempt::Retry(redact(format!("网络错误：{}", e), key)),
        Ok(Ok(r)) => r,
    };
    let status = resp.status();
    if !status.is_success() {
        let code = status.as_u16();
        let snippet = match read_text(resp, cancel, deadline, key).await {
            Ok(t) => t.chars().take(200).collect::<String>(),
            Err(_) => String::new(),
        };
        if is_cancelled(cancel) {
            return cancelled();
        }
        let msg = redact(format!("供应商返回 {}：{}", code, snippet), key);
        return if retryable_status(code) {
            Attempt::Retry(msg)
        } else {
            Attempt::Fatal(msg)
        };
    }
    let text = match read_text(resp, cancel, deadline, key).await {
        Ok(t) => t,
        Err(e) => {
            return if is_cancelled(cancel) {
                cancelled()
            } else {
                Attempt::Retry(e)
            };
        }
    };
    match parse_content(&text) {
        Ok(c) => Attempt::Ok(c),
        // 2xx 但不是补全形状（SPA 回落的 HTML 首页等）：这里不是该 API，换下一个候选。
        Err(e) => Attempt::Retry(e),
    }
}

async fn read_text(
    resp: reqwest::Response,
    cancel: Option<&AtomicBool>,
    deadline: tokio::time::Instant,
    key: &str,
) -> Result<String, String> {
    let read = tokio::select! {
        biased;
        _ = wait_cancel(cancel) => return Err("已停止".to_string()),
        r = tokio::time::timeout_at(deadline, resp.text()) => r,
    };
    match read {
        Err(_) => Err("读取响应体超时".to_string()),
        Ok(Err(e)) => Err(redact(format!("读取响应体失败：{}", e), key)),
        Ok(Ok(t)) => Ok(t),
    }
}

/// 解析补全响应：取 choices[0].message.content、finish_reason 与原生 tool_calls（后两者可能没有）。
fn parse_content(text: &str) -> Result<Completion, String> {
    let v: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("响应不是 JSON：{}", e))?;
    let choice = v.get("choices").and_then(|c| c.get(0));
    // 有些供应商在纯工具调用时 content 是 null（不是缺字段）：按空串处理，不算错。
    let raw = choice
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or_default()
        .to_string();
    let reasoning = choice
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("reasoning_content").or_else(|| m.get("reasoning")))
        .and_then(|r| r.as_str())
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
    if raw.is_empty() && reasoning.is_empty() && calls.is_empty() {
        return Err("响应缺少 choices[0].message.content（也没有 tool_calls）".to_string());
    }
    Ok(Completion {
        raw,
        reasoning,
        finish,
        calls,
        error: None,
        cancelled: false,
    })
}

/// 从一段 OpenAI 形状的 tool_calls 元素里取（id, name, arguments）。
/// arguments 是**一段 JSON 文本**（供应商原样给），能不能解析留给上层如实报。
fn native_call(v: &serde_json::Value) -> Option<ToolCall> {
    let f = v.get("function")?;
    let name = f.get("name").and_then(|n| n.as_str())?.to_string();
    let id = v
        .get("id")
        .and_then(|i| i.as_str())
        .unwrap_or_default()
        .to_string();
    let args_json = f
        .get("arguments")
        .and_then(|a| a.as_str())
        .unwrap_or("{}")
        .to_string();
    Some(ToolCall {
        id,
        name,
        args_json,
    })
}
