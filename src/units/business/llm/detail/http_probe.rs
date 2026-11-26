//! 目的：原生工具调用支持探测——两条最小请求把「支不支持」变成事实。
//! 管：带 / 不带 tools 的对比请求；三种结论（支持 / 明确不支持 / 无法判定）的归类与回放形状实测。
//! 不管：通道会话本身（在 http_chat）；wire 形状与回放候选（在 protocol）；探测结果与设置的持久化（在 registry）。
//! 联动：经 api::Channel 与 detail/endpoint.rs 的候选发请求；形状由协议实现给；结论由 registry/service.rs 的发现用例记账。

use crate::kernel::ports::Log;
use crate::units::business::llm::api::Channel;
use crate::units::business::llm::api::{
    Completion, ProbeOutcome, ReplayReport, ReplayShape, ToolDecl,
};
use crate::units::business::llm::detail::endpoint::{Attempt, chat_candidates, resolve_candidates};
use crate::units::business::llm::ports::Protocol;

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
fn once(
    proto: &dyn Protocol,
    url: &str,
    key: &str,
    channel: &Channel,
    with_tools: bool,
) -> Attempt<Completion> {
    let decl = ping_decl();
    let tools: Option<&[ToolDecl]> = if with_tools {
        Some(std::slice::from_ref(&decl))
    } else {
        None
    };
    crate::units::business::llm::detail::http_chat::attempt_with_tools(
        proto,
        url,
        key,
        &channel.model,
        "请调用 solomni_ping 工具，不需要参数。",
        tools,
    )
}

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

/// 目的：探测"工具调用历史怎么发回供应商才收"（机制；不写任何登记处）。
///    为什么要它：原生通道下核心必须把上一轮的工具调用发回去（assistant 的 tool_calls + 各条 tool 结果），
///    这是"回放与实时一致"的载体；但各家兼容实现对这种历史的宽容度不同，
///    所以这里把几种可判定的写法都试一遍，如实回报哪种被接受——不猜，也不替用户拍板。
///    形状本身由协议实现给（detail/protocol）：探测发的必须与线上真发的一致。
pub fn probe_replay(
    channel: &Channel,
    log: &std::sync::Arc<dyn Log + Send + Sync>,
) -> Result<ReplayReport, String> {
    probe_replay_with(channel, log, &probe_nonce())
}

/// 目的：同上，但本次编号由调用方给定（契约测试要断言"模型真的把编号带回来了"）。
pub(crate) fn probe_replay_with(
    channel: &Channel,
    log: &std::sync::Arc<dyn Log + Send + Sync>,
    nonce: &str,
) -> Result<ReplayReport, String> {
    let key = channel.api_key.clone();
    let candidates = chat_candidates(&channel.base_url);
    let decl = ping_decl();
    let tools = Some(std::slice::from_ref(&decl));
    let proto = super::protocol::default_protocol();
    // 判据取编号里最独特的那一段（前缀 '-' 之后）：实测模型会直接把前缀省掉，
    // 只回后半段——那不是"没读懂"，所以判据不能死抠整串。
    let needle = nonce.rsplit('-').next().unwrap_or(nonce).to_lowercase();
    let mut shapes: Vec<ReplayShape> = Vec::new();
    for (name, messages) in proto.replay_histories(nonce) {
        let got = resolve_candidates(
            &candidates,
            |url| {
                crate::units::business::llm::detail::http_chat::attempt_raw(
                    proto.as_ref(),
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
            Ok((_, done)) => ReplayShape {
                name: name.to_string(),
                accepted: true,
                understood: done.raw.to_lowercase().contains(&needle),
                detail: format!(
                    "finish_reason={}，回答前 30 字：{}",
                    done.finish,
                    done.raw.chars().take(30).collect::<String>()
                ),
            },
            Err(e) => ReplayShape {
                name: name.to_string(),
                accepted: false,
                understood: false,
                detail: e,
            },
        });
    }
    // 基线不通 = 通道本身的问题：后面的结论没有意义，如实报"测不了"，不把它们当"形状被拒"。
    if let Some(first) = shapes.first()
        && !first.accepted
    {
        return Err(format!(
            "通道本身就没打通（连现在线上在用的文本回放形状都失败）：{}",
            first.detail
        ));
    }
    Ok(ReplayReport { shapes })
}

/// 目的：探测一条通道（机制；策略在 runtime）。
pub fn probe(
    channel: &Channel,
    log: &std::sync::Arc<dyn Log + Send + Sync>,
) -> Result<ProbeOutcome, String> {
    let key = channel.api_key.clone();
    let candidates = chat_candidates(&channel.base_url);
    let proto = super::protocol::default_protocol();
    // 第一条：不带 tools（证明通道本身通不通）
    let base = resolve_candidates(
        &candidates,
        |url| once(proto.as_ref(), url, &key, channel, false),
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
        |url| once(proto.as_ref(), url, &key, channel, true),
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
