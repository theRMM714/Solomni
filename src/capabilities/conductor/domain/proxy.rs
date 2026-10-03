//! 核心代理（core_proxy）工具的纯逻辑：入参解析、授权校验、载荷规整与回执拼装。
//!
//! 它不认识会话、不碰任何端口：外部动作一律经 conductor::ports::ProxyHost（见 service/proxy.rs）。
//! 真实会话宿主尚未落地（见 src/capabilities/conductor/testgaps.yaml）。
#![allow(dead_code)] // 见 docs/testing/quality-isolation.md §三：契约已冻结，生产调用点在下一步

use serde::{Deserialize, Serialize};

// ---------- 工具名（与 systools/tools.yaml 同一份名单） ----------

pub const CATALOG: &str = "catalog_agents";
pub const CREATE: &str = "create_session";
pub const SEND: &str = "send_session_message";
pub const OBSERVE: &str = "observe_session";
pub const CONTROL: &str = "control_session";
/// 消息查询：按“最新→更早”倒查子会话的消息（不把整份转录推给代理）。
pub const MESSAGES: &str = "read_session_messages";

/// 这六个 id 就是“代理工具面”（角色表 core_proxy 引用它们）。
pub fn is_proxy_tool(name: &str) -> bool {
    name == CATALOG
        || name == CREATE
        || name == SEND
        || name == OBSERVE
        || name == CONTROL
        || name == MESSAGES
}

// ---------- 调用上下文（由机制提供，不从模型参数取） ----------

/// 一条消息 / 一次动作的真实来源：由机制验证或填充，不能只信任模型传入的字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// 核心代用户做的（核心转达一律如此标记）。
    CoreProxy,
    /// 用户原话（保留独立来源引用，不伪装成核心生成的内容）。
    UserOriginal,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::CoreProxy => "core_proxy",
            Source::UserOriginal => "user_original",
        }
    }
}

/// 任务级授权（真实用户授予，机制注入）：本轮只做**全权**——`tools` 用角色表发放的整套
/// 代理工具面构造，`models`/`sessions` 留空 = 不限制，无有效期。
/// 核心不得自行授予或扩大；`tools` 仍是硬条件（不在里面 = 越范围）。
/// 细粒度范围、期限与**撤销**属独立的“会话权限状态”能力，本轮不做
/// （见 src/capabilities/conductor/testgaps.yaml）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Grant {
    /// 允许调用的代理工具 id。
    pub tools: Vec<String>,
    /// 允许使用的模型（空 = 不限制）。
    pub models: Vec<String>,
    /// 允许操作的会话（空 = 不限制）。
    pub sessions: Vec<String>,
    /// 到期时间（Unix 秒；None = 不过期）。
    pub expires_at: Option<i64>,
}

/// 一次代理调用的上下文：调用者身份、授权、父会话与当前时间。
#[derive(Debug, Clone)]
pub struct ProxyCall {
    pub source: Source,
    pub grant: Option<Grant>,
    pub parent: Option<String>,
    pub now: i64,
}

/// 授权校验（纯逻辑）：没有授权、越范围、超期、模型或会话不在授权内，都如实拒绝。
pub fn authorize(
    call: &ProxyCall,
    tool: &str,
    model: Option<&str>,
    session: Option<&str>,
) -> Result<(), String> {
    let Some(grant) = call.grant.as_ref() else {
        return Err("没有代理授权：核心不能自行授予权限，需要真实用户先授予任务级授权".to_string());
    };
    if let Some(at) = grant.expires_at {
        if call.now > at {
            return Err(format!("代理授权已过期（有效期到 {}）", at));
        }
    }
    if !grant.tools.iter().any(|t| t == tool) {
        return Err(format!("代理授权不包含这个工具：{}（越出授权范围）", tool));
    }
    if !grant.models.is_empty() {
        if let Some(m) = model {
            if !grant.models.iter().any(|x| x == m) {
                return Err(format!("代理授权不覆盖这个模型：{}（越出授权范围）", m));
            }
        }
    }
    if !grant.sessions.is_empty() {
        if let Some(s) = session {
            if !grant.sessions.iter().any(|x| x == s) {
                return Err(format!("代理授权不覆盖这个会话：{}（越出授权范围）", s));
            }
        }
    }
    Ok(())
}

// ---------- 枚举 ----------

/// catalog_agents 的 scope。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogScope {
    Agents,
    Modules,
    Models,
    All,
}

impl CatalogScope {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "agents" => Ok(Self::Agents),
            "modules" => Ok(Self::Modules),
            "models" => Ok(Self::Models),
            "all" => Ok(Self::All),
            other => Err(format!(
                "scope 只能是 agents / modules / models / all：{}",
                other
            )),
        }
    }
    pub fn covers_agents(self) -> bool {
        matches!(self, Self::Agents | Self::All)
    }
    pub fn covers_modules(self) -> bool {
        matches!(self, Self::Modules | Self::All)
    }
    pub fn covers_models(self) -> bool {
        matches!(self, Self::Models | Self::All)
    }
}

/// create_session 的形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionMode {
    Single,
    Multi,
}

impl SessionMode {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "single" => Ok(Self::Single),
            "multi" => Ok(Self::Multi),
            other => Err(format!("mode 只能是 single 或 multi：{}", other)),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Multi => "multi",
        }
    }
}

/// send_session_message 的消息种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    Task,
    UserReply,
    FollowUp,
    Review,
    Rework,
}

impl MessageKind {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "task" => Ok(Self::Task),
            "user_reply" => Ok(Self::UserReply),
            "follow_up" => Ok(Self::FollowUp),
            "review" => Ok(Self::Review),
            "rework" => Ok(Self::Rework),
            other => Err(format!(
                "kind 只能是 task / user_reply / follow_up / review / rework：{}",
                other
            )),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::UserReply => "user_reply",
            Self::FollowUp => "follow_up",
            Self::Review => "review",
            Self::Rework => "rework",
        }
    }
}

/// observe_session 要看的那一面。**没有“最新消息”这一面**：正文一律经
/// read_session_messages 主动倒查（子会话不把整份转录推给代理）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObserveView {
    Status,
    Pending,
    Artifacts,
    Full,
}

impl ObserveView {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "status" => Ok(Self::Status),
            "pending" => Ok(Self::Pending),
            "artifacts" => Ok(Self::Artifacts),
            "full" => Ok(Self::Full),
            other => Err(format!(
                "view 只能是 status / pending / artifacts / full：{}",
                other
            )),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Pending => "pending",
            Self::Artifacts => "artifacts",
            Self::Full => "full",
        }
    }
}

/// control_session 的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlAction {
    Pause,
    Resume,
    Stop,
    Close,
}

impl ControlAction {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "pause" => Ok(Self::Pause),
            "resume" => Ok(Self::Resume),
            "stop" => Ok(Self::Stop),
            "close" => Ok(Self::Close),
            other => Err(format!(
                "action 只能是 pause / resume / stop / close：{}",
                other
            )),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Stop => "stop",
            Self::Close => "close",
        }
    }
}

// ---------- 事实与回执（宿主产出 / 工具回给模型） ----------

/// 只读清单事实：只含公开信息（无密钥、无真实私有路径、无私有工作区内容）。
#[derive(Debug, Clone, Default, Serialize)]
pub struct Catalog {
    pub agents: Vec<AgentFact>,
    pub modules: Vec<ModuleFact>,
    pub models: Vec<ModelFact>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentFact {
    pub name: String,
    pub modules: Vec<String>,
    pub model: Option<String>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleFact {
    pub id: String,
    pub tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelFact {
    pub id: String,
    pub name: String,
    pub tools: String,
}

/// 建立一个会话后的稳定引用。
#[derive(Debug, Clone, Serialize)]
pub struct Created {
    pub session: String,
    pub agents: Vec<String>,
}

/// 一次观察的规范化视图 + 下一游标（`full` 之外的字段可选）。
/// **不回消息正文**：正文一律经 `read_session_messages` 主动倒查。
#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub session: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<Vec<String>>,
    /// 该会话的消息总条数（`read_session_messages` 倒查的上界）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// `read_session_messages` 回给代理的**一页消息**：`from` 是“距最新多少条”（0 = 最新一条），
/// 消息按**新→旧**排列；`next` 是继续往更早翻的下一 `from`（到底了为 None）。
#[derive(Debug, Clone, Serialize)]
pub struct MessagesPage {
    pub session: String,
    pub messages: Vec<MessageLine>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<usize>,
}

/// 一条消息（转录行的对外形态）：稳定行 id + 结构化身份 + 正文。
#[derive(Debug, Clone, Serialize)]
pub struct MessageLine {
    pub id: u64,
    pub speaker: String,
    pub verb: String,
    pub kind: String,
    pub text: String,
}

/// 一次控制之后会话的实际状态（不是“请求已提交”）。
#[derive(Debug, Clone, Serialize)]
pub struct ControlState {
    pub session: String,
    pub action: ControlAction,
    pub state: String,
}

/// 建会话的规整载荷（工具层解析完交给宿主；宿主不再解析模型参数）。
#[derive(Debug, Clone, PartialEq)]
pub struct NewSession {
    pub mode: SessionMode,
    pub agents: Vec<NewAgent>,
    pub workspace: Option<String>,
    pub request_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewAgent {
    pub name: String,
    /// 临时 agent（不在登记处）：随本次会话落档，不写回 agents.yaml。
    pub transient: bool,
    pub modules: Vec<String>,
    pub model: Option<String>,
    pub objective: String,
}

/// 一条要转达的消息（元信息 + 正文）：正文由工具层从参数取，宿主据此真正投递。
#[derive(Debug, Clone, PartialEq)]
pub struct Relayed {
    pub kind: MessageKind,
    pub source: Source,
    pub source_ref: Option<String>,
    pub parent: Option<String>,
    pub text: String,
}

// ---------- 入参（形状由 systools/tools.yaml 声明驱动；这里做语义校验） ----------

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogArgs {
    pub scope: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateArgs {
    pub mode: String,
    /// 每项是对象，形状在 parse_agents 里逐条校验（数组元素形状声明层表达不了）。
    pub agents: Vec<serde_json::Value>,
    #[serde(default)]
    pub workspace: Option<String>,
    pub request_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SendArgs {
    pub targets: Vec<String>,
    pub message: String,
    pub kind: String,
    #[serde(default)]
    pub source_ref: Option<String>,
    pub request_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ObserveArgs {
    pub session_id: String,
    pub view: String,
    #[serde(default)]
    pub since: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ControlArgs {
    pub session_id: String,
    pub action: String,
    pub reason: String,
    pub request_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MessagesArgs {
    pub session_id: String,
    /// 0 = 最新一条；1 = 倒数第二条；省略 = 0。
    #[serde(default)]
    pub from: Option<i64>,
    /// 本次取几条（1..=50）；省略 = 10。
    #[serde(default)]
    pub count: Option<i64>,
}

/// 解析出来的一条 agent 声明（尚未与登记处事实核对）。
struct ParsedAgent {
    name: String,
    reuse: bool,
    modules: Vec<String>,
    model: Option<String>,
    objective: String,
}

// ---------- 语义校验 ----------

/// catalog_agents：scope 只认四个值。
pub fn catalog_scope(args: &CatalogArgs) -> Result<CatalogScope, String> {
    CatalogScope::parse(args.scope.trim())
}

/// create_session：把模型给的名字/模块/模型与登记处事实核对后规整成载荷。
/// 失败一律如实报错——调用方据此拒绝整次调用，不留半成品。
pub fn resolve_new_session(args: &CreateArgs, catalog: &Catalog) -> Result<NewSession, String> {
    let mode = SessionMode::parse(args.mode.trim())?;
    let request_id = args.request_id.trim().to_string();
    if request_id.is_empty() {
        return Err("request_id 不能为空（幂等标识）".to_string());
    }
    let agents = parse_agents(&args.agents)?;
    match mode {
        SessionMode::Single if agents.len() != 1 => {
            return Err("mode=single 只接受一个 agent".to_string())
        }
        SessionMode::Multi if agents.len() < 2 => {
            return Err("mode=multi 至少要两个 agent".to_string())
        }
        _ => {}
    }
    let mut used: Vec<String> = Vec::new();
    let mut resolved: Vec<NewAgent> = Vec::new();
    for (i, a) in agents.into_iter().enumerate() {
        let (name, modules, model, transient) = if a.reuse {
            let fact = catalog
                .agents
                .iter()
                .find(|f| f.name == a.name)
                .ok_or_else(|| format!("agents[{}]：agent {} 不在登记处", i, a.name))?;
            (
                fact.name.clone(),
                fact.modules.clone(),
                fact.model.clone(),
                false,
            )
        } else {
            if a.modules.is_empty() {
                return Err(format!("agents[{}]：agent {} 至少要有一个模块", i, a.name));
            }
            for id in &a.modules {
                if !catalog.modules.iter().any(|m| m.id == *id) {
                    return Err(format!("agents[{}]：无此模块 {}", i, id));
                }
            }
            if let Some(m) = &a.model {
                if !catalog.models.iter().any(|x| x.id == *m) {
                    return Err(format!("agents[{}]：无此模型 {}", i, m));
                }
            }
            (a.name.clone(), a.modules.clone(), a.model.clone(), true)
        };
        crate::capabilities::registry::api::validate_name(&name)
            .map_err(|e| format!("agents[{}] 的名字不合法：{}", i, e))?;
        for id in &modules {
            if used.iter().any(|u| u == id) {
                return Err(format!(
                    "模块 {} 被多个 agent 同时使用；同一模块只能属于一个 agent",
                    id
                ));
            }
            used.push(id.clone());
        }
        resolved.push(NewAgent {
            name,
            transient,
            modules,
            model,
            objective: a.objective.clone(),
        });
    }
    let workspace = args
        .workspace
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    Ok(NewSession {
        mode,
        agents: resolved,
        workspace,
        request_id,
    })
}

/// send_session_message：目标、种类与来源引用的语义校验；多目标在这里规整成一张表。
pub fn relay(args: &SendArgs, call: &ProxyCall) -> Result<(Vec<String>, Relayed), String> {
    let targets: Vec<String> = args
        .targets
        .iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    if targets.is_empty() {
        return Err("targets 不能为空".to_string());
    }
    if args.message.trim().is_empty() {
        return Err("message 不能为空".to_string());
    }
    if args.request_id.trim().is_empty() {
        return Err("request_id 不能为空（幂等标识）".to_string());
    }
    let kind = MessageKind::parse(args.kind.trim())?;
    let source_ref = args
        .source_ref
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if kind == MessageKind::UserReply && source_ref.is_none() {
        return Err("kind=user_reply 必须给 source_ref：核心不能把自己的话当用户原文".to_string());
    }
    Ok((
        targets,
        Relayed {
            kind,
            source: call.source,
            source_ref,
            parent: call.parent.clone(),
            text: args.message.trim().to_string(),
        },
    ))
}

/// observe_session：会话 id、view 与游标的语义校验。
pub fn observe_request(
    args: &ObserveArgs,
) -> Result<(String, ObserveView, Option<String>), String> {
    let session = args.session_id.trim().to_string();
    if session.is_empty() {
        return Err("session_id 不能为空".to_string());
    }
    let view = ObserveView::parse(args.view.trim())?;
    let since = args
        .since
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    Ok((session, view, since))
}

/// control_session：会话 id、action、reason 与幂等标识的语义校验。
pub fn control_request(
    args: &ControlArgs,
) -> Result<(String, ControlAction, String, String), String> {
    let session = args.session_id.trim().to_string();
    if session.is_empty() {
        return Err("session_id 不能为空".to_string());
    }
    let action = ControlAction::parse(args.action.trim())?;
    let reason = args.reason.trim().to_string();
    if reason.is_empty() {
        return Err("reason 不能为空（它会进可回放记录）".to_string());
    }
    let request_id = args.request_id.trim().to_string();
    if request_id.is_empty() {
        return Err("request_id 不能为空（幂等标识）".to_string());
    }
    Ok((session, action, reason, request_id))
}

/// read_session_messages：会话 id + 倒查起点/条数的语义校验（`from` 以最新为 0）。
pub fn messages_request(args: &MessagesArgs) -> Result<(String, usize, usize), String> {
    let session = args.session_id.trim().to_string();
    if session.is_empty() {
        return Err("session_id 不能为空".to_string());
    }
    let from = args.from.unwrap_or(0);
    if from < 0 {
        return Err("from 不能为负（0 = 最新一条）".to_string());
    }
    let count = args.count.unwrap_or(10);
    if !(1..=50).contains(&count) {
        return Err("count 只能是 1 到 50".to_string());
    }
    Ok((session, from as usize, count as usize))
}

/// 清单回执：只回请求的 scope（不给无关字段，也就不会顺手泄漏）。
pub fn render_catalog(scope: CatalogScope, c: &Catalog) -> Result<String, String> {
    let mut out = serde_json::Map::new();
    if scope.covers_agents() {
        out.insert(
            "agents".to_string(),
            serde_json::to_value(&c.agents).map_err(|e| e.to_string())?,
        );
    }
    if scope.covers_modules() {
        out.insert(
            "modules".to_string(),
            serde_json::to_value(&c.modules).map_err(|e| e.to_string())?,
        );
    }
    if scope.covers_models() {
        out.insert(
            "models".to_string(),
            serde_json::to_value(&c.models).map_err(|e| e.to_string())?,
        );
    }
    Ok(serde_json::Value::Object(out).to_string())
}

fn parse_agents(items: &[serde_json::Value]) -> Result<Vec<ParsedAgent>, String> {
    if items.is_empty() {
        return Err("agents 不能为空".to_string());
    }
    let mut out = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let obj = item
            .as_object()
            .ok_or_else(|| format!("agents[{}] 必须是对象", i))?;
        let objective = match obj.get("objective").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s.trim().to_string(),
            _ => return Err(format!("agents[{}] 缺少 objective（写给它的任务）", i)),
        };
        if let Some(rv) = obj.get("ref") {
            let reference = rv
                .as_str()
                .ok_or_else(|| format!("agents[{}].ref 必须是字符串", i))?
                .trim();
            if reference.is_empty() {
                return Err(format!("agents[{}].ref 不能为空", i));
            }
            reject_extra(obj, &["ref", "objective"], i)?;
            out.push(ParsedAgent {
                name: reference.to_string(),
                reuse: true,
                modules: Vec::new(),
                model: None,
                objective,
            });
        } else {
            let name = match obj.get("name").and_then(|v| v.as_str()) {
                Some(s) if !s.trim().is_empty() => s.trim().to_string(),
                _ => return Err(format!("agents[{}] 缺少 name（新 agent 的名字）", i)),
            };
            let modules = match obj.get("modules").and_then(|v| v.as_array()) {
                Some(a) => {
                    let mut ids = Vec::new();
                    for m in a {
                        let id = m
                            .as_str()
                            .ok_or_else(|| format!("agents[{}].modules 里有非字符串项", i))?
                            .trim();
                        if id.is_empty() {
                            return Err(format!("agents[{}].modules 里有空模块 id", i));
                        }
                        ids.push(id.to_string());
                    }
                    ids
                }
                None => return Err(format!("agents[{}] 缺少 modules 数组", i)),
            };
            let model = obj
                .get("model")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            reject_extra(obj, &["name", "modules", "model", "objective"], i)?;
            out.push(ParsedAgent {
                name,
                reuse: false,
                modules,
                model,
                objective,
            });
        }
    }
    Ok(out)
}

fn reject_extra(
    obj: &serde_json::Map<String, serde_json::Value>,
    allowed: &[&str],
    i: usize,
) -> Result<(), String> {
    for k in obj.keys() {
        if !allowed.contains(&k.as_str()) {
            return Err(format!("agents[{}] 里有不认识的键：{}", i, k));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Catalog {
        Catalog {
            agents: vec![AgentFact {
                name: "a".to_string(),
                modules: vec!["m1".to_string()],
                model: Some("gpt".to_string()),
                note: String::new(),
            }],
            modules: vec![
                ModuleFact {
                    id: "m1".to_string(),
                    tools: vec!["t".to_string()],
                },
                ModuleFact {
                    id: "m2".to_string(),
                    tools: Vec::new(),
                },
            ],
            models: vec![ModelFact {
                id: "gpt".to_string(),
                name: "GPT".to_string(),
                tools: "native".to_string(),
            }],
        }
    }

    /// 名字/模块/模型与登记处事实核对：不符就整条拒绝。
    #[test]
    fn resolve_new_session_checks_against_the_catalog() {
        let c = facts();
        let ok: CreateArgs = serde_json::from_value(serde_json::json!({
            "mode": "single",
            "agents": [{"name": "a", "modules": ["m1"], "model": "gpt", "objective": "做事"}],
            "request_id": "r1"
        }))
        .unwrap();
        let spec = resolve_new_session(&ok, &c).expect("合法声明");
        assert_eq!(spec.mode, SessionMode::Single);
        assert_eq!(spec.agents[0].modules, vec!["m1".to_string()]);
        assert!(spec.agents[0].transient);

        let unknown_module: CreateArgs = serde_json::from_value(serde_json::json!({
            "mode": "single",
            "agents": [{"name": "a", "modules": ["nope"], "objective": "做事"}],
            "request_id": "r2"
        }))
        .unwrap();
        assert!(resolve_new_session(&unknown_module, &c)
            .unwrap_err()
            .contains("无此模块"));

        let dup: CreateArgs = serde_json::from_value(serde_json::json!({
            "mode": "multi",
            "agents": [
                {"name": "a", "modules": ["m1"], "objective": "一"},
                {"name": "b", "modules": ["m1"], "objective": "二"}
            ],
            "request_id": "r3"
        }))
        .unwrap();
        assert!(resolve_new_session(&dup, &c)
            .unwrap_err()
            .contains("同一模块只能属于一个 agent"));

        let one: CreateArgs = serde_json::from_value(serde_json::json!({
            "mode": "multi",
            "agents": [{"name": "a", "modules": ["m1"], "objective": "一"}],
            "request_id": "r4"
        }))
        .unwrap();
        assert!(resolve_new_session(&one, &c)
            .unwrap_err()
            .contains("至少要两个 agent"));

        // 复用登记处的 agent：模块与模型取它自己的，核心不代拟。
        let reuse: CreateArgs = serde_json::from_value(serde_json::json!({
            "mode": "single",
            "agents": [{"ref": "a", "objective": "做事"}],
            "request_id": "r5"
        }))
        .unwrap();
        let spec = resolve_new_session(&reuse, &c).expect("复用项");
        assert!(!spec.agents[0].transient);
        assert_eq!(spec.agents[0].model.as_deref(), Some("gpt"));

        let missing: CreateArgs = serde_json::from_value(serde_json::json!({
            "mode": "single",
            "agents": [{"ref": "nope", "objective": "做事"}],
            "request_id": "r6"
        }))
        .unwrap();
        assert!(resolve_new_session(&missing, &c)
            .unwrap_err()
            .contains("不在登记处"));
    }

    /// 授权是硬条件：没授权 / 越范围 / 超期 / 模型或会话不在授权内都拒绝。
    #[test]
    fn authorization_is_checked_before_anything_else() {
        let base = ProxyCall {
            source: Source::CoreProxy,
            grant: None,
            parent: Some("main".to_string()),
            now: 100,
        };
        assert!(authorize(&base, CREATE, None, None)
            .unwrap_err()
            .contains("没有代理授权"));

        let call = ProxyCall {
            grant: Some(Grant {
                tools: vec![CATALOG.to_string()],
                ..Default::default()
            }),
            ..base.clone()
        };
        assert!(authorize(&call, CREATE, None, None)
            .unwrap_err()
            .contains("不包含这个工具"));
        assert!(authorize(&call, CATALOG, None, None).is_ok());

        let expired = ProxyCall {
            grant: Some(Grant {
                tools: vec![CATALOG.to_string()],
                expires_at: Some(50),
                ..Default::default()
            }),
            ..base.clone()
        };
        assert!(authorize(&expired, CATALOG, None, None)
            .unwrap_err()
            .contains("已过期"));

        let scoped = ProxyCall {
            grant: Some(Grant {
                tools: vec![CREATE.to_string()],
                models: vec!["gpt".to_string()],
                sessions: vec!["s1".to_string()],
                ..Default::default()
            }),
            ..base.clone()
        };
        assert!(authorize(&scoped, CREATE, Some("other"), None)
            .unwrap_err()
            .contains("不覆盖这个模型"));
        assert!(authorize(&scoped, CREATE, Some("gpt"), Some("s9"))
            .unwrap_err()
            .contains("不覆盖这个会话"));
        assert!(authorize(&scoped, CREATE, Some("gpt"), Some("s1")).is_ok());
    }

    /// 代答必须带独立来源引用：不把核心生成的内容伪装成用户原文。
    #[test]
    fn user_reply_requires_a_source_ref() {
        let call = ProxyCall {
            source: Source::CoreProxy,
            grant: Some(Grant {
                tools: vec![SEND.to_string()],
                ..Default::default()
            }),
            parent: None,
            now: 0,
        };
        let bad: SendArgs = serde_json::from_value(serde_json::json!({
            "targets": ["t1"], "message": "好", "kind": "user_reply", "request_id": "x"
        }))
        .unwrap();
        assert!(relay(&bad, &call)
            .unwrap_err()
            .contains("必须给 source_ref"));

        let good: SendArgs = serde_json::from_value(serde_json::json!({
            "targets": ["t1", "t1"], "message": "好", "kind": "user_reply",
            "source_ref": "用户第 3 句", "request_id": "x"
        }))
        .unwrap();
        let (targets, relayed) = relay(&good, &call).unwrap();
        assert_eq!(targets.len(), 2);
        assert_eq!(relayed.source, Source::CoreProxy);
        assert_eq!(relayed.source_ref.as_deref(), Some("用户第 3 句"));
    }

    /// 清单回执只回请求的 scope。
    #[test]
    fn catalog_renders_only_the_requested_scope() {
        let c = facts();
        let only = render_catalog(CatalogScope::Agents, &c).unwrap();
        assert!(only.contains("agents"));
        assert!(!only.contains("models"));
        let all = render_catalog(CatalogScope::All, &c).unwrap();
        assert!(all.contains("modules"));
        assert!(all.contains("models"));
    }

    /// 消息倒查的边界：`from` 以最新为 0、`count` 有上下界、缺省是 0/10。
    #[test]
    fn messages_request_checks_from_and_count() {
        let ok: MessagesArgs = serde_json::from_value(serde_json::json!({
            "session_id": "s1", "from": 2, "count": 5
        }))
        .unwrap();
        assert_eq!(messages_request(&ok).unwrap(), ("s1".to_string(), 2, 5));

        let default: MessagesArgs =
            serde_json::from_value(serde_json::json!({ "session_id": "s1" })).unwrap();
        assert_eq!(
            messages_request(&default).unwrap(),
            ("s1".to_string(), 0, 10)
        );

        let bad: MessagesArgs = serde_json::from_value(serde_json::json!({
            "session_id": "s1", "from": -1
        }))
        .unwrap();
        assert!(messages_request(&bad).unwrap_err().contains("from"));

        let big: MessagesArgs = serde_json::from_value(serde_json::json!({
            "session_id": "s1", "count": 51
        }))
        .unwrap();
        assert!(messages_request(&big).unwrap_err().contains("count"));
    }
}
