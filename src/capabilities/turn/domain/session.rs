//! 目的：会话领域的纯规则——整张名单的扁平模块清单、会话是否"已经开过"。
//! 管：`flat_modules`（多个 agent 的模块保序去重）、`session_started`（流水里有没有内容）。
//! 不管：名单怎么来（调用方给事实）、流水怎么读（`session` 能力）;不碰 IO、不引端口。
//! 联动：`service/tree.rs` 的 `tree_meta_with` 用扁平清单；`conductor` 的创建与配置判据用"已开过"。

use crate::capabilities::session::api::AgentMeta;

/// 目的：整张名单的扁平模块清单（保序去重）：同一模块被多个 agent 共用时只出现一次。
pub fn flat_modules(agents: &[AgentMeta]) -> Vec<String> {
    let mut out = Vec::new();
    for a in agents {
        for m in &a.modules {
            if !out.contains(m) {
                out.push(m.clone());
            }
        }
    }
    out
}

/// 目的：会话是否「已经开过」（流水里有内容）：agent 名单与形态据此冻结。
/// 约束：配置记录与回档记录不算内容；转录只看有没有行。
pub fn session_started(events: &[serde_json::Value]) -> bool {
    events
        .iter()
        .any(|ev| match ev.get("type").and_then(|t| t.as_str()) {
            Some("config") | Some("rewind") => false,
            Some("transcript") => ev
                .get("lines")
                .and_then(|l| l.as_array())
                .map(|l| !l.is_empty())
                .unwrap_or(false),
            Some(_) => true,
            None => false,
        })
}
