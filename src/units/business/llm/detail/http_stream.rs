//! 目的：可取消的流式 HTTP 适配器（SSE 分行、超时、取消关闭）。
//! 管：一次模型调用的出站流、SSE 分行、分层超时与取消关闭；把每个数据块交给协议实现解析。
//! 不管：端点候选与重试归类（endpoint）、通道构造（http_chat）、字段名与 JSON 形状（detail/protocol）、会话状态与行落盘（会话单元）。
//! 联动：http_chat 调它；预算来自 llm::api::CompleteOpts；TLS 装配与代理由 kernel 的 `io/net` 统一决定（经 `api::net` 注入）。

use super::endpoint::{Attempt, retryable_status};
use super::http_agent::redact;
use crate::units::business::llm::api::{Cancelled, Chunk, CompleteOpts, Completion};
use crate::units::business::llm::domain::stream::StreamAcc;
use crate::units::business::llm::ports::{EventStep, Protocol};
use futures_util::StreamExt;
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
    let builder = crate::kernel::api::net::configure_reqwest(builder)?;
    builder
        .build()
        .map_err(|e| format!("建立 HTTP 客户端失败：{}", e))
}

fn budget(secs: u64) -> Duration {
    Duration::from_secs(secs.max(1))
}

fn idle_budget(secs: u64) -> Duration {
    budget(secs.min(STREAM_IDLE_TIMEOUT_SECS))
}

fn is_cancelled(cancel: Option<&dyn Cancelled>) -> bool {
    cancel.is_some_and(|c| c.cancelled())
}

async fn wait_cancel(cancel: Option<&dyn Cancelled>) {
    match cancel {
        Some(c) => {
            while !c.cancelled() {
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
/// 约束：分块解析归协议实现；`on` 返回 false、取消位置位或超过容积上限时立即停止。
pub(crate) async fn stream_once(
    proto: &dyn Protocol,
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
    let mut acc = StreamAcc::default();
    let mut buf: Vec<u8> = Vec::new();
    let mut first_chunk = true;
    let mut done = false;
    while !done {
        let wait = if first_chunk { first } else { idle };
        let next = tokio::select! {
            biased;
            _ = wait_cancel(opts.cancel.as_deref()) => return Attempt::Ok(acc.cancelled_completion()),
            r = tokio::time::timeout(wait, stream.next()) => r,
        };
        match next {
            // 首块超时 = 没拿到任何回复；空闲超时 = 流走到一半不再出数据。
            Err(_) if first_chunk => {
                return Attempt::Retry(format!("等待首个数据块超时（{} 秒）", first.as_secs()));
            }
            Err(_) => {
                let msg = format!("流空闲超时（{} 秒）", idle.as_secs());
                return if acc.got_any {
                    Attempt::Fatal(msg)
                } else {
                    Attempt::Retry(msg)
                };
            }
            Ok(None) => break,
            Ok(Some(Err(e))) => {
                let msg = redact(format!("读取流失败：{}", e), key);
                return if acc.got_any {
                    Attempt::Fatal(msg)
                } else {
                    Attempt::Retry(msg)
                };
            }
            Ok(Some(Ok(bytes))) => {
                first_chunk = false;
                buf.extend_from_slice(&bytes);
                match feed(proto, &mut buf, &mut acc, on, opts.cancel.as_deref()) {
                    Feed::Continue => {}
                    Feed::Done => done = true,
                    Feed::Cancelled => return Attempt::Ok(acc.cancelled_completion()),
                }
                if acc.content.len() + acc.reasoning.len() > MAX_STREAM_CHARS {
                    done = true;
                }
            }
        }
    }
    // 最后一行可能没有换行符（有些供应商不发结尾空行）。
    if !buf.is_empty() {
        buf.push(b'\n');
        match feed(proto, &mut buf, &mut acc, on, opts.cancel.as_deref()) {
            Feed::Continue | Feed::Done => {}
            Feed::Cancelled => return Attempt::Ok(acc.cancelled_completion()),
        }
    }
    if acc.is_blank() {
        return Attempt::Retry("流式响应没有正文内容".to_string());
    }
    Attempt::Ok(acc.done_completion())
}

enum Feed {
    Continue,
    Done,
    Cancelled,
}

/// 目的：把缓冲区里攒够的完整行进 SSE 一层拆开（空行、注释、data: 前缀、[DONE]），
///    剩下的 JSON 交给协议实现；协议认不出的块跳过，不当错误。
fn feed(
    proto: &dyn Protocol,
    buf: &mut Vec<u8>,
    acc: &mut StreamAcc,
    on: &mut dyn FnMut(Chunk) -> bool,
    cancel: Option<&dyn Cancelled>,
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
        match proto.event(&v, acc, on) {
            EventStep::Continue => {}
            EventStep::Stopped => return Feed::Cancelled,
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
    proto: &dyn Protocol,
    url: &str,
    key: &str,
    body: &str,
    timeout_secs: u64,
    cancel: Option<&dyn Cancelled>,
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
    match proto.reply(&text) {
        Ok(c) => Attempt::Ok(c),
        // 2xx 但不是补全形状（SPA 回落的 HTML 首页等）：这里不是该 API，换下一个候选。
        Err(e) => Attempt::Retry(e),
    }
}

async fn read_text(
    resp: reqwest::Response,
    cancel: Option<&dyn Cancelled>,
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
