//! 目的：agent 的存档形态——登记处里具名能力组合的纯值模型。
//! 管：Agent / AgentView / AgentMeta / AgentDraft 与名单 DTO；名字校验与保留名检查；名录视图与按名点名；列表文案。
//! 不管：落盘（在 catalog 的 detail/yaml_settings.rs）；名单落地与唯一实例命名（在 catalog 的 domain/picks.rs）；登记处用例（在 catalog 的 service.rs）。
//! 联动：会话名单、登记处、runtime 与呈现层都经 registry_model::api 取这些形状。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 目的：agent 没声明能力组时给的全集（读面、工作面、执行、共享区写入）。
/// 约束：能力组名与 `session_model` 的来源口径一致；声明了空列表就是什么都不给。
pub const DEFAULT_ABILITIES: [&str; 4] = ["fs-read", "fs-write", "exec", "shared-write"];

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
    /// 目的：能力组（fs-read / fs-write / exec / shared-write）；`None` = 没声明，按全集给。
    /// 约束：空列表是有意义的声明（什么都不给），不能归一成 `None`。
    #[serde(default)]
    pub abilities: Option<Vec<String>>,
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
            abilities: None,
        }
    }

    /// 目的：这个 agent 实际拿到的能力组（没声明就给全集）。
    pub fn abilities(&self) -> Vec<String> {
        self.abilities
            .clone()
            .unwrap_or_else(|| DEFAULT_ABILITIES.iter().map(|s| s.to_string()).collect())
    }

    /// 目的：声明能力组（建名单时从登记处带过来；只做去空白与去重，空列表保留）。
    pub fn with_abilities(mut self, abilities: Option<Vec<String>>) -> AgentMeta {
        self.abilities = abilities.map(|list| {
            let mut out: Vec<String> = Vec::with_capacity(list.len());
            for a in list {
                let id = a.trim().to_string();
                if !id.is_empty() && !out.contains(&id) {
                    out.push(id);
                }
            }
            out
        });
        self
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
    /// 目的：能力组（fs-read / fs-write / exec / shared-write）；`None` = 没声明，按全集给。
    #[serde(default)]
    pub abilities: Option<Vec<String>>,
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
    pub abilities: Option<Vec<String>>,
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
    .with_abilities(v.abilities.clone())
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
    pub fn why(&self) -> String {
        match self {
            RosterPick::Reuse { why, .. } | RosterPick::Build { why, .. } => why.clone(),
        }
    }
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
            abilities: a.abilities.clone(),
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
