//! 真实通道：OpenAI 兼容 /chat/completions（可取消的 reqwest 流；见 http_stream）。
//! 网关只做机制：给啥通道（供应商 + 模型）建啥会话；没有通道是调用方的前置条件失败，
//! 这里**不回落假模型**。端点补全/回落规则见 endpoint 模块：无版本段先直连，404/405 再试 /v1。
//! 密钥只在出站调用里使用，永不落提示词/转录/日志；错误信息经脱敏（红线）。

use super::endpoint::{chat_candidates, memo_get, memo_set, resolve_candidates, Attempt, Memo};
use crate::capabilities::llm::api::Channel;
use crate::capabilities::llm::api::{
    BoxedChat, Chat, Chunk, CompleteOpts, Completion, Msg, ProbeOutcome, TokenUsage, ToolDecl,
};
use crate::capabilities::llm::ports::ChatGateway;

/// 真实会话通道：拥有通道副本（含密钥；密钥不出适配层）。
/// resolved = 本会话首次命中的有效端点，后续轮次直接复用，不再重复探测。
pub struct HttpChat {
    pub channel: Channel,
    pub provider_id: String,
    pub log: std::sync::Arc<dyn crate::kernel::ports::Log + Send + Sync>,
    resolved: Option<String>,
    /// 进程内共享的端点记忆：一旦通了就固定，后续会话不再探测。
    memo: Memo,
    memo_key: String,
    /// 最近一次调用供应商报的 usage（拿不到 = None）；会话状态栏与压缩判定读它。
    last_usage: Option<TokenUsage>,
}

/// 一条消息 → wire 形态。**协议字段按需出现**：手写信封通道的消息没有调用，
/// 所以它的请求体按信封协议逐字节构造；只有原生通道的助手/工具消息才带上 tool_calls 与 tool_call_id。
fn msg_json(m: &Msg) -> serde_json::Value {
    let mut v = serde_json::json!({ "role": m.role, "content": m.content });
    if !m.tool_calls.is_empty() {
        v["tool_calls"] = serde_json::Value::Array(
            m.tool_calls
                .iter()
                .map(|c| {
                    serde_json::json!({
                        "id": c.id,
                        "type": "function",
                        "function": { "name": c.name, "arguments": c.args_json },
                    })
                })
                .collect(),
        );
    }
    if !m.tool_call_id.is_empty() {
        v["tool_call_id"] = serde_json::json!(m.tool_call_id);
    }
    v
}

/// 组一次 /chat/completions 的请求体。**真实会话与探针共用同一份形状**——
/// 探针发出去的必须是线上真会发的东西，否则它测出来的结论代表不了线上行为。
fn request_body(
    model: &str,
    stream: bool,
    messages: serde_json::Value,
    tools: Option<&[ToolDecl]>,
    thinking: Option<&serde_json::Value>,
) -> serde_json::Value {
    let mut body = serde_json::json!({ "model": model, "stream": stream, "messages": messages });
    // 流式请求也要 usage（状态栏与自动压缩要有精确数字）；不支持的供应商由 complete 回退掉。
    if stream {
        body["stream_options"] = serde_json::json!({ "include_usage": true });
    }
    // 原生工具调用：把工具声明带给供应商（"参数走结构化槽位"的全部秘密就在这一段）。
    // 不声明 tools = 手写信封模式：模型照旧在正文里写信封，核心自己解析。
    if let Some(tools) = tools {
        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(tools.iter().map(decl_json).collect());
            body["tool_choice"] = serde_json::json!("auto");
        }
    }
    // 思考强度：登记处解析好的片段按模型映射原样合并；保留键在登记处已经拒收，这里再挡一次
    //（覆盖 model / messages / stream / tools 会直接把这次请求弄坏）。
    if let Some(extra) = thinking.and_then(|v| v.as_object()) {
        for (k, v) in extra {
            if !crate::capabilities::llm::api::REQUEST_RESERVED_KEYS.contains(&k.as_str()) {
                body[k] = v.clone();
            }
        }
    }
    body
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

/// 目的：一条最小请求——单条用户消息 + 可选工具声明（探测用；正式对话走 complete）。
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
    // 探针是一次性诊断，用默认预算（它不参与真实会话的设置）。
    attempt_with_cancel(
        url,
        key,
        &body.to_string(),
        crate::kernel::api::DEFAULT_LLM_TIMEOUT_SECS,
        None,
    )
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
    let body = request_body(model, false, messages, tools, None).to_string();
    // 探针是一次性诊断，用默认预算（它不参与真实会话的设置）。
    attempt_with_cancel(
        url,
        key,
        &body,
        crate::kernel::api::DEFAULT_LLM_TIMEOUT_SECS,
        None,
    )
}

impl HttpChat {
    /// 组合根/网关内部构造：通道 + 日志 + 共享端点记忆。
    fn new(
        channel: Channel,
        log: std::sync::Arc<dyn crate::kernel::ports::Log + Send + Sync>,
        memo: Memo,
    ) -> HttpChat {
        let memo_key = format!("{}|chat", channel.base_url);
        let resolved = memo_get(&memo, &memo_key);
        HttpChat {
            provider_id: channel.base_url.clone(),
            channel,
            log,
            resolved,
            memo,
            memo_key,
            last_usage: None,
        }
    }
}

impl HttpChat {
    /// 一次出站交换：端点候选、重试归类、日志都在这里；请求体由调用方给（便于兼容回退换一份）。
    fn exchange(
        &self,
        body: &str,
        opts: &CompleteOpts<'_>,
        on: &mut dyn FnMut(Chunk) -> bool,
    ) -> Result<(String, Completion), String> {
        let candidates: Vec<String> = match &self.resolved {
            Some(url) => vec![url.clone()],
            None => chat_candidates(&self.channel.base_url),
        };
        let key = self.channel.api_key.clone();
        if opts.stream {
            let rt = super::http_stream::runtime()?;
            resolve_candidates(
                &candidates,
                |url| {
                    rt.block_on(super::http_stream::stream_once(
                        url,
                        &key,
                        body,
                        opts.clone(),
                        on,
                    ))
                },
                |url, err, next| {
                    self.log.warn(
                        "http_chat::stream",
                        &format!("端点 {} 不可用（{}），改试 {}", url, err, next),
                    );
                },
            )
        } else {
            let cancel = opts.cancel.clone();
            resolve_candidates(
                &candidates,
                |url| attempt_with_cancel(url, &key, body, opts.timeout_secs, cancel.as_deref()),
                |url, err, next| {
                    self.log.warn(
                        "http_chat::complete",
                        &format!("端点 {} 不可用（{}），改试 {}", url, err, next),
                    );
                },
            )
        }
    }
}

/// 供应商把流式 usage 参数当未知字段拒掉了吗？只在 400/422 且点名字段时回退一次。
fn stream_options_rejected(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    let rejected_status = e.contains("供应商返回 400") || e.contains("供应商返回 422");
    rejected_status
        && ["stream_options", "include_usage", "stream options"]
            .iter()
            .any(|needle| e.contains(needle))
}

impl Chat for HttpChat {
    fn complete(
        &mut self,
        messages: &[Msg],
        opts: CompleteOpts<'_>,
        on: &mut dyn FnMut(Chunk) -> bool,
    ) -> Completion {
        let wire: Vec<serde_json::Value> = messages.iter().map(msg_json).collect();
        let body_value = request_body(
            &self.channel.model,
            opts.stream,
            serde_json::Value::Array(wire),
            opts.tools,
            opts.thinking.as_deref(),
        );
        let body = body_value.to_string();
        let mut outcome = self.exchange(&body, &opts, on);
        // 兼容回退：只在首个 400/422 且错误点名字段时去掉 stream_options 重试一次。
        // 这时还没收到任何数据块（stream_once 在非 2xx 时不回调 on），不会重复正文。
        if opts.stream {
            if let Err(e) = &outcome {
                if stream_options_rejected(e) {
                    self.log.warn(
                        "http_chat::stream",
                        "供应商不接受 stream_options，去掉该参数重试；本次用量将按字符估算",
                    );
                    let mut retry = body_value.clone();
                    if let Some(obj) = retry.as_object_mut() {
                        obj.remove("stream_options");
                    }
                    outcome = self.exchange(&retry.to_string(), &opts, on);
                }
            }
        }
        match outcome {
            Ok((url, done)) => {
                // 用户停止：不把取消当成"成功命中端点"，也不写端点记忆。
                if done.cancelled {
                    return done;
                }
                // 通了就定死：进程内共享，后续会话直接用它，不再探测候选。
                memo_set(&self.memo, &self.memo_key, &url);
                self.resolved = Some(url);
                self.last_usage = done.usage;
                done
            }
            Err(e) => {
                self.log.error(
                    "http_chat::complete",
                    &format!("通道 {} 调用失败：{}", self.provider_id, e),
                );
                // 失败**不是**模型的回复：带上原因、不带正文，由核心如实告知并中断这一轮。
                Completion::failure(format!("模型调用失败：{}", e))
            }
        }
    }

    fn last_usage(&self) -> Option<TokenUsage> {
        self.last_usage
    }
}

/// 非流式调用：没有中间分片可重置，整次调用共用 `timeout_secs`。
/// 探针用 None；真实会话把停止位传进来，停止时请求同样立刻关闭。
fn attempt_with_cancel(
    url: &str,
    key: &str,
    body: &str,
    timeout_secs: u64,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Attempt<Completion> {
    match super::http_stream::runtime() {
        Ok(rt) => rt.block_on(super::http_stream::attempt_async(
            url,
            key,
            body,
            timeout_secs,
            cancel,
        )),
        Err(e) => Attempt::Fatal(e),
    }
}

/// 真实网关：机制 only。通道由调用方解析后传入；这里不制造任何假模型。
pub struct HttpGateway {
    log: std::sync::Arc<dyn crate::kernel::ports::Log + Send + Sync>,
    memo: Memo,
}

impl HttpGateway {
    /// 组合根注入日志端口（异常路径落盘）与共享端点记忆。
    pub fn with_log(
        log: std::sync::Arc<dyn crate::kernel::ports::Log + Send + Sync>,
        memo: Memo,
    ) -> HttpGateway {
        HttpGateway { log, memo }
    }
}

impl ChatGateway for HttpGateway {
    /// 回放形状探测（要真实网络）：转给适配层的实现，结论是"关于这条通道的事实"。
    fn probe_replay(
        &self,
        channel: &Channel,
    ) -> Result<crate::capabilities::llm::api::ReplayReport, String> {
        crate::capabilities::llm::detail::http_probe::probe_replay(channel, &self.log)
    }

    fn probe_tools(&self, channel: &Channel) -> Result<ProbeOutcome, String> {
        crate::capabilities::llm::detail::http_probe::probe(channel, &self.log)
    }

    fn member_channel(&self, channel: &Channel, module_id: &str) -> BoxedChat {
        self.log.info(
            "gateway::member_channel",
            &format!(
                "模块 {} → 供应商 {}（模型 {}）",
                module_id, channel.base_url, channel.model
            ),
        );
        Box::new(HttpChat::new(
            channel.clone(),
            std::sync::Arc::clone(&self.log),
            std::sync::Arc::clone(&self.memo),
        ))
    }

    fn core_channel(&self, channel: &Channel) -> BoxedChat {
        self.log.info(
            "gateway::core_channel",
            &format!(
                "核心通道建立：{}（模型 {}）",
                channel.base_url, channel.model
            ),
        );
        Box::new(HttpChat::new(
            channel.clone(),
            std::sync::Arc::clone(&self.log),
            std::sync::Arc::clone(&self.memo),
        ))
    }
}
