//! 呈现侧契约：事件与介入请求。词汇定义在 core，前端按此渲染。
//! 呈现即上下文：转录行与核心记录完全一致。

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
        chain: crate::core::chain::TaskChain,
    },
    /// 上下文压缩：`up_to` 之前的转录**不再发给模型**（转录本身完整保留、用户仍可查看），
    /// 由一份 summary 代替（见 docs/architecture/session-model.md 六）。
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
    /// **请用户裁决**（短暂、不落盘）：核心把"为什么停下来等你"和"我建议怎么做"说清楚，
    /// 用户用自然语言回一句；核心 AI 判定意图是否明确，明确了才开工/放行。
    /// 与 `Pending` 同源：快照里带同一份（刷新页面照样画得出），这条只是推的增量。
    Decision {
        /// 什么在等裁决：ask / confirm_slate / confirm_begin / plan_review / node_blocked
        kind: String,
        /// 核心对当前情况的说明（为什么要你定）
        summary: String,
        /// 核心的建议（没有建议时为空串）
        advice: String,
        /// 要用户回答的那句（ask 时是成员的原话）
        question: String,
        /// 相关载荷（未过的节点 id、名单说明之类）
        payload: serde_json::Value,
    },
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
/// 前端据此显示"正在工作：某某"与「停止」按钮（见 docs/architecture/session-model.md 二之二）。
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
    pub llm: crate::core::ports::LlmOpts,
    /// 用户点「停止」时置位；会话与适配层据此立即中止生成。
    pub cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub emit: &'a mut dyn FnMut(SessionEvent),
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
    /// 为什么分开：呈现层与状态派生读字段，不再从正文里抠标签（见 docs/architecture/session-model.md 二）。
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
    /// 见 docs/architecture/session-model.md 二"系统消息"与四之二"节点执行只有一条管道"。
    #[serde(default, skip_serializing_if = "is_false")]
    pub task: bool,
    /// 这一行属于哪个**回合**（讨论的一次发言回合；0 = 不属任何回合，如需求/用户行）。
    /// 两边的转录行靠它对齐：回档主会话时，各 agent 会话按同一个回合 id 同步截断
    /// （见 docs/architecture/session-model.md 五）。
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
            SessionEvent::Decision {
                kind,
                summary,
                advice,
                question,
                payload,
            } => serde_json::json!({
                "type": "decision",
                "kind": kind,
                "summary": summary,
                "advice": advice,
                "question": question,
                "payload": payload
            }),
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
    /// 方案待审：整理完**不自动开工**，等用户点「同意」（见 docs/architecture/task-chain.md）。
    PlanReview,
    /// 节点验收没过：等用户点「继续」重派这些节点。
    NodeBlocked { nodes: Vec<String> },
}

impl Pending {
    /// 裁决的四个部分：kind / 说明 / 要回答的那句 / 载荷。
    /// **只有这一处派生**：推的 `Decision` 事件与快照里的 `pending` 都来自它，不做第二真相。
    /// **建议（advice）不在这里**：它由核心 AI 在产生这一关的那次调用里一起给出（plan / node_verdict），
    /// 所以由调用方传进来（没有就是空串）。
    pub fn decision_parts(&self) -> (&'static str, String, String, serde_json::Value) {
        match self {
            Pending::Ask { member, question } => (
                "ask",
                format!(
                    "{} 在等你回话。你说的话会进主会话，所有成员都看得到。",
                    member
                ),
                question.clone(),
                serde_json::json!({ "member": member }),
            ),
            Pending::ConfirmSlate => (
                "confirm_slate",
                "核心已代拟名单（见转录）。".to_string(),
                "是否按此建组？".to_string(),
                serde_json::json!({}),
            ),
            Pending::ConfirmBegin => (
                "confirm_begin",
                "名单已定。".to_string(),
                "现在开始讨论？".to_string(),
                serde_json::json!({}),
            ),
            Pending::PlanReview => (
                "plan_review",
                "方案与任务链已备好；按规则**不自动开工**。".to_string(),
                "要不要现在开工？".to_string(),
                serde_json::json!({}),
            ),
            Pending::NodeBlocked { nodes } => (
                "node_blocked",
                "有节点没过验收。".to_string(),
                "要不要放行 / 返工？".to_string(),
                serde_json::json!({ "nodes": nodes }),
            ),
        }
    }

    /// 推给用户的裁决事件（与快照里的 `pending` 同一个事实）。
    pub fn decision(&self, advice: &str) -> SessionEvent {
        let (kind, summary, question, payload) = self.decision_parts();
        SessionEvent::Decision {
            kind: kind.to_string(),
            summary,
            advice: advice.to_string(),
            question,
            payload,
        }
    }

    /// 快照形态（会话视图里的 `pending`）：与 `decision` 同一个形状。
    pub fn to_json(&self, advice: &str) -> serde_json::Value {
        let (kind, summary, question, payload) = self.decision_parts();
        serde_json::json!({
            "type": "decision",
            "kind": kind,
            "summary": summary,
            "advice": advice,
            "question": question,
            "payload": payload
        })
    }
}
