//! 目的：OpenAI 兼容协议的 wire 形状——/chat/completions 的请求体、非流式回执、流式分块与用量归一。
//! 管：角色与消息映射、工具声明映射、usage 两种口径归一、流式 delta 累积、回放形状候选。
//! 不管：出站机制（端点候选、超时、取消、SSE 分行、重试）在 http_chat / http_stream；协议无关的消息形状在 api。
//! 联动：由 http_chat / http_stream / http_probe 经 ports::Protocol 调用；**全项目只有这里认识 wire 字段名**。

use crate::units::business::llm::api::{Completion, Msg, Role, TokenUsage, ToolCall, ToolDecl};
use crate::units::business::llm::domain::stream::StreamAcc;
use crate::units::business::llm::ports::{EventStep, Protocol, Request};
use crate::units::pure::registry_model::api::RESERVED_REQUEST_KEYS;

/// 目的：OpenAI 兼容协议（/chat/completions；绝大多数供应商与中转层都用这个形状）。
pub struct OpenAiCompat;

/// 目的：这一种协议的唯一构造点（选择点：目前只有一种实现，见 detail/protocol.rs）。
pub fn new() -> OpenAiCompat {
    OpenAiCompat
}

/// 目的：协议无关的角色 → 这一家 wire 上的字符串。
fn role_wire(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

/// 一条消息 → wire 形态。**协议字段按需出现**：手写信封通道的消息没有调用，
/// 所以它的请求体按信封协议逐字节构造；只有原生通道的助手/工具消息才带上 tool_calls 与 tool_call_id。
fn msg_json(m: &Msg) -> serde_json::Value {
    let mut v = serde_json::json!({ "role": role_wire(m.role), "content": m.content });
    if !m.calls.is_empty() {
        v["tool_calls"] = serde_json::Value::Array(
            m.calls
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
    if !m.answering.is_empty() {
        v["tool_call_id"] = serde_json::json!(m.answering);
    }
    v
}

/// 组一次 /chat/completions 的请求体骨架。**真实会话与探针共用同一份形状**——
/// 探针发出去的必须是线上真会发的东西，否则它测出来的结论代表不了线上行为。
fn body_skeleton(model: &str, messages: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "model": model, "stream": false, "messages": messages })
}

/// 原生工具调用：把工具声明带给供应商（「参数走结构化槽位」的全部秘密就在这一段）。
/// 不声明 tools = 手写信封模式：模型照旧在正文里写信封，核心自己解析。
fn with_tools(mut body: serde_json::Value, tools: Option<&[ToolDecl]>) -> serde_json::Value {
    if let Some(tools) = tools
        && !tools.is_empty()
    {
        body["tools"] = serde_json::Value::Array(tools.iter().map(decl_json).collect());
        body["tool_choice"] = serde_json::json!("auto");
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

/// 从响应/流数据块里取 usage（OpenAI 与 Anthropic 两种形状；缺的字段就是 None）。
/// 归一的口径见函数体：两种形状的输入含义不同，prompt 必须是**这次请求的完整输入**。
/// 全零 / 没有 usage = None：不把「供应商没报」伪装成精确的 0。
fn usage_of(v: &serde_json::Value) -> Option<TokenUsage> {
    let u = v.get("usage")?;
    let n = |keys: &[&str]| keys.iter().find_map(|k| u.get(*k).and_then(|x| x.as_u64()));
    // 缓存：Anthropic 形状给顶层两个字段；OpenAI 形状给 prompt_tokens_details.cached_tokens。
    let cache_read = n(&["cache_read_input_tokens"]).or_else(|| {
        u.get("prompt_tokens_details")
            .and_then(|d| d.get("cached_tokens"))
            .and_then(|x| x.as_u64())
    });
    let cache_creation = n(&["cache_creation_input_tokens"]);
    // **两种形状的输入口径不同**：Anthropic 的 input_tokens 只是「没命中缓存的输入」，缓存读写单独报；
    // OpenAI 的 prompt_tokens 已含命中缓存的部分（同时给两字段的转换层按 OpenAI 读）。
    let anthropic = u.get("prompt_tokens").is_none() && u.get("input_tokens").is_some();
    let prompt = if anthropic {
        n(&["input_tokens"]).unwrap_or(0) + cache_read.unwrap_or(0) + cache_creation.unwrap_or(0)
    } else {
        n(&["prompt_tokens"]).unwrap_or(0)
    };
    let completion = n(&["completion_tokens", "output_tokens"]).unwrap_or(0);
    // Anthropic 若也回了 total_tokens，那多半是不含缓存的旧口径：按归一后的 prompt 重算。
    let total = if anthropic {
        (prompt > 0 || completion > 0).then_some(prompt + completion)
    } else {
        n(&["total_tokens"])
            .or_else(|| (prompt > 0 || completion > 0).then_some(prompt + completion))
    };
    if prompt == 0 && completion == 0 && total.unwrap_or(0) == 0 {
        return None;
    }
    let mut usage = TokenUsage::new(prompt, completion, total);
    usage.cache_read = cache_read;
    usage.cache_creation = cache_creation;
    Some(usage)
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
        usage: usage_of(&v),
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

impl Protocol for OpenAiCompat {
    fn reserved_keys(&self) -> &'static [&'static str] {
        &RESERVED_REQUEST_KEYS
    }

    fn request(&self, req: &Request<'_>) -> serde_json::Value {
        let wire: Vec<serde_json::Value> = req.messages.iter().map(msg_json).collect();
        let mut body = with_tools(
            body_skeleton(req.model, serde_json::Value::Array(wire)),
            req.tools,
        );
        body["stream"] = serde_json::json!(req.stream);
        // 流式请求也要 usage（状态栏与自动压缩要有精确数字）；不支持的供应商由 complete 回退掉。
        if req.stream {
            body["stream_options"] = serde_json::json!({ "include_usage": true });
        }
        // 思考强度：登记处解析好的片段按模型映射原样合并；保留键在登记处已经拒收，这里再挡一次
        //（覆盖 model / messages / stream / tools 会直接把这次请求弄坏）。
        if let Some(extra) = req.thinking.and_then(|v| v.as_object()) {
            for (k, v) in extra {
                if !self.reserved_keys().contains(&k.as_str()) {
                    body[k] = v.clone();
                }
            }
        }
        body
    }

    fn request_raw(
        &self,
        model: &str,
        messages: serde_json::Value,
        tools: Option<&[ToolDecl]>,
    ) -> serde_json::Value {
        with_tools(body_skeleton(model, messages), tools)
    }

    fn reply(&self, text: &str) -> Result<Completion, String> {
        parse_content(text)
    }

    fn event(
        &self,
        v: &serde_json::Value,
        acc: &mut StreamAcc,
        on: &mut dyn FnMut(crate::units::business::llm::api::Chunk) -> bool,
    ) -> EventStep {
        // 供应商可以在最后一块只发 usage（没有 choices）；先收用量，再处理正文。
        if let Some(usage) = usage_of(v) {
            acc.absorb_usage(usage);
        }
        let Some(choice) = v.get("choices").and_then(|c| c.get(0)) else {
            return EventStep::Continue;
        };
        if let Some(f) = choice.get("finish_reason").and_then(|f| f.as_str()) {
            acc.set_finish(f);
        }
        let Some(delta) = choice.get("delta") else {
            return EventStep::Continue;
        };
        if let Some(arr) = delta.get("tool_calls").and_then(|t| t.as_array()) {
            if !arr.is_empty() {
                acc.got_any = true;
            }
            for item in arr {
                let idx = item
                    .get("index")
                    .and_then(|i| i.as_u64())
                    .unwrap_or(acc.calls.len() as u64) as usize;
                let slot = acc.call_slot(idx);
                if let Some(id) = item.get("id").and_then(|i| i.as_str())
                    && !id.is_empty()
                {
                    slot.id = id.to_string();
                }
                if let Some(f) = item.get("function") {
                    if let Some(n) = f.get("name").and_then(|n| n.as_str())
                        && !n.is_empty()
                    {
                        slot.name = n.to_string();
                    }
                    if let Some(a) = f.get("arguments").and_then(|a| a.as_str()) {
                        slot.args_json.push_str(a);
                    }
                }
            }
        }
        if let Some(t) = delta.get("content").and_then(|c| c.as_str())
            && let Some(piece) = acc.absorb_text(t)
            && !on(crate::units::business::llm::api::Chunk::Text(piece))
        {
            return EventStep::Stopped;
        }
        let reasoning = delta
            .get("reasoning_content")
            .and_then(|c| c.as_str())
            .or_else(|| delta.get("reasoning").and_then(|c| c.as_str()));
        if let Some(r) = reasoning
            && let Some(piece) = acc.absorb_reasoning(r)
            && !on(crate::units::business::llm::api::Chunk::Reasoning(piece))
        {
            return EventStep::Stopped;
        }
        EventStep::Continue
    }

    fn drop_stream_usage(&self, body: &mut serde_json::Value) -> bool {
        match body.as_object_mut() {
            Some(obj) => obj.remove("stream_options").is_some(),
            None => false,
        }
    }

    fn replay_histories(&self, nonce: &str) -> Vec<(&'static str, serde_json::Value)> {
        replay_shapes(nonce)
    }
}

/// 探针的合成调用 id（产物里不出现供应商真给的 id）。
const PROBE_CALL_ID: &str = "call_solomni_probe";

/// 目的：一种候选的回放形状：报告里点名的名字 + 该形状要发的 messages 数组。
///    第一项固定是**基线**（现在线上真在用的形状），用来先证明通道与「文本回放」是通的；
///    后面几项是同一件事（把上一轮的工具调用发回去）的几种可判定写法，一次探测全试一遍。
///    工具结果里放的是本次编号（由调用方给出）：回答里带回它 = 真的读到了。
pub(crate) fn replay_shapes(nonce: &str) -> Vec<(&'static str, serde_json::Value)> {
    let outcome = serde_json::json!({ "nonce": nonce }).to_string();
    let head = serde_json::json!({
        "role": "user",
        "content": "上一轮你调用过 solomni_ping；请只回答工具结果里的那个编号，不要解释。",
    });
    let call = |content: serde_json::Value| {
        serde_json::json!({
            "role": "assistant",
            "content": content,
            "tool_calls": [{
                "id": PROBE_CALL_ID,
                "type": "function",
                "function": { "name": "solomni_ping", "arguments": "{}" },
            }],
        })
    };
    let result = |extra: Option<serde_json::Value>| {
        let mut tool = serde_json::json!({
            "role": "tool",
            "tool_call_id": PROBE_CALL_ID,
            "content": outcome,
        });
        if let Some(serde_json::Value::Object(extra)) = extra {
            for (k, v) in extra {
                tool[k] = v;
            }
        }
        tool
    };
    let with = |assistant: serde_json::Value, tool: serde_json::Value| {
        serde_json::json!([head, assistant, tool])
    };
    vec![
        // 基线：现在线上真在用的形状（助手正文记下调了什么 + 结果当用户消息发回去）。
        (
            "baseline-text",
            serde_json::json!([
                head,
                { "role": "assistant", "content": "[原生工具调用] solomni_ping {}" },
                { "role": "user", "content": format!("[工具结果] solomni_ping\n{}", outcome) },
            ]),
        ),
        // 协议形状：assistant 的 tool_calls + role=tool 的结果（模型真写正文时常给空串）。
        (
            "content-empty",
            with(call(serde_json::json!("")), result(None)),
        ),
        (
            "content-null",
            with(call(serde_json::Value::Null), result(None)),
        ),
        (
            "content-prose",
            with(call(serde_json::json!("我调用了探针工具。")), result(None)),
        ),
        (
            "tool-with-name",
            with(
                call(serde_json::json!("")),
                result(Some(serde_json::json!({ "name": "solomni_ping" }))),
            ),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 目的：usage 归一成供应商形状能给的四个数——缓存读/写缺失就是 None，不伪造成 0。
    #[test]
    fn usage_of_reads_cache_fields_without_inventing_zeros() {
        let openai = json!({"usage": {"prompt_tokens": 900, "completion_tokens": 30,
            "prompt_tokens_details": {"cached_tokens": 800}}});
        let u = usage_of(&openai).expect("有 usage");
        assert_eq!(
            (u.prompt_tokens, u.completion_tokens, u.total_tokens),
            (900, 30, 930)
        );
        assert_eq!(u.cache_read, Some(800));
        assert_eq!(u.cache_creation, None, "没报缓存写入就是 None");

        // Anthropic 的 input_tokens 只是"没命中缓存的那部分"：prompt 必须把缓存读写加回来，
        // 否则 total 会小得离谱（50+10=60 vs 真实 1560），自动压缩的阈值也就永远触发不了。
        let anthropic = json!({"usage": {"input_tokens": 50, "output_tokens": 10,
            "cache_read_input_tokens": 1200, "cache_creation_input_tokens": 300}});
        let u = usage_of(&anthropic).expect("有 usage");
        assert_eq!(
            (u.prompt_tokens, u.completion_tokens, u.total_tokens),
            (1550, 10, 1560)
        );
        assert_eq!(u.cache_read, Some(1200));
        assert_eq!(u.cache_creation, Some(300));

        let plain = json!({"usage": {"prompt_tokens": 7, "completion_tokens": 3}});
        let u = usage_of(&plain).expect("有 usage");
        assert_eq!((u.cache_read, u.cache_creation), (None, None));
        assert!(
            usage_of(&json!({"usage": {"prompt_tokens": 0, "completion_tokens": 0}})).is_none()
        );
    }

    /// 目的：流式分块只报一部分字段时，后一块不把前一块已知的输入/缓存抹成 0。
    #[test]
    fn usage_chunks_merge_field_wise() {
        let tail = usage_of(&json!({"usage": {"output_tokens": 10}})).expect("有 output");
        let head = usage_of(&json!({"usage": {"input_tokens": 50,
            "cache_read_input_tokens": 1200, "cache_creation_input_tokens": 300}}))
        .expect("有 input");
        let merged = tail.widen(head);
        assert_eq!(
            (
                merged.prompt_tokens,
                merged.completion_tokens,
                merged.total_tokens
            ),
            (1550, 10, 1560)
        );
        assert_eq!(merged.cache_read, Some(1200));
        assert_eq!(merged.cache_creation, Some(300));
        // 反过来并也一样（谁先到都一样）。
        assert_eq!(tail.widen(head).total_tokens, head.widen(tail).total_tokens);
    }

    /// 形状表：第一项是基线，每一项都是"上一轮调用过工具"的**完整**历史，
    /// 且覆盖那几处真会写不一样的字段（content 空串 / null / 有正文 / 结果消息带 name）。
    #[test]
    fn replay_shapes_are_complete_histories_covering_the_writable_differences() {
        let shapes = replay_shapes("nonce-xyz");
        let names: Vec<&str> = shapes.iter().map(|(n, _)| *n).collect();
        assert_eq!(
            names.first().copied(),
            Some("baseline-text"),
            "第一项必须是基线"
        );
        for (name, msgs) in &shapes {
            let arr = msgs
                .as_array()
                .unwrap_or_else(|| panic!("{} 的 messages 要是数组", name));
            assert_eq!(
                arr.len(),
                3,
                "{} 要发完整三条：用户问题 + 助手回合 + 工具结果",
                name
            );
            assert!(
                arr[1].get("role").and_then(|r| r.as_str()) == Some("assistant"),
                "{} 的第二条是助手回合",
                name
            );
        }
        let by = |name: &str| {
            shapes
                .iter()
                .find(|(n, _)| *n == name)
                .unwrap_or_else(|| panic!("缺形状 {}", name))
                .1
                .clone()
        };
        // 基线 = 现在线上在用的文本回放：没有协议字段。
        let base = by("baseline-text");
        assert!(base[1].get("tool_calls").is_none(), "基线不带 tool_calls");
        assert_eq!(base[2].get("role").and_then(|r| r.as_str()), Some("user"));
        // 协议形状：助手回合带 tool_calls（id + arguments），结果消息用同一个 tool_call_id 对应。
        for name in [
            "content-empty",
            "content-null",
            "content-prose",
            "tool-with-name",
        ] {
            let v = by(name);
            assert_eq!(v[1]["tool_calls"][0]["id"], PROBE_CALL_ID, "{}", name);
            assert_eq!(v[1]["tool_calls"][0]["type"], "function", "{}", name);
            assert_eq!(
                v[1]["tool_calls"][0]["function"]["arguments"], "{}",
                "{}",
                name
            );
            assert_eq!(
                v[2].get("role").and_then(|r| r.as_str()),
                Some("tool"),
                "{}",
                name
            );
            assert_eq!(v[2]["tool_call_id"], PROBE_CALL_ID, "{}", name);
        }
        assert_eq!(by("content-empty")[1]["content"], "");
        assert!(by("content-null")[1]["content"].is_null());
        assert_eq!(by("content-prose")[1]["content"], "我调用了探针工具。");
        assert_eq!(by("tool-with-name")[2]["name"], "solomni_ping");
        // 工具结果里放着本次编号：它是"模型真的读到了"的唯一凭据。
        for (name, _) in &shapes {
            let content = by(name)[2]["content"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            assert!(
                content.contains("nonce-xyz"),
                "{} 的结果里要带本次编号：{}",
                name,
                content
            );
        }
    }
}
