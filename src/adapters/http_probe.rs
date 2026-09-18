//! 原生工具调用支持探测：发**两条最小请求**对比，把"支不支持"变成事实。
//! 为什么要两条：不带 tools 的那条先证明通道本身是通的（密钥/端点/模型都对），
//! 这样"带上 tools 才失败"才能归因到 tools 上——否则 401 之类的错误会被误判成"不支持工具调用"。
//! 三种结论都如实回报（支持 / 明确不支持 / 无法判定），绝不替用户拍板。

use crate::adapters::endpoint::{chat_candidates, resolve_candidates, Attempt};
use crate::core::ports::{Completion, Log, ProbeOutcome, ToolDecl};
use crate::core::providers::Channel;

/// 探针工具：无参数、只有说明——目的是让模型有东西可调。
fn ping_decl() -> ToolDecl {
    ToolDecl {
        name: "solomni_ping".to_string(),
        description: "探针工具：无参数，调用后返回 pong。".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {},
            "required": [],
            "additionalProperties": false,
        }),
    }
}

/// 一次最小请求（可选带工具声明）；返回适配层的尝试归类。
fn once(url: &str, key: &str, channel: &Channel, with_tools: bool) -> Attempt<Completion> {
    let decl = ping_decl();
    let tools: Option<&[ToolDecl]> = if with_tools {
        Some(std::slice::from_ref(&decl))
    } else {
        None
    };
    crate::adapters::http_chat::attempt_with_tools(
        url,
        key,
        &channel.model,
        "请调用 solomni_ping 工具，不需要参数。",
        tools,
    )
}

/// 探针的合成调用 id（产物里不出现供应商真给的 id）。
const PROBE_CALL_ID: &str = "call_solomni_probe";

/// 本次探测的编号：每跑一次都不一样，且只出现在**工具结果**里。
/// 为什么要它：200 只说明供应商收下了这个形状，不等于模型看懂了那段历史——
/// 只有"回答里带回了这个编号"才能证明它真的读到了。
fn probe_nonce() -> String {
    let bits = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() ^ d.subsec_nanos() as u64)
        .unwrap_or(0);
    format!("solomni-{:x}", bits)
}

/// 一种候选的回放形状：报告里点名的名字 + 该形状要发的 messages 数组。
/// 第一项固定是**基线**（现在线上真在用的形状），用来先证明通道与"文本回放"是通的；
/// 后面几项是同一件事（把上一轮的工具调用发回去）的几种可判定写法，一次探测全试一遍。
/// 工具结果里放的是本次编号（由调用方给出）：回答里带回它 = 真的读到了。
pub fn replay_shapes(nonce: &str) -> Vec<(&'static str, serde_json::Value)> {
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

/// 一种回放形状的探测结论：收了没有（HTTP 层）+ 看懂了没有（回答里带回了本次编号）+ 供应商原话或回答片段。
#[derive(Debug, Clone)]
pub struct ShapeResult {
    pub name: String,
    /// 供应商接受了这个形状。
    pub accepted: bool,
    /// 模型真的读到了那段历史（回答里带回了工具结果里的编号）。
    pub understood: bool,
    pub detail: String,
}

/// 回放形状探测报告：形状按探测顺序排列，第一项是基线。
#[derive(Debug, Clone)]
pub struct ReplayProbe {
    pub shapes: Vec<ShapeResult>,
}

/// 探测"工具调用历史怎么发回供应商才收"（机制；不写任何登记处）。
///
/// 为什么要它：原生通道下核心必须把上一轮的工具调用发回去（assistant 的 tool_calls + 各条 tool 结果），
/// 这是"回放与实时一致"的载体；但各家兼容实现对这种历史的宽容度不同，
/// 所以这里把几种可判定的写法都试一遍，如实回报哪种被接受——不猜，也不替用户拍板。
pub fn probe_replay(
    channel: &Channel,
    log: &std::sync::Arc<dyn Log + Send + Sync>,
) -> Result<ReplayProbe, String> {
    probe_replay_with(channel, log, &probe_nonce())
}

/// 同上，但本次编号由调用方给定（契约测试要断言"模型真的把编号带回来了"）。
pub(crate) fn probe_replay_with(
    channel: &Channel,
    log: &std::sync::Arc<dyn Log + Send + Sync>,
    nonce: &str,
) -> Result<ReplayProbe, String> {
    let key = channel.provider.api_key.clone();
    let candidates = chat_candidates(&channel.provider.base_url);
    let decl = ping_decl();
    let tools = Some(std::slice::from_ref(&decl));
    // 判据取编号里最独特的那一段（前缀 '-' 之后）：实测模型会直接把前缀省掉，
    // 只回后半段——那不是"没读懂"，所以判据不能死抠整串。
    let needle = nonce.rsplit('-').next().unwrap_or(nonce).to_lowercase();
    let mut shapes: Vec<ShapeResult> = Vec::new();
    for (name, messages) in replay_shapes(nonce) {
        let got = resolve_candidates(
            &candidates,
            |url| {
                crate::adapters::http_chat::attempt_raw(
                    url,
                    &key,
                    &channel.model,
                    messages.clone(),
                    tools,
                )
            },
            |url, err, next| {
                log.warn(
                    "probe::replay",
                    &format!("端点 {} 不可用（{}），改试 {}", url, err, next),
                )
            },
        );
        shapes.push(match got {
            Ok((_, done)) => ShapeResult {
                name: name.to_string(),
                accepted: true,
                understood: done.raw.to_lowercase().contains(&needle),
                detail: format!(
                    "finish_reason={}，回答前 30 字：{}",
                    done.finish,
                    done.raw.chars().take(30).collect::<String>()
                ),
            },
            Err(e) => ShapeResult {
                name: name.to_string(),
                accepted: false,
                understood: false,
                detail: e,
            },
        });
    }
    // 基线不通 = 通道本身的问题：后面的结论没有意义，如实报"测不了"，不把它们当"形状被拒"。
    if let Some(first) = shapes.first() {
        if !first.accepted {
            return Err(format!(
                "通道本身就没打通（连现在线上在用的文本回放形状都失败）：{}",
                first.detail
            ));
        }
    }
    Ok(ReplayProbe { shapes })
}

/// 探测一条通道（机制；策略在 core）。
pub fn probe(
    channel: &Channel,
    log: &std::sync::Arc<dyn Log + Send + Sync>,
) -> Result<ProbeOutcome, String> {
    let key = channel.provider.api_key.clone();
    let candidates = chat_candidates(&channel.provider.base_url);
    // 第一条：不带 tools（证明通道本身通不通）
    let base = resolve_candidates(
        &candidates,
        |url| once(url, &key, channel, false),
        |url, err, next| {
            log.warn(
                "probe::base",
                &format!("端点 {} 不可用（{}），改试 {}", url, err, next),
            )
        },
    );
    let (_, baseline) =
        base.map_err(|e| format!("通道本身就没打通（不带工具声明的请求也失败）：{}", e))?;
    // 第二条：带 tools（这一条才是在测"支不支持工具调用"）
    let with = resolve_candidates(
        &candidates,
        |url| once(url, &key, channel, true),
        |url, err, next| {
            log.warn(
                "probe::tools",
                &format!("端点 {} 不可用（{}），改试 {}", url, err, next),
            )
        },
    );
    match with {
        Ok((_, done)) => {
            if let Some(first) = done.calls.first() {
                Ok(ProbeOutcome::Supported {
                    detail: format!(
                        "供应商返回了工具调用（name={} finish_reason={}）",
                        first.name, done.finish
                    ),
                })
            } else {
                Ok(ProbeOutcome::Unknown {
                    detail: format!(
                        "带了工具声明，但这次没有发起调用（finish_reason={}，正文前 40 字：{}）；不带声明的那次是 {}，说明通道本身是通的——可能只是模型没选它",
                        done.finish,
                        done.raw.chars().take(40).collect::<String>(),
                        baseline.finish
                    ),
                })
            }
        }
        // 不带 tools 通、带 tools 失败 = 这一条通道不收工具声明
        Err(e) => Ok(ProbeOutcome::Unsupported { detail: e }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
