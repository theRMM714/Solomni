//! 目的：名单与模型的落地——把核心拟的名单校验成 agent 实例，并给拟名单提供可用模型清单。
//! 管：resolve_picks（存在性 / 模型 / 模块校验与拒收理由）、unique_instance_name（重名尾号）、model_listing（可用模型清单）。
//! 不管：被校验与取名的形状（在环 1 的 registry_model）；能力注册表（在 catalog 的 modules 子业务）；登记处的写用例（在 service.rs）。
//! 联动：service.rs 的 agent 写入消费 resolve_picks；registry/api.rs 对外重导出这三个函数。

use crate::units::catalog::modules::api::Roster;
use crate::units::pure::registry_model::api::ModelEntry;
use crate::units::pure::registry_model::api::{AgentMeta, Agents, RosterPick, validate_name};
use std::collections::BTreeMap;

/// 目的：可用模型清单（拟名单时给模型看）：id / 名称 / api_model / 说明。
pub fn model_listing(models: &BTreeMap<String, ModelEntry>) -> String {
    models
        .iter()
        .map(|(id, m)| {
            let note = if m.note.trim().is_empty() {
                String::new()
            } else {
                format!("；{}", m.note)
            };
            format!("- {}：{}（{}{}）", id, m.name, m.api_model, note)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 目的：把核心拟的名单落地成 agent 实例：逐条校验（存在性、模型真实），
///    非法条目整条拒收并给出原因（不静默改写）。同一模块可以被多个 agent 共用，同一 agent 内重复只留一份。
///    返回（已接受的实例 + 理由, 拒收说明）。
pub fn resolve_picks(
    picks: Vec<RosterPick>,
    known: &Agents,
    roster: &Roster,
    models: &BTreeMap<String, ModelEntry>,
) -> (Vec<(AgentMeta, String)>, Vec<String>) {
    let mut out: Vec<(AgentMeta, String)> = Vec::new();
    let mut rejected: Vec<String> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for p in picks {
        let why = p.why();
        // 复用项以登记处为准（模块与模型都取它自己的，核心不代拟模型）。
        let (base, modules, model, chara, transient) = match p {
            RosterPick::Reuse { agent, .. } => match known.get(agent.trim()) {
                Some(a) => (
                    agent.trim().to_string(),
                    a.modules.clone(),
                    a.model.clone(),
                    a.chara.clone(),
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
                chara,
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
                (label, modules, Some(model), chara.trim().to_string(), true)
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
            if !local.contains(id) {
                local.push(id.clone());
            }
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
        out.push((
            AgentMeta::normalized(name, local, model, chara, transient),
            why,
        ));
    }
    (out, rejected)
}

/// 目的：同一工作内 agent 名去重：重名时依次追加 -2、-3（用户不改名时的兜底）。
///   后缀探测是内核的名字契约（`kernel::api::name::dedup_suffix`），这里只把 `taken` 变成本次已占集合。
pub fn unique_instance_name(base: &str, taken: &[String]) -> String {
    crate::kernel::api::name::dedup_suffix(base, |c| taken.iter().any(|t| t == c))
}
