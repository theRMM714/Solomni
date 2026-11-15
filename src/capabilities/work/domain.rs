//! 目的：工作形态的纯规则——形态标识、工作名校验、名单规整；不碰端口，不持有状态。
//! 管：`mode_str`、`validate_work_name`、`dedup_modules`、`merge_into_one`。
//! 不管：这些规则怎么被用（`service/create.rs`、`service/config.rs`）、错误怎么外送（调用方）。
//! 联动：`service` 各文件与 `api` 的入站校验经这里取规则。

use crate::capabilities::work::api::{AgentInstance, WorkMode};

/// 目的：工作形态 → 会话元信息里的标识。
pub(crate) fn mode_str(mode: WorkMode) -> &'static str {
    match mode {
        WorkMode::Single => "single",
        WorkMode::Collab => "collab",
        WorkMode::Proxy => "proxy",
    }
}

/// 目的：工作名即将来的落盘目录名：非空、不含路径分隔符与 Windows 非法字符、不是保留名。
pub fn validate_work_name(name: &str) -> Result<(), String> {
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

/// 目的：单模式的组合语义：多个 agent 的模块并成一个**临时组合**实例（去重、保序；模型取核心默认）。
///  人设取空：多份人设无法诚实合并，由用户在会话配置里补。
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
