//! 目的：agent 的存档形态与解析——用户配置的具名能力组合。
//! 管：Agent / AgentView；名字校验与保留名检查；RosterPick 解析与唯一实例命名；列表文案。
//! 不管：能力注册表（在 modules/ 公地）；落盘（在 detail/yaml_settings.rs）；登记处用例（在 service.rs）。
//! 联动：被 registry/api.rs 重导出；service.rs 读写；runtime 按 agent 名取 Roster。

use crate::capabilities::catalog::modules::api::Roster;
use crate::capabilities::catalog::registry::domain::providers::ModelEntry;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 目的：会话里的一个 agent 实例记录——会话名单的唯一形状（R15）：登记处视图经
///   `agent_meta` 转换，代理建会话与核心点名都复用它。归登记处所有：agent 的身份来自登记处，
///   会话只持有快照并与会话一起走（方向是 session → registry，不是反过来）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMeta {
    /// 目的：实例名（重名时已加 -2/-3；也是其沙箱目录名）。
    pub name: String,
    /// 目的：是否临时 agent（不来自 agents.yaml）。
    #[serde(default)]
    pub transient: bool,
    pub modules: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// 目的：人设 / 身份（建会话时从登记处或临时声明里快照；之后与会话一起走）。
    #[serde(default)]
    pub chara: String,
}
impl AgentMeta {
    /// 目的：会话名单里一个 agent 实例的**唯一**构造口径：名字与身份去首尾空白、模块保序去重、空模型归一成 `None`。
    /// 参数：`transient` 由调用方定——它表达"这份名单来自 agents.yaml 还是本次临时声明"，本函数不猜。
    /// 约束：只做规整；名字合法性见 `validate_name`，模型在不在册归登记处判定。
    pub fn normalized(
        name: impl Into<String>,
        modules: Vec<String>,
        model: Option<String>,
        chara: impl Into<String>,
        transient: bool,
    ) -> AgentMeta {
        let mut unique: Vec<String> = Vec::with_capacity(modules.len());
        for m in modules {
            let id = m.trim().to_string();
            if !id.is_empty() && !unique.contains(&id) {
                unique.push(id);
            }
        }
        AgentMeta {
            name: name.into().trim().to_string(),
            transient,
            modules: unique,
            model: model
                .map(|m| m.trim().to_string())
                .filter(|m| !m.is_empty()),
            chara: chara.into().trim().to_string(),
        }
    }
}

/// 目的：一个具名 agent。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    /// 目的：该 agent 的成员模块（能力）。
    pub modules: Vec<String>,
    /// 目的：默认模型（模型 id）；None = 用核心默认。
    #[serde(default)]
    pub model: Option<String>,
    /// 目的：一句话说明（给用户看）。
    #[serde(default)]
    pub note: String,
    /// 目的：人设 / 身份（给模型看）：进入这个 agent 的身份提示词；可选、可多行。
    #[serde(default)]
    pub chara: String,
}

/// 目的：agent 登记处（agents.yaml 的内存形态）。
pub type Agents = BTreeMap<String, Agent>;

/// 目的：展示视图。
#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentView {
    pub name: String,
    pub modules: Vec<String>,
    pub model: Option<String>,
    pub note: String,
    pub chara: String,
}

/// 目的：登记处视图 → 会话里的 agent 实例（`transient=false`）。**唯一**的转换口径：
///   点名建工作与「无参 = 全部」两条路都经这里，不在调用方各写一份同形转换（R15）。
pub fn agent_meta(v: &AgentView) -> AgentMeta {
    AgentMeta::normalized(
        v.name.clone(),
        v.modules.clone(),
        v.model.clone(),
        v.chara.clone(),
        false,
    )
}

/// 目的：保存 agent 的入参（用户给的定义）。
#[derive(Debug, Clone)]
pub struct AgentDraft {
    pub name: String,
    pub modules: Vec<String>,
    pub model: String,
    pub note: String,
    pub chara: String,
}

/// 目的：agent 名即将成为（session 内）目录名：非空、不含路径分隔符与 Windows 非法字符、不是保留名。
///   规则本身是内核的名字契约（`kernel::api::name::validate_dir_name`），这里只补主语。
pub fn validate_name(name: &str) -> Result<(), String> {
    crate::kernel::api::name::validate_dir_name(name, "agent 名")
}

/// 目的：目录保留名校验：这些名字由工作区布局固定占用（`session/<工作>/work`、`session/<工作>/children`），
///    agent 实例名不得占用。名单来自 `systools/names.yaml`，不硬编码在代码里。
///    大小写不敏感：Windows 的文件系统本来就不区分。
pub fn check_reserved(name: &str, reserved: &[String]) -> Result<(), String> {
    if reserved.iter().any(|r| r.eq_ignore_ascii_case(name)) {
        return Err(format!(
            "agent 名是目录保留名（{}）：换一个名字（它会与工作区固定目录撞路径）",
            name
        ));
    }
    Ok(())
}

/// 人设的一行预览（拟名单时给模型看）：取第一段非空文字并压成一行，过长截断。
/// 为什么截断：清单是给核心挑人用的，不是全文朗读；完整人设在 catalog_agents 与会话快照里。
fn chara_preview(chara: &str) -> String {
    let line = chara
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let mut out: String = line.chars().take(80).collect();
    if line.chars().count() > 80 {
        out.push('…');
    }
    out
}

/// 目的：已存 agent 清单（拟名单时给模型看）：名字 / 模块 / 模型 / 人设 / 说明；空登记处如实说明。
pub fn listing(known: &Agents) -> String {
    if known.is_empty() {
        return "（登记处还没有 agent；这次只能组装新的）".to_string();
    }
    known
        .iter()
        .map(|(name, a)| {
            let model = a
                .model
                .clone()
                .unwrap_or_else(|| "（核心默认）".to_string());
            let note = if a.note.trim().is_empty() {
                String::new()
            } else {
                format!("；{}", a.note.trim())
            };
            let chara = chara_preview(&a.chara);
            let chara = if chara.is_empty() {
                String::new()
            } else {
                format!("；人设：{}", chara)
            };
            format!(
                "- {}：模块 {}；模型 {}{}{}",
                name,
                a.modules.join(" + "),
                model,
                chara,
                note
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

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

/// 目的：核心拟名单时的一条：复用已存 agent，或组装一个新的（JSON 里二选一；Reuse 在前）。
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
        chara: String,
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
/// 目的：登记处全部 agent 的视图（空登记处不是错误——调用方据此给引导）。
pub fn views(agents: &Agents) -> Vec<AgentView> {
    agents
        .iter()
        .map(|(name, a)| AgentView {
            name: name.clone(),
            modules: a.modules.clone(),
            model: a.model.clone(),
            note: a.note.clone(),
            chara: a.chara.clone(),
        })
        .collect()
}

/// 目的：按名字取 agent 视图：**不猜、不代选**——名字不在登记处就如实报错，并列出现有的。
///    归属：点名是**登记处的查询**（它拥有 agent 身份），不是前端的便利函数。
pub fn pick(agents: &Agents, names: &[String]) -> Result<Vec<AgentView>, String> {
    let all = views(agents);
    if names.is_empty() {
        return Err("没有点名任何 agent".to_string());
    }
    let mut out = Vec::new();
    for n in names {
        match all.iter().find(|a| &a.name == n) {
            Some(a) => out.push(a.clone()),
            None => {
                return Err(format!(
                    "无此 agent：{}（现有：{}）",
                    n,
                    all.iter()
                        .map(|a| a.name.as_str())
                        .collect::<Vec<_>>()
                        .join(" · ")
                ));
            }
        }
    }
    Ok(out)
}
