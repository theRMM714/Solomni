//! agent：用户配置的具名能力组合（一个 AI 实例 + N 份模块能力 + 一个模型）。
//! 只是"用户编排选择的存档"，不是能力注册表——能力仍在 modules/ 公地里。
//! 落盘在 .home/agents.yaml（与 providers/models/settings 同处用户私有区）。

use crate::core::history::AgentMeta;
use crate::core::module::Roster;
use crate::core::prompt::Prompts;
use crate::core::providers::ModelEntry;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 一个具名 agent。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    /// 该 agent 的成员模块（能力）。
    pub modules: Vec<String>,
    /// 默认模型（模型 id）；None = 用核心默认。
    #[serde(default)]
    pub model: Option<String>,
    /// 一句话说明（给用户看）。
    #[serde(default)]
    pub note: String,
}

/// agent 登记处（agents.yaml 的内存形态）。
pub type Agents = BTreeMap<String, Agent>;

/// 展示视图。
#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentView {
    pub name: String,
    pub modules: Vec<String>,
    pub model: Option<String>,
    pub note: String,
}

/// agent 名即将成为（session 内）目录名：非空、不含路径分隔符与 Windows 非法字符、不是保留名。
pub fn validate_name(name: &str) -> Result<(), String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("agent 名不能为空".to_string());
    }
    if name != n {
        return Err("agent 名首尾不能有空白".to_string());
    }
    if n == "." || n == ".." {
        return Err("agent 名非法".to_string());
    }
    const BAD: [char; 9] = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    if n.chars().any(|c| BAD.contains(&c) || c.is_control()) {
        return Err("agent 名不能包含 \\ / : * ? \" < > | 或控制字符".to_string());
    }
    let upper = n.to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&upper.as_str())
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit());
    if reserved {
        return Err("agent 名是系统保留名".to_string());
    }
    Ok(())
}

/// 已存 agent 清单（拟名单时给模型看）：名字 / 模块 / 模型 / 说明；空登记处用册子里的说法。
pub fn listing(prompts: &Prompts, known: &Agents) -> String {
    if known.is_empty() {
        return prompts.core.no_agents.clone();
    }
    let texts = &prompts.core.tool_texts;
    known
        .iter()
        .map(|(name, a)| {
            let model = a
                .model
                .clone()
                .unwrap_or_else(|| prompts.core.no_model.clone());
            let note = if a.note.trim().is_empty() {
                String::new()
            } else {
                format!("；{}", a.note.trim())
            };
            texts.render(
                &texts.agent_listing_line,
                &[
                    ("name", name.clone()),
                    ("modules", a.modules.join(" + ")),
                    ("model", model),
                    ("note", note),
                ],
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 可用模型清单（拟名单时给模型看）：id / 名称 / api_model / 说明。
pub fn model_listing(
    models: &BTreeMap<String, ModelEntry>,
    texts: &crate::core::prompt::ToolTexts,
) -> String {
    models
        .iter()
        .map(|(id, m)| {
            let note = if m.note.trim().is_empty() {
                String::new()
            } else {
                format!("；{}", m.note)
            };
            texts.render(
                &texts.model_listing_line,
                &[
                    ("id", id.clone()),
                    ("name", m.name.clone()),
                    ("api_model", m.api_model.clone()),
                    ("note", note),
                ],
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 核心拟名单时的一条：复用已存 agent，或组装一个新的（JSON 里二选一；Reuse 在前）。
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RosterPick {
    /// 复用：只报登记处里的 agent 名。
    Reuse {
        agent: String,
        #[serde(default)]
        why: String,
    },
    /// 组装：给出新实例的名字、模块与模型。
    Build {
        #[serde(default)]
        name: String,
        modules: Vec<String>,
        model: String,
        #[serde(default)]
        why: String,
    },
}

impl RosterPick {
    fn why(&self) -> String {
        match self {
            RosterPick::Reuse { why, .. } | RosterPick::Build { why, .. } => why.clone(),
        }
    }
}

/// 把核心拟的名单落地成 agent 实例：逐条校验（存在性、模型真实、模块不重复），
/// 非法条目整条拒收并给出原因（不静默改写）。复用项带进来的模块同样参与去重。
/// 返回（已接受的实例 + 理由, 拒收说明）。
pub fn resolve_picks(
    picks: Vec<RosterPick>,
    known: &Agents,
    roster: &Roster,
    models: &BTreeMap<String, ModelEntry>,
) -> (Vec<(AgentMeta, String)>, Vec<String>) {
    let mut out: Vec<(AgentMeta, String)> = Vec::new();
    let mut rejected: Vec<String> = Vec::new();
    let mut used: Vec<String> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for p in picks {
        let why = p.why();
        // 复用项以登记处为准（模块与模型都取它自己的，核心不代拟模型）。
        let (base, modules, model, transient) = match p {
            RosterPick::Reuse { agent, .. } => match known.get(agent.trim()) {
                Some(a) => (
                    agent.trim().to_string(),
                    a.modules.clone(),
                    a.model.clone(),
                    false,
                ),
                None => {
                    rejected.push(format!("agent {} 不在登记处", agent.trim()));
                    continue;
                }
            },
            RosterPick::Build {
                name,
                modules,
                model,
                ..
            } => {
                let label = if name.trim().is_empty() {
                    "组装项".to_string()
                } else {
                    name.trim().to_string()
                };
                if !models.contains_key(&model) {
                    rejected.push(format!("{} 指定的模型 {} 不存在", label, model));
                    continue;
                }
                (label, modules, Some(model), true)
            }
        };
        let label = base.clone();
        let mut local: Vec<String> = Vec::new();
        let mut bad = false;
        for id in &modules {
            if !roster.modules.iter().any(|m| &m.manifest.id == id) {
                rejected.push(format!("{} 的模块 {} 不存在", label, id));
                bad = true;
                break;
            }
            if used.iter().any(|u| u == id) || local.iter().any(|u| u == id) {
                rejected.push(format!("模块 {} 在名单里出现了两次（{}）", id, label));
                bad = true;
                break;
            }
            local.push(id.clone());
        }
        if bad {
            continue;
        }
        if local.is_empty() {
            rejected.push(format!("{} 没有任何模块", label));
            continue;
        }
        // 名字不合法就用首个模块 id 兜底；同名单内重名追加尾号。
        let name = if validate_name(&base).is_ok() {
            base
        } else {
            local[0].clone()
        };
        let name = unique_instance_name(&name, &names);
        names.push(name.clone());
        used.extend(local.iter().cloned());
        out.push((
            AgentMeta {
                name,
                transient,
                modules: local,
                model,
            },
            why,
        ));
    }
    (out, rejected)
}

/// 同一工作内 agent 名去重：重名时依次追加 -2、-3（用户不改名时的兜底）。
pub fn unique_instance_name(base: &str, taken: &[String]) -> String {
    if !taken.iter().any(|t| t == base) {
        return base.to_string();
    }
    let mut n = 2;
    loop {
        let cand = format!("{}-{}", base, n);
        if !taken.iter().any(|t| t == &cand) {
            return cand;
        }
        n += 1;
    }
}
