//! 目的：work 的领域判据——与状态无关的纯函数（模块去重、单模式合并、会话开没开过、形态标识、工作名校验）。
//! 管：这些判据本身的规则；不碰能力面、不读盘、不发事件。
//! 不管：谁在什么时机调用它们（create.rs / config.rs / conductor 的代理入口）。
//! 联动：create.rs、config.rs 与 conductor 的代理入口共用这里，避免同一规则两处实现。

use crate::capabilities::work::api::{AgentInstance, WorkMode};

/// 目的：模块列表保序去重（一个 agent 内重复只留一份；跨 agent 的重叠是合法的共用）。
pub(crate) fn dedup_modules(list: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(list.len());
    for m in list {
        if !out.contains(m) {
            out.push(m.clone());
        }
    }
    out
}

/// 目的：单模式的组合语义——多个 agent 的模块并成一个**临时组合**实例（去重、保序；模型取核心默认）。
/// 约束：人设取空——多份人设无法诚实合并，由用户在会话配置里补。
pub(crate) fn merge_into_one(picked: &[AgentInstance], name: &str) -> AgentInstance {
    let mut merged: Vec<String> = Vec::new();
    for a in picked {
        for id in &a.modules {
            if !merged.contains(id) {
                merged.push(id.clone());
            }
        }
    }
    AgentInstance {
        name: name.to_string(),
        transient: true,
        modules: merged,
        model: None,
        chara: String::new(),
    }
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

/// 目的：工作形态 → 会话元信息里的标识。
pub(crate) fn mode_str(mode: WorkMode) -> &'static str {
    match mode {
        WorkMode::Single => "single",
        WorkMode::Collab => "collab",
        WorkMode::Proxy => "proxy",
    }
}

/// 目的：工作名即将来的落盘目录名——非空、不含路径分隔符与 Windows 非法字符、不是保留名。
pub(crate) fn validate_work_name(name: &str) -> Result<(), String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("工作名不能为空".to_string());
    }
    if name != n {
        return Err("工作名首尾不能有空白".to_string());
    }
    if n == "." || n == ".." {
        return Err("工作名非法".to_string());
    }
    const BAD: [char; 9] = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    if n.chars().any(|c| BAD.contains(&c) || c.is_control()) {
        return Err("工作名不能包含 \\ / : * ? \" < > | 或控制字符".to_string());
    }
    let upper = n.to_ascii_uppercase();
    let reserved_device = ["CON", "PRN", "AUX", "NUL"].contains(&upper.as_str())
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit());
    if reserved_device {
        return Err("工作名是系统保留名".to_string());
    }
    Ok(())
}
