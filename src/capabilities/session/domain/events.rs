//! 呈现侧契约：事件与介入请求。词汇定义在本能力（`api` 导出），前端按此渲染。
//! 呈现即上下文：转录行与核心记录完全一致。

/// 目的：裁决卡的信封——谁在问。role = core（核心）/ member（某一席）/ tools（工具层）；name = 具体是谁。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DecisionEnvelope {
    pub role: String,
    pub name: String,
}

/// 目的：裁决卡的消息——发起方给的三段话（标题 / 正文 / 详情）。通道不解释内容。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DecisionMessage {
    pub title: String,
    pub body: String,
    /// 目的：详情（建议、原话、清单这类"展开看"的东西）；没有就是空串。
    pub detail: String,
}

/// 目的：一个选项——id 是行为契约（回答按 id 分派、改文案不改行为），label 只给渲染。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DecisionOption {
    pub id: String,
    pub label: String,
}

/// 目的：裁决卡——渲染层只认这四个字段（id + 信封 + 消息 + 选项集），不认业务含义。
///   id 会话内唯一且稳定：回答按它认卡，重启后由转录重建（见 docs/session/session-model.md）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DecisionCard {
    pub id: String,
    pub envelope: DecisionEnvelope,
    pub message: DecisionMessage,
    pub options: Vec<DecisionOption>,
}

impl DecisionCard {
    /// 目的：这张卡的选项集**有没有**这个 id（回答校验的唯一判据）。
    /// 约束：判据是选项集本身、不是文案——旧卡的答案因此放行不了新的请求。
    pub fn has_option(&self, id: &str) -> bool {
        self.options.iter().any(|o| o.id == id)
    }
}

/// 目的：队首之后还在等的一张卡——界面只看信封与标题（谁在等、问的什么）。
/// 约束：`id` / `gate` / `payload` 是**机制自己的重建材料**（与队首那张同一套），呈现层不看它们；
///   带上它们，整队才能只凭队首那条事件从转录重建（见 docs/session/session-model.md「请用户裁决」）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DecisionWaiter {
    /// 目的：这一张自己的卡号（会话内唯一、稳定）：它升为队首后按这个号回答。
    pub id: String,
    pub envelope: DecisionEnvelope,
    pub title: String,
    /// 目的：这一关的机制名（与队首那条同一口径）。
    pub gate: String,
    /// 目的：这一关的机制载荷（重建这一关用它；不含渲染内容）。
    pub payload: serde_json::Value,
}

/// 目的：当前的那一队裁决——队首卡（用户可见 / 可答的那张）+ 后面还在等的几张（先来后到）。
#[derive(Debug, Clone)]
pub struct DecisionQueue {
    pub card: DecisionCard,
    pub waiting: Vec<DecisionWaiter>,
}

/// 目的：一次回答的记录——谁答的、选了哪个 id、附言。落盘后重启仍能重建"答过什么"。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DecisionAnswer {
    /// 目的：回答的是哪张卡（防旧卡的答案被用来放行新的请求）。
    pub card: String,
    /// 目的：谁答的（人经呈现层：用户）。
    pub by: String,
    /// 目的：选中的选项 id。
    pub option: String,
    /// 目的：附言（用户自己的想法；没有就是空串）。
    #[serde(default)]
    pub note: String,
}

/// 会话事件：驱动前端渲染；转录行为增量，前端按序累积。
/// 预留字段说明：DiscussionDone 的 round/over_cap 供 Web 前端做裁决确认页（CLI 暂不渲染）。
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum SessionEvent {
    /// 状态提示（通道回落、建组、返工、上限等）。
    Notice(String),
    /// 转录新增行（带会话内稳定 id，回档按它定位）。
    Transcript(Vec<LineView>),
    /// 讨论收敛（over_cap = 轮次超限，需用户裁决）。
    DiscussionDone { round: usize, over_cap: bool },
    /// 整理方案就绪。
    Plan(String),
    /// 方案待审（审查关卡）：把方案交给用户，等他点「同意」才开工。
    /// 与 \`Plan\` 的区别：\`Plan\` 是"整理好了"这个事实，\`PlanReview\` 是"请用户裁决"这个请求。
    PlanReview {
        plan: String,
        /// 核心给出的任务链（审查关卡要连同链一起给用户看）。
        chain: crate::capabilities::taskchain::api::TaskChain,
    },
    /// 上下文压缩：`up_to` 之前的转录**不再发给模型**（转录本身完整保留、用户仍可查看），
    /// 由一份 summary 代替（见 docs/session/session-model.md 六）。
    Compacted { up_to: u64, summary: String },
    /// 任务链的一个节点开工：它跑在自己的**子会话**里（用户可进去干预）。
    NodeStarted {
        node: String,
        sid: String,
        assignee: String,
    },
    /// 成员执行回报（rework = 第几轮执行，0 为首轮）。
    Report {
        id: String,
        text: String,
        rework: usize,
    },
    /// 验收清单（raw = 解析失败时的原文）。
    Review { items: Vec<CheckView>, raw: String },
    /// 交付结论（over_rework = 返工超限交用户裁决）。
    Delivery { ok: bool, over_rework: bool },
    /// 会话结束。
    Ended,
    /// 一次工具调用（短暂，不落盘）：与 tool 转录行同源，供活动会话实时刷新。
    ToolCall(ToolCallView),
    /// 流式增量（短暂，不落盘）：按到达顺序的分段。
    /// kind = start / text / reasoning；start 表示新一轮开始（前端清空本轮占位）。
    Delta {
        speaker: String,
        kind: String,
        text: String,
    },
    /// **裁决卡**（推 + 落盘）：渲染层按 card 的四个字段画、按选项 id 回答。
    /// gate 与本关的载荷是机制自己的重建材料（重启后由转录重建挂起，见 docs/session/session-model.md）。
    DecisionCard {
        card: DecisionCard,
        /// 这一关的机制名（ask / confirm_slate / confirm_begin / plan_review / node_blocked）。
        gate: String,
        payload: serde_json::Value,
        /// 排在队首之后还在等的几张（**只有队首可答**）：界面据此显示"前面还排着几条"。
        waiting: Vec<DecisionWaiter>,
    },
    /// **整队作废**（推 + 落盘）：用户按停止（或会话关闭）时，队列里还没答的卡一律作废。
    /// 等待方按「停止 = 拒绝」解开（不是放行、也不是永远挂着）；已答过的不受影响。
    DecisionVoid { cards: Vec<String>, reason: String },
    /// **一次裁决回答**（推 + 落盘）：谁答的、选了哪个 id、附言。
    DecisionAnswer(DecisionAnswer),
    /// **正在工作**（短暂，不落盘）：主会话据此知道"现在是谁在干活"。
    /// 为什么要有它：成员回合跑在它自己的会话里，主会话在整回合里一个事件都收不到——
    /// 前端只能靠"有增量"去猜运行态，猜不到就不切按钮、也没有占位动画（用户完全不知道在干什么）。
    /// `agent = None` = 空闲（这一回合结束了）。
    Working { agent: Option<String> },
}

/// 调用失败时的用户可见说明：**如实说原因**，并说清会话没被作废（可以点「继续」重试）。
/// 为什么统一在这里生成：讨论、执行、验收、单 agent 都要说同一句话，各写一份必然漂移。
pub fn interrupted_note(reason: &str) -> String {
    format!(
        "[中断] {}；本轮已中断，会话保留——点「继续」可重试。",
        reason
    )
}

/// 用户点「停止」后的用户可见说明：说清这一轮**停在哪**、会话没被作废、怎么接着走。
/// 为什么与"失败"分开说：停止是用户的意图，不是故障；两者混为一句话会让用户以为出了问题。
pub fn stopped_note() -> String {
    "[停止] 已按你的要求停下（被中断的那条发言没有记入转录）；会话保留——点「继续」可接着推进。"
        .to_string()
}

/// 运行态：**这一刻谁在干活**。它在每个干活的人开始前推一条（带名字），收尾推空闲——
/// 前端据此显示"正在工作：某某"与「停止」按钮（见 docs/session/session-model.md 二之二）。
/// 为什么要有它：核心自己的模型调用（整理 / 裁决判定 / 验收）也在干活，不推的话界面显示的
/// 就一直是上一个成员的名字，用户无法判断会话到底在不在跑。
pub fn working(agent: &str) -> SessionEvent {
    SessionEvent::Working {
        agent: Some(agent.to_string()),
    }
}

/// 收尾 / 空闲 / 等用户（下一棒开始时会再推自己的名字）。
pub fn idle() -> SessionEvent {
    SessionEvent::Working { agent: None }
}

/// 实时输出通道：调用参数（流式与预算，来自全局设置）+ 中止开关 + 短暂事件出口
/// （不落盘，仅活动会话实时刷新）。
pub struct Live<'a> {
    pub llm: crate::capabilities::llm::api::LlmOpts,
    /// 用户点「停止」时置位；会话与适配层据此立即中止生成。
    pub cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub emit: &'a mut dyn FnMut(SessionEvent),
    /// 本会话的裁决队（工具级确认这类"等在工作线程上"的关进它）。
    /// `None` = 这一趟不接工具级确认（照常执行）——测试、讨论席与没有交互前端的生成都走它。
    pub decisions: Option<std::sync::Arc<super::decisions::DecisionDoor>>,
    /// 目的：本会话的**提问端口**（工具执行层与围栏经它请用户裁决）：走同一条裁决队、**阻塞**等回答。
    ///   `None` = 这一趟没有可回答的前端（CLI 非交互、端到端夹具、测试、讨论席）——按 fail-closed 拒绝。
    pub ask: Option<std::sync::Arc<dyn crate::kernel::ports::AskUser>>,
}

impl Live<'_> {
    /// 是否已被要求中止。
    pub fn cancelled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// 一次工具调用的转录视图：module 为空串 = 内置工具（read/write）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolCallView {
    /// 发言席（agent 实例名）。
    pub speaker: String,
    /// 工具所属模块；空串 = 内置工具。
    pub module: String,
    pub name: String,
    pub ok: bool,
    /// 模型给的参数 JSON 原文。
    pub args: String,
    /// 回注给模型的结果原文（转录即内容）。
    pub output: String,
    /// 该行所属回复的**助手消息正文**（重建上下文用；与实时推出去的那条取同一个串；界面默认不展开）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub raw: String,
    /// 供应商给的调用 id：重建时靠它把结果消息与助手消息里的调用对上（手写信封通道为空）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub call_id: String,
    /// 这次调用属于哪一次模型回复（= 该回复第一行的稳定 id）：重建按它分组，回档按它原子截断。
    #[serde(default)]
    pub reply: u64,
}

impl ToolCallView {
    /// 给人看的标签：有模块就是 模块.工具名，内置工具就是工具名。
    pub fn label(&self) -> String {
        if self.module.is_empty() {
            self.name.clone()
        } else {
            format!("{}.{}", self.module, self.name)
        }
    }
}

/// 一条转录行：id = 会话内稳定序号（自 0 递增，回放可复现）。
/// 一行 = 一轮模型调用；工具调用另占一行并带上调用视图。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LineView {
    pub id: u64,
    /// 这一行属于哪次模型回复（同一次回复的所有行同号；值 = 该回复第一行的 id）。
    /// 重建上下文时靠它把"一条助手消息 + N 条结果"重新拼回去，回档也按它原子截断。
    #[serde(default)]
    pub reply: u64,
    /// **正文**：这一行说的内容。**行首标签不在这里**——说话人与动词是结构化字段
    /// （`speaker` / `verb`），渲染时才拼回 `[谁:动词] 正文`（见 `render`）。
    /// 为什么分开：呈现层与状态派生读字段，不再从正文里抠标签（见 docs/session/session-model.md 二）。
    pub line: String,
    /// **说话人**：agent 实例名 / `用户` / 核心自己的标签（`轮次` / `代拟` / `节点`…）；空 = 无标签。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub speaker: String,
    /// **标签的第二段**（`[说话人:动词]` 的那个动词）：say / agree / leave / ask / 需求 / 开始 / 撤回…；空 = 没有。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub verb: String,
    /// **行的种类**：`msg`（模型发言）/ `user`（用户说的）/ `system`（系统注入或核心自己的行）/
    /// `tool`（工具行，另有 `tool` 视图）/ `round`（讨论的轮次分隔行，正文是轮次号）。
    /// 呈现、重建与状态派生都读它——不靠匹配行文本。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    /// 思维链（若该轮模型给出）；前端永远默认折叠，点击才展开。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// 该行是一次工具调用时带上调用视图；普通文本行没有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<ToolCallView>,
    /// 该行是"信封缺失、按发言原文收录"的降级行。
    /// **结构化信号**：呈现层据此做样式，不靠匹配行文本里的说明文案。
    #[serde(default, skip_serializing_if = "is_false")]
    pub degraded: bool,
    /// 这一行是**系统消息**（系统注入的提醒/边界这类"不是用户说的、也不是模型说的"内容）。
    /// **结构化信号**：呈现层据此换样式，不靠匹配文本——否则系统消息在前端与记录里长得像用户发的。
    #[serde(default, skip_serializing_if = "is_false")]
    pub system: bool,
    /// 这一行是**核心派给这个 agent 的任务**（派发行）：界面上是系统行（说话人是核心，不冒充用户），
    /// 上下文里以 **user 角色**转发——派活是一次"回合"，而会话协议要求请求里至少有一条 user 消息
    /// （一条 user 都没有的请求会被供应商整条拒收，实测）。
    /// **结构化信号**：重建/回放据此产出发出去的那**同一条消息**，不靠匹配文本。
    /// 见 docs/session/session-model.md 二"系统消息"与四之二"节点执行只有一条管道"。
    #[serde(default, skip_serializing_if = "is_false")]
    pub task: bool,
    /// 这一行属于哪个**回合**（讨论的一次发言回合；0 = 不属任何回合，如需求/用户行）。
    /// 两边的转录行靠它对齐：回档主会话时，各 agent 会话按同一个回合 id 同步截断
    /// （见 docs/session/session-model.md 五）。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub turn: u64,
}

impl LineView {
    /// **模型发言行**：谁说的 + 动词（空 = 该身份没有动词，如单 agent 的正文行）。
    pub fn speech(speaker: &str, verb: &str, line: String) -> LineView {
        LineView {
            speaker: speaker.to_string(),
            verb: verb.to_string(),
            kind: "msg".to_string(),
            line,
            ..Default::default()
        }
    }

    /// **用户说的行**：第二段是核心给用户行起的标签（需求 / 开始 / 撤回 / 同意方案…），空 = 普通发言。
    pub fn user(verb: &str, line: String) -> LineView {
        LineView {
            speaker: "用户".to_string(),
            verb: verb.to_string(),
            kind: "user".to_string(),
            line,
            ..Default::default()
        }
    }

    /// **核心自己的行**：系统注入的提醒/边界（`speaker` 空 = 没有说话人），
    /// 或核心的分隔/说明行（如"代拟"、"节点"）。正文即所见。
    pub fn system(speaker: &str, line: String) -> LineView {
        LineView {
            speaker: speaker.to_string(),
            kind: "system".to_string(),
            system: true,
            line,
            ..Default::default()
        }
    }

    /// **讨论的轮次分隔行**：正文 = 轮次号（结构化，不再从"轮次 2"里抠数字）。
    pub fn round(n: usize) -> LineView {
        LineView {
            speaker: "轮次".to_string(),
            kind: "round".to_string(),
            line: n.to_string(),
            ..Default::default()
        }
    }

    /// 渲染成**文本**：`[谁:动词] 正文` / `[谁] 正文` / `[轮次 N]` / 正文。
    /// 提示词里的转录、回档重建、回放比较都读它——"行怎么变成文本"只有这一处。
    pub fn render(&self) -> String {
        if self.kind == "round" {
            return format!("[{} {}]", self.speaker, self.line);
        }
        let label = match (self.speaker.is_empty(), self.verb.is_empty()) {
            (true, _) => String::new(),
            (false, true) => self.speaker.clone(),
            (false, false) => format!("{}:{}", self.speaker, self.verb),
        };
        match (label.is_empty(), self.line.is_empty()) {
            (true, _) => self.line.clone(),
            (false, true) => format!("[{}]", label),
            (false, false) => format!("[{}] {}", label, self.line),
        }
    }
}

/// serde 用：0 时不写进线格式。
fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// serde 用：false 时不写进线格式。
fn is_false(b: &bool) -> bool {
    !*b
}

/// 验收条目的呈现视图。
#[derive(Debug, Clone, serde::Serialize)]
pub struct CheckView {
    pub item: String,
    pub status: String,
    pub note: String,
}

/// 流水里**最后一次**压缩（`compacted` 事件）：重建发送视图时按它把 `up_to` 之前的行换成摘要。
/// 回档到压缩点之前时这条事件已随转录被截掉，所以「没有它」就是「回到压缩前」。
pub fn last_compaction(events: &[serde_json::Value]) -> Option<(u64, String)> {
    let mut found = None;
    for ev in events {
        if ev.get("type").and_then(|t| t.as_str()) != Some("compacted") {
            continue;
        }
        let up_to = ev.get("up_to").and_then(|u| u.as_u64()).unwrap_or(0);
        let summary = ev.get("summary").and_then(|s| s.as_str()).unwrap_or("");
        if up_to > 0 && !summary.is_empty() {
            found = Some((up_to, summary.to_string()));
        }
    }
    found
}

impl SessionEvent {
    /// 线格式：Web 长轮询与会话历史落盘共用同一形态（转录即内容，落盘即回放）。
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            SessionEvent::Notice(n) => serde_json::json!({ "type": "notice", "text": n }),
            SessionEvent::PlanReview { plan, chain } => {
                serde_json::json!({ "type": "plan_review", "plan": plan, "chain": chain })
            }
            SessionEvent::Compacted { up_to, summary } => serde_json::json!({
                "type": "compacted",
                "up_to": up_to,
                "summary": summary
            }),
            SessionEvent::NodeStarted {
                node,
                sid,
                assignee,
            } => serde_json::json!({
                "type": "node_started",
                "node": node,
                "sid": sid,
                "assignee": assignee
            }),
            SessionEvent::Transcript(lines) => {
                serde_json::json!({ "type": "transcript", "lines": lines })
            }
            SessionEvent::DiscussionDone { round, over_cap } => {
                serde_json::json!({ "type": "discussion_done", "round": round, "over_cap": over_cap })
            }
            SessionEvent::Plan(p) => serde_json::json!({ "type": "plan", "text": p }),
            SessionEvent::Report { id, text, rework } => {
                serde_json::json!({ "type": "report", "id": id, "text": text, "rework": rework })
            }
            SessionEvent::Review { items, raw } => {
                serde_json::json!({ "type": "review", "items": items, "raw": raw })
            }
            SessionEvent::Delivery { ok, over_rework } => {
                serde_json::json!({ "type": "delivery", "ok": ok, "over_rework": over_rework })
            }
            SessionEvent::Ended => serde_json::json!({ "type": "ended" }),
            SessionEvent::Delta {
                speaker,
                kind,
                text,
            } => {
                serde_json::json!({ "type": "delta", "speaker": speaker, "kind": kind, "text": text })
            }
            SessionEvent::Working { agent } => {
                serde_json::json!({ "type": "working", "agent": agent })
            }
            SessionEvent::DecisionCard {
                card,
                gate,
                payload,
                waiting,
            } => {
                let mut v = serde_json::to_value(card).unwrap_or(serde_json::Value::Null);
                if let Some(o) = v.as_object_mut() {
                    o.insert("type".to_string(), serde_json::json!("decision_card"));
                    o.insert("gate".to_string(), serde_json::json!(gate));
                    o.insert("payload".to_string(), payload.clone());
                    o.insert("waiting".to_string(), serde_json::json!(waiting));
                }
                v
            }
            SessionEvent::DecisionVoid { cards, reason } => serde_json::json!({
                "type": "decision_void",
                "cards": cards,
                "reason": reason
            }),
            SessionEvent::DecisionAnswer(a) => {
                let mut v = serde_json::to_value(a).unwrap_or(serde_json::Value::Null);
                if let Some(o) = v.as_object_mut() {
                    o.insert("type".to_string(), serde_json::json!("decision_answer"));
                }
                v
            }
            SessionEvent::ToolCall(v) => serde_json::json!({
                "type": "tool_call",
                "speaker": v.speaker,
                "module": v.module,
                "name": v.name,
                "ok": v.ok,
                "args": v.args,
                "output": v.output,
                "raw": v.raw,
            }),
        }
    }
}

/// 用户介入请求：会话暂停，等前端回应。
#[derive(Debug, Clone)]
pub enum Pending {
    /// 模块请教用户（yes,allow 自裁模式下不会出现）。
    Ask { member: String, question: String },
    /// 核心代拟名单待确认。
    ConfirmSlate,
    /// 名单已定，等用户确认开始讨论（可授权自裁）。
    ConfirmBegin,
    /// 方案待审：整理完**不自动开工**，等用户点「同意」（见 docs/collab/task-chain.md）。
    PlanReview,
    /// 节点验收没过：等用户点「继续」重派这些节点。
    NodeBlocked { nodes: Vec<String> },
    /// **工具层自己发起的一类**（围栏这一环装不上、今后任何底层 yes/no）：消息与选项**由发起方给**，
    /// 通道不解释内容。与工具级确认同属**等在工作线程上**的那一类——发起方阻塞等回答，不点不继续。
    /// 装箱：它比其它变体大得多，而这一关本来就只挂几条（不值得让整队每个格子都变宽）。
    ToolAsk(Box<crate::kernel::api::Ask>),
    /// 工具级确认（`ask` 粒度命中）：这一席的这次调用**在执行前**等用户放行。
    /// 它是**等在工作线程上**的那一类——发起方（工具循环）阻塞等回答，不点不继续。
    ToolApproval {
        /// 谁要跑这次调用（agent 实例名）：卡片的信封显示它。
        agent: String,
        /// 这次调用属于哪个模块（内置 / 核心自有为空，没有模块的工具按工具名问）。
        module: Option<String>,
        tool: String,
        /// 模型给的参数原文（如实展示，让用户看清要执行什么）。
        args: String,
    },
}

/// 目的：请教这一关的选项 id——附一句回话（id 是契约，改文案不改行为）。
pub const OPT_ASK_REPLY: &str = "ask_reply";
/// 目的：代拟名单这一关的选项 id——按此建组。
pub const OPT_SLATE_CONFIRM: &str = "slate_confirm";
/// 目的：代拟名单这一关的选项 id——取消这次工作。
pub const OPT_SLATE_CANCEL: &str = "slate_cancel";
/// 目的：开始讨论这一关的选项 id——开始。
pub const OPT_BEGIN: &str = "begin";
/// 目的：开始讨论这一关的选项 id——开始，并授权小组自裁细节。
pub const OPT_BEGIN_ALLOW: &str = "begin_allow";
/// 目的：方案待审这一关的选项 id——开工。
pub const OPT_PLAN_START: &str = "plan_start";
/// 目的：方案待审这一关的选项 id——先说一句，由核心 AI 判这句话够不够明确。
pub const OPT_PLAN_SAY: &str = "plan_say";
/// 目的：节点没过这一关的选项 id——重派没过的那些节点。
pub const OPT_NODE_REWORK: &str = "node_rework";
/// 目的：节点没过这一关的选项 id——先说一句，由核心 AI 判这句话够不够明确。
pub const OPT_NODE_SAY: &str = "node_say";
/// 目的：工具级确认的选项 id——放行这一次。
pub const OPT_TOOL_ALLOW: &str = "allow";
/// 目的：工具级确认的选项 id——拒绝（工具不执行，回一条"用户拒绝"的结果给模型）。
pub const OPT_TOOL_DENY: &str = "deny";
/// 目的：工具级确认的选项 id——放行这一次，且**本轮（这次生成）剩余调用都不再问**（不落盘成策略）。
pub const OPT_TOOL_FULL: &str = "full";

/// 目的：工具的全名（内置 / 核心自有 = 工具名；模块工具 = 模块.工具）：卡片与记录都用这一种写法。
pub fn qualify(module: Option<&str>, tool: &str) -> String {
    match module {
        Some(m) => format!("{}.{}", m, tool),
        None => tool.to_string(),
    }
}

impl Pending {
    /// 目的：这一关的机制名（落盘与重建按它认门）。
    pub fn kind(&self) -> &'static str {
        match self {
            Pending::Ask { .. } => "ask",
            Pending::ConfirmSlate => "confirm_slate",
            Pending::ConfirmBegin => "confirm_begin",
            Pending::PlanReview => "plan_review",
            Pending::NodeBlocked { .. } => "node_blocked",
            Pending::ToolApproval { .. } => "tool_approval",
            // 工具层自己发起的那一类（围栏这类）：机制名固定，重建材料就是那条问题本身。
            Pending::ToolAsk(_) => "tool_ask",
        }
    }

    /// 目的：这一关的**机制载荷**（重启后重建这一关的材料：谁在问、问的什么、哪些节点没过）。
    /// 约束：它是机制自己的材料，不是渲染内容——呈现层不看它（见 session-model.md「请用户裁决」）。
    pub fn payload(&self) -> serde_json::Value {
        match self {
            Pending::Ask { member, question } => {
                serde_json::json!({ "member": member, "question": question })
            }
            Pending::NodeBlocked { nodes } => serde_json::json!({ "nodes": nodes }),
            // 工具层那一类：重建材料就是那条问题（谁在问 + 消息 + 选项集）。
            Pending::ToolAsk(ask) => serde_json::to_value(ask).unwrap_or(serde_json::Value::Null),
            Pending::ToolApproval {
                agent,
                module,
                tool,
                args,
            } => serde_json::json!({
                "agent": agent,
                "module": module,
                "tool": tool,
                "args": args
            }),
            _ => serde_json::json!({}),
        }
    }

    /// 目的：从落盘的机制名与载荷重建这一关（重启后按转录重建挂起，已答过的不再挂）。
    /// 返回：认得出就是这一关；认不出就是 None——不猜、不硬凑一张卡出来。
    pub fn from_payload(gate: &str, payload: &serde_json::Value) -> Option<Pending> {
        let s = |k: &str| {
            payload
                .get(k)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        Some(match gate {
            "ask" => Pending::Ask {
                member: s("member"),
                question: s("question"),
            },
            "confirm_slate" => Pending::ConfirmSlate,
            "confirm_begin" => Pending::ConfirmBegin,
            "plan_review" => Pending::PlanReview,
            "node_blocked" => Pending::NodeBlocked {
                nodes: payload
                    .get("nodes")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default(),
            },
            // 工具级确认与**工具层自己发起的那一类**都**不重建**：它们等在工作线程上，
            // 重启后那个等待方已经不存在——重建出来的卡没人能做主，等于拿一张答不了卡糊用户。
            _ => return None,
        })
    }

    /// 目的：这一关的选项集（有序）：id 是行为契约，label 只给渲染。
    /// 约束：每一条都必须是真能执行的——通道里没有"置灰"这一说（见 session-model.md「请用户裁决」）。
    pub fn options(&self) -> Vec<DecisionOption> {
        let o = |id: &str, label: &str| DecisionOption {
            id: id.to_string(),
            label: label.to_string(),
        };
        match self {
            Pending::Ask { .. } => vec![o(OPT_ASK_REPLY, "回话")],
            Pending::ConfirmSlate => vec![
                o(OPT_SLATE_CONFIRM, "确认建组"),
                o(OPT_SLATE_CANCEL, "取消"),
            ],
            Pending::ConfirmBegin => vec![
                o(OPT_BEGIN, "开始"),
                o(OPT_BEGIN_ALLOW, "开始（授权小组自裁细节）"),
            ],
            Pending::PlanReview => vec![
                o(OPT_PLAN_START, "开工"),
                o(OPT_PLAN_SAY, "先说一句（核心判明确性）"),
            ],
            Pending::NodeBlocked { .. } => vec![
                o(OPT_NODE_REWORK, "重派没过的节点"),
                o(OPT_NODE_SAY, "先说一句（核心判明确性）"),
            ],
            // 工具级确认：三条都是**真能执行**的（通道里没有置灰那一说）。
            Pending::ToolApproval { .. } => vec![
                o(OPT_TOOL_ALLOW, "放行这一次"),
                o(OPT_TOOL_DENY, "拒绝（不执行）"),
                o(OPT_TOOL_FULL, "放行，且本轮都不再问"),
            ],
            // 工具层那一类：选项集**由发起方给**（它才知道每条选项真能不能执行），通道原样渲染。
            Pending::ToolAsk(ask) => ask.options.iter().map(|(id, label)| o(id, label)).collect(),
        }
    }

    /// 目的：这一关的某个选项**要不要附言**：要就给出"为什么"的原文（队列在出队前按它校验）。
    /// 约束：判据属于发起方（这一关自己的规则）；校验只此一处——被拒的回答不出队、不落回答。
    pub fn note_requirement(&self, option: &str) -> Option<String> {
        match (self, option) {
            (Pending::Ask { .. }, OPT_ASK_REPLY) => {
                Some("这一关要附一句回话：把要说的话写在附言里。".to_string())
            }
            (Pending::PlanReview, OPT_PLAN_SAY) | (Pending::NodeBlocked { .. }, OPT_NODE_SAY) => {
                Some("这一关要附一句你的想法：把要说的话写在附言里。".to_string())
            }
            _ => None,
        }
    }

    /// 目的：这一关的卡片（id + 信封 + 消息 + 选项）——推的事件与快照**只从这一处**派生。
    /// 参数：id = 会话内唯一的卡号；advice = 核心 AI 给的建议（随产生这一关的那次调用一起产出）。
    pub fn card(&self, id: &str, advice: &str) -> DecisionCard {
        let (role, name, title, body, detail): (&str, String, String, String, String) = match self {
            Pending::Ask { member, question } => (
                "member",
                member.clone(),
                format!("{} 在等你回话", member),
                "你说的话会进主会话，所有成员都看得到。".to_string(),
                question.clone(),
            ),
            Pending::ConfirmSlate => (
                "core",
                "核心".to_string(),
                "是否按此建组？".to_string(),
                "核心已代拟名单（见转录）。".to_string(),
                advice.to_string(),
            ),
            Pending::ConfirmBegin => (
                "core",
                "核心".to_string(),
                "现在开始讨论？".to_string(),
                "名单已定。".to_string(),
                advice.to_string(),
            ),
            Pending::PlanReview => (
                "core",
                "核心".to_string(),
                "要不要现在开工？".to_string(),
                "方案与任务链已备好；按规则不自动开工。".to_string(),
                advice.to_string(),
            ),
            Pending::NodeBlocked { nodes } => (
                "core",
                "核心".to_string(),
                "有节点没过验收，要不要重派？".to_string(),
                format!("没过验收的节点：{}。", nodes.join("、")),
                advice.to_string(),
            ),
            Pending::ToolApproval {
                agent,
                module,
                tool,
                args,
            } => (
                "tools",
                agent.clone(),
                format!("是否执行工具 {}？", qualify(module.as_deref(), tool)),
                format!(
                    "{} 请求执行这次调用：这一席的权限表把它列为「要问」（ask 粒度）。",
                    agent
                ),
                args.clone(),
            ),
            // 工具层那一类：**消息与选项原样来自发起方**——通道不解释内容，只把三段话与选项渲染出来。
            Pending::ToolAsk(ask) => (
                ask.role.as_str(),
                ask.name.clone(),
                ask.title.clone(),
                ask.body.clone(),
                ask.detail.clone(),
            ),
        };
        DecisionCard {
            id: id.to_string(),
            envelope: DecisionEnvelope {
                role: role.to_string(),
                name,
            },
            message: DecisionMessage {
                title,
                body,
                detail,
            },
            options: self.options(),
        }
    }

    /// 目的：这一关作为**等待者**的形态（谁在等、问的什么 + 它自己的重建材料）。
    /// 参数：id = 它自己的卡号；advice = 它那一关的建议（与队首同一来源，重建时要它）。
    pub fn waiter(&self, id: &str, advice: &str) -> DecisionWaiter {
        let card = self.card(id, advice);
        let mut payload = self.payload();
        if let Some(o) = payload.as_object_mut() {
            o.insert("advice".to_string(), serde_json::json!(advice));
        }
        DecisionWaiter {
            id: id.to_string(),
            envelope: card.envelope,
            title: card.message.title,
            gate: self.kind().to_string(),
            payload,
        }
    }

    /// 目的：推给用户的裁决事件（卡片 + 机制自己的重建材料 + 后面还在等的那几张）。
    /// 参数：waiting = 排在队首之后的等待者（先来后到）；队列里只有它一张就是空。
    pub fn event(&self, id: &str, advice: &str, waiting: &[DecisionWaiter]) -> SessionEvent {
        let mut payload = self.payload();
        if let Some(o) = payload.as_object_mut() {
            o.insert("advice".to_string(), serde_json::json!(advice));
        }
        SessionEvent::DecisionCard {
            card: self.card(id, advice),
            gate: self.kind().to_string(),
            payload,
            waiting: waiting.to_vec(),
        }
    }

    /// 目的：快照形态（会话视图里的 pending）——与推的事件**同一份事实**。
    pub fn to_json(&self, id: &str, advice: &str, waiting: &[DecisionWaiter]) -> serde_json::Value {
        self.event(id, advice, waiting).to_json()
    }
}
