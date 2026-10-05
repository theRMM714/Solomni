//! 核心代理（core_proxy）工具的纯逻辑：入参解析、授权校验、载荷规整与回执拼装。
//!
//! 它不认识会话、不碰任何端口：外部动作一律经 conductor::ports::ProxyHost（见 service/proxy.rs）。

use serde::{Deserialize, Serialize};

// ---------- 工具名（与 systools/tools.yaml 同一份名单） ----------

pub const CATALOG: &str = "catalog_agents";
pub const CREATE: &str = "create_session";
pub const SEND: &str = "send_session_message";
pub const OBSERVE: &str = "observe_session";
pub const CONTROL: &str = "control_session";
/// 消息查询：按“最新→更早”倒查子会话的消息（不把整份转录推给代理）。
pub const MESSAGES: &str = "read_session_messages";

/// 代理会话里核心的说话人名（只此一处，重建与实时同源）。
pub const SPEAKER: &str = "核心";

/// 这六个 id 就是“代理工具面”（角色表 core_proxy 引用它们）。
pub fn is_proxy_tool(name: &str) -> bool {
    name == CATALOG
        || name == CREATE
        || name == SEND
        || name == OBSERVE
        || name == CONTROL
        || name == MESSAGES
}

/// 任务级授权（真实用户授予，机制注入）：本轮只做**全权**——`tools` 用角色表发放的整套
/// 代理工具面构造，`models`/`sessions` 留空 = 不限制，无有效期。
/// 核心不得自行授予或扩大；`tools` 仍是硬条件（不在里面 = 越范围）。
/// 细粒度范围、期限与**撤销**属独立的“会话权限状态”能力，本轮不做
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

/// 一次代理调用的上下文：授权、父会话与当前时间。
#[derive(Debug, Clone)]
pub struct ProxyCall {
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
}

/// control_session 的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlAction {
    Stop,
    Continue,
    Close,
}

impl ControlAction {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "stop" => Ok(Self::Stop),
            "continue" => Ok(Self::Continue),
            "close" => Ok(Self::Close),
            other => Err(format!("action 只能是 stop / continue / close：{}", other)),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Continue => "continue",
            Self::Close => "close",
        }
    }
}

/// 协作子会话的**落门方式**：代理转达一条消息时，按目标"此刻等的是哪一关"决定送到哪里。
/// 与前端读 `pending.kind` 再选动作是同一口径（`Pending::decision_parts`）——
/// 别处不许再按消息种类猜门（猜错就会把"开工"按到"请教"上）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateRoute {
    /// 二选一的关（代拟名单）：短步骤，不跑泵。
    ConfirmSlate,
    /// 二选一的关（开始讨论）：跑泵。
    Begin,
    /// 其余门（请教 / 方案待审 / 节点没过）：自由文本，由协作自己的核心 AI 判明确性。
    Decide,
    /// 没在等门（停在中途 / 已收敛）：把它推着接着走，而不是塞一句"没有等你定的事"。
    Resume,
}

/// 从"等的是哪一关"（`Pending::decision_parts().0`；None = 没在等门）定落门方式。
pub fn gate_route(pending: Option<&str>) -> GateRoute {
    match pending {
        Some("confirm_slate") => GateRoute::ConfirmSlate,
        Some("confirm_begin") => GateRoute::Begin,
        Some(_) => GateRoute::Decide,
        None => GateRoute::Resume,
    }
}

/// 一次控制动作写进目标会话的**可回放记录**文案（机制写，前端照同一条显示）。
/// 记的是"谁、为什么动了这条会话"——不静默改运行态。
pub fn control_note(action: ControlAction, reason: &str) -> String {
    format!("[代理] {} 这条会话：{}", action.as_str(), reason)
}

/// 一次转达写进目标会话的**来源记录**文案：说清接下来这条**不是用户原话**，
/// 而是核心代理转达的——这条记录是它的可回放凭据。
pub fn relay_note(kind: MessageKind) -> String {
    format!("[代理] 接下来这条由核心代理转达（{}）", kind.as_str())
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
    /// 游标 = 消息条数的字符串形式；下次观察把它当 `since` 传回来即可。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// 自 `since` 以来的新增消息条数（给了 `since` 才有；解不出数字就当没给）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_messages: Option<usize>,
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
    /// **这个会话的开头**：single = 它的第一句（点火用的派发），multi = 本次需求。
    /// 建好就开始——不是"给某个 agent 的任务"（见 `ProxyBridge::create_session`）。
    pub opening: String,
    pub request_id: String,
    /// 父会话：由**机制**从调用上下文填，不从模型参数取（模型不能自选父）。
    pub parent: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewAgent {
    pub name: String,
    /// 临时 agent（不在登记处）：随本次会话落档，不写回 agents.yaml。
    pub transient: bool,
    pub modules: Vec<String>,
    pub model: Option<String>,
}

/// 一条要转达的消息（元信息 + 正文）：正文由工具层从参数取，宿主据此真正投递。
#[derive(Debug, Clone, PartialEq)]
pub struct Relayed {
    pub kind: MessageKind,
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
    pub opening: String,
    pub request_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SendArgs {
    pub targets: Vec<String>,
    pub message: String,
    pub kind: String,
    pub request_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ObserveArgs {
    pub session_id: String,
    pub view: String,
    /// 上次观察的游标（消息条数）；只用来算 `new_messages`，回执仍是幂等快照。
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
    let opening = args.opening.trim().to_string();
    if opening.is_empty() {
        return Err(
            "opening 不能为空（这个会话的开头：single 是它的第一句，multi 是本次需求）".to_string(),
        );
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
                    return Err(format!(
                        "agents[{}]：无此模块 {}（在册的模块 id：{}）",
                        i,
                        id,
                        catalog
                            .modules
                            .iter()
                            .map(|m| m.id.as_str())
                            .collect::<Vec<_>>()
                            .join("、")
                    ));
                }
            }
            if let Some(m) = &a.model {
                if !catalog.models.iter().any(|x| x.id == *m) {
                    return Err(format!(
                        "agents[{}]：无此模型 {}（在册的模型 id：{}）",
                        i,
                        m,
                        catalog
                            .models
                            .iter()
                            .map(|x| x.id.as_str())
                            .collect::<Vec<_>>()
                            .join("、")
                    ));
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
        });
    }
    Ok(NewSession {
        mode,
        agents: resolved,
        opening,
        request_id,
        parent: None,
    })
}

/// send_session_message：目标与种类的语义校验；多目标在这里规整成一张表。
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
    Ok((
        targets,
        Relayed {
            kind,
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
        if let Some(rv) = obj.get("ref") {
            let reference = rv
                .as_str()
                .ok_or_else(|| format!("agents[{}].ref 必须是字符串", i))?
                .trim();
            if reference.is_empty() {
                return Err(format!("agents[{}].ref 不能为空", i));
            }
            reject_extra(obj, &["ref"], i)?;
            out.push(ParsedAgent {
                name: reference.to_string(),
                reuse: true,
                modules: Vec::new(),
                model: None,
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
            reject_extra(obj, &["name", "modules", "model"], i)?;
            out.push(ParsedAgent {
                name,
                reuse: false,
                modules,
                model,
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

    /// 身份（名字 / 模块 / 模型）与登记处事实核对：不符就整条拒绝，并把**在册候选**列回给模型；
    /// 任务不在 agent 上——它是会话级的 opening。
    #[test]
    fn resolve_new_session_checks_against_the_catalog() {
        let c = facts();
        let ok: CreateArgs = serde_json::from_value(serde_json::json!({
            "mode": "single",
            "agents": [{"name": "a", "modules": ["m1"], "model": "gpt"}],
            "opening": "做事",
            "request_id": "r1"
        }))
        .unwrap();
        let spec = resolve_new_session(&ok, &c).expect("合法声明");
        assert_eq!(spec.mode, SessionMode::Single);
        assert_eq!(spec.agents[0].modules, vec!["m1".to_string()]);
        assert!(spec.agents[0].transient);
        assert_eq!(spec.opening, "做事");

        let unknown_module: CreateArgs = serde_json::from_value(serde_json::json!({
            "mode": "single",
            "agents": [{"name": "a", "modules": ["nope"]}],
            "opening": "做事",
            "request_id": "r2"
        }))
        .unwrap();
        let err = resolve_new_session(&unknown_module, &c).unwrap_err();
        assert!(err.contains("无此模块"), "{}", err);
        assert!(err.contains("m1"), "失败要把在册模块列回去：{}", err);

        // 模型名（display name）不是 id：如实拒绝，并列出在册 id。
        let model_name: CreateArgs = serde_json::from_value(serde_json::json!({
            "mode": "single",
            "agents": [{"name": "a", "modules": ["m1"], "model": "GPT"}],
            "opening": "做事",
            "request_id": "r2b"
        }))
        .unwrap();
        let err = resolve_new_session(&model_name, &c).unwrap_err();
        assert!(err.contains("无此模型"), "{}", err);
        assert!(err.contains("gpt"), "失败要把在册模型 id 列回去：{}", err);

        let empty_opening: CreateArgs = serde_json::from_value(serde_json::json!({
            "mode": "single",
            "agents": [{"name": "a", "modules": ["m1"]}],
            "opening": "   ",
            "request_id": "r2c"
        }))
        .unwrap();
        assert!(resolve_new_session(&empty_opening, &c)
            .unwrap_err()
            .contains("opening"));

        // 身份项不接受任务字段（任务是会话级的 opening）。
        let extra: CreateArgs = serde_json::from_value(serde_json::json!({
            "mode": "single",
            "agents": [{"ref": "a", "objective": "做事"}],
            "opening": "做事",
            "request_id": "r2d"
        }))
        .unwrap();
        assert!(resolve_new_session(&extra, &c)
            .unwrap_err()
            .contains("不认识的键"));

        let dup: CreateArgs = serde_json::from_value(serde_json::json!({
            "mode": "multi",
            "agents": [
                {"name": "a", "modules": ["m1"]},
                {"name": "b", "modules": ["m1"]}
            ],
            "opening": "一起做",
            "request_id": "r3"
        }))
        .unwrap();
        assert!(resolve_new_session(&dup, &c)
            .unwrap_err()
            .contains("同一模块只能属于一个 agent"));

        let one: CreateArgs = serde_json::from_value(serde_json::json!({
            "mode": "multi",
            "agents": [{"name": "a", "modules": ["m1"]}],
            "opening": "一起做",
            "request_id": "r4"
        }))
        .unwrap();
        assert!(resolve_new_session(&one, &c)
            .unwrap_err()
            .contains("至少要两个 agent"));

        // 复用登记处的 agent：模块与模型取它自己的，核心不代拟。
        let reuse: CreateArgs = serde_json::from_value(serde_json::json!({
            "mode": "single",
            "agents": [{"ref": "a"}],
            "opening": "做事",
            "request_id": "r5"
        }))
        .unwrap();
        let spec = resolve_new_session(&reuse, &c).expect("复用项");
        assert!(!spec.agents[0].transient);
        assert_eq!(spec.agents[0].model.as_deref(), Some("gpt"));

        let missing: CreateArgs = serde_json::from_value(serde_json::json!({
            "mode": "single",
            "agents": [{"ref": "nope"}],
            "opening": "做事",
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

    /// 转达不再要求"来源引用"：只按 kind 记一条"由核心代理转达"的记录。
    #[test]
    fn relay_only_needs_targets_and_a_kind() {
        let call = ProxyCall {
            grant: None,
            parent: Some("p".to_string()),
            now: 0,
        };
        let args: SendArgs = serde_json::from_value(serde_json::json!({
            "targets": ["t1", "t1"], "message": "好", "kind": "user_reply", "request_id": "x"
        }))
        .unwrap();
        let (targets, relayed) = relay(&args, &call).unwrap();
        assert_eq!(targets.len(), 2);
        assert_eq!(relayed.kind, MessageKind::UserReply);
        assert_eq!(relayed.text, "好");
        assert_eq!(relayed.parent.as_deref(), Some("p"));

        let bad: SendArgs = serde_json::from_value(serde_json::json!({
            "targets": [], "message": "好", "kind": "task", "request_id": "x"
        }))
        .unwrap();
        assert!(relay(&bad, &call).unwrap_err().contains("targets"));

        let bad_kind: SendArgs = serde_json::from_value(serde_json::json!({
            "targets": ["t1"], "message": "好", "kind": "nope", "request_id": "x"
        }))
        .unwrap();
        assert!(relay(&bad_kind, &call).unwrap_err().contains("kind"));
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

    /// 落门方式只看"它此刻等的是哪一关"：二选一走确认，其余门走裁决，没在等门就接着推进。
    #[test]
    fn gate_route_follows_the_pending_kind() {
        assert_eq!(gate_route(Some("confirm_slate")), GateRoute::ConfirmSlate);
        assert_eq!(gate_route(Some("confirm_begin")), GateRoute::Begin);
        for kind in ["ask", "plan_review", "node_blocked"] {
            assert_eq!(gate_route(Some(kind)), GateRoute::Decide, "{}", kind);
        }
        assert_eq!(gate_route(None), GateRoute::Resume);
    }

    /// 转达与控制都要留下"谁、为什么"的可回放记录。
    #[test]
    fn notes_record_who_really_said_it() {
        let note = relay_note(MessageKind::UserReply);
        assert!(
            note.contains("核心代理转达") && note.contains("user_reply"),
            "{}",
            note
        );
        let note = relay_note(MessageKind::Task);
        assert!(
            note.contains("核心代理转达") && note.contains("task"),
            "{}",
            note
        );
        let ctl = control_note(ControlAction::Stop, "先停一下");
        assert!(ctl.contains("stop") && ctl.contains("先停一下"), "{}", ctl);
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
