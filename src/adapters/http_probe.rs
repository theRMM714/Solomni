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
