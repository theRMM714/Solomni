//! 目的：work 的领域判据——与状态无关的纯函数（模块去重、单模式合并、会话开没开过、形态标识、工作名校验）。
//! 管：这些判据本身的规则；不碰能力面、不读盘、不发事件。
//! 不管：谁在什么时机调用它们（create.rs / config.rs / runtime 的代理入口）。
//! 联动：create.rs、config.rs 与 runtime 的代理入口共用这里，避免同一规则两处实现。

use crate::capabilities::catalog::registry::api::AgentMeta;

/// 目的：单模式的组合语义——多个 agent 的模块并成一个**临时组合**实例（去重、保序；模型取核心默认）。
/// 约束：人设取空——多份人设无法诚实合并，由用户在会话配置里补。
pub(crate) fn merge_into_one(picked: &[AgentMeta], name: &str) -> AgentMeta {
    let mut merged: Vec<String> = Vec::new();
    for a in picked {
        for id in &a.modules {
            if !merged.contains(id) {
                merged.push(id.clone());
            }
        }
    }
    AgentMeta::normalized(name.to_string(), merged, None, String::new(), true)
}

/// 目的：会话是否「已经开过」（流水里有内容）——agent 名单与形态据此冻结；配置记录与回档记录不算内容。
pub(crate) fn session_started(events: &[serde_json::Value]) -> bool {
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

/// 目的：工作名即将来的落盘目录名——非空、不含路径分隔符与 Windows 非法字符、不是保留名。
///   规则本身是内核的名字契约（`kernel::api::validate_dir_name`），这里只补主语。
pub(crate) fn validate_work_name(name: &str) -> Result<(), String> {
    crate::kernel::api::validate_dir_name(name, "工作名")
}
