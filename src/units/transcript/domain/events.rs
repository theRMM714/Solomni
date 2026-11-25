//! 目的：呈现侧契约——事件与介入请求的词汇与形状，前端按此渲染；呈现即上下文，转录行与核心记录完全一致。
//! 管：`SessionEvent` / `LineView` / `ToolCallView` / `DecisionCard` 等类型，以及 `idle` / `working` / `qualify` 等构造。
//! 不管：会话状态与派生（在会话单元的 `domain/session.rs`）、裁决排队与等待（在会话单元的 `domain/decisions.rs`）、前端渲染（在呈现层）。
//! 联动：由会话单元与 runtime 产生，经 `src/units/transcript/api.rs` 导出；契约见 `docs/session/session-model.md`。

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
///   裁决卡的选项与思考档的选项是同一形状，全项目只此一份。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OptionView {
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
    pub options: Vec<OptionView>,
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

/// 目的：一条会话最近一次模型调用的上下文用量（状态栏与压缩判定读它）。
/// 约束：**不是**转录行、不进模型上下文；它只回答"上一次请求用了多少 token"。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ContextUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    /// 目的：这次调用时该模型的上下文窗口（缺省模型 = 128K 兜底，由 runtime 给）。
    pub window: u64,
    /// 目的：true = 供应商回的 usage；false = 按对话字符估算（界面必须标注）。
    pub exact: bool,
    /// 目的：输入里命中前缀缓存的那部分（供应商没报 = None，不是 0）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<u64>,
    /// 目的：这次写进前缀缓存的输入（供应商没报 = None，不是 0）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation: Option<u64>,
}

impl ContextUsage {
    /// 目的：供应商 usage → 带窗口的上下文用量。
    pub fn from_tokens(tokens: crate::units::llm::api::TokenUsage, window: u64) -> ContextUsage {
        ContextUsage {
            prompt_tokens: tokens.prompt_tokens,
            completion_tokens: tokens.completion_tokens,
            total_tokens: tokens.total_tokens,
            window,
            exact: true,
            cache_read: tokens.cache_read,
            cache_creation: tokens.cache_creation,
        }
    }

    /// 目的：拿不到供应商 usage 时的保守估算（≈ 字符 / 4）；界面据此标"估算"。
    pub fn estimated(chars: usize, window: u64) -> ContextUsage {
        let tokens = (chars / 4).max(if chars > 0 { 1 } else { 0 }) as u64;
        ContextUsage {
            prompt_tokens: tokens,
            completion_tokens: 0,
            total_tokens: tokens,
            window,
            exact: false,
            cache_read: None,
            cache_creation: None,
        }
    }
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

/// 目的：会话事件：驱动前端渲染；转录行为增量，前端按序累积。
///   通用词汇只在这里（notice / transcript / push / ended / tool_call / delta / 裁决 / 回档 / 运行态 / 用量）：
///   某一机制的专有事件一律走 `Push`——`kind` 就是线上 `type`，`payload` 铺在顶层。
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum SessionEvent {
    /// 状态提示（建组、返工、上限等）。
    Notice(String),
    /// 转录新增行（带会话内稳定 id，回档按它定位）。
    Transcript(Vec<LineView>),
    /// **机制自己的事件**（推 + 落盘）：线格式 = payload 铺平后插入 `type: kind`。
    /// 为什么要有它：会话只提供通道，不认识任何机制的词汇；渲染与派生按 kind 解释。
    Push {
        kind: String,
        payload: serde_json::Value,
    },
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
    /// **回档重置**（短暂，不落盘）：回档成功后，事件台用它携带该会话回档后的**活动视图**。
    /// 前端收到后清空当前视图并按 `events` 重建；历史合流以它为**重置点**，不把回档前的旧事实
    /// 当成实时尾巴再拼回来。为什么要有它：回档改的是盘上转录，事件台本身不能回退——
    /// 没有这条事实，刷新/多端就无法知道"从这里开始，旧事件已经作废"。
    Rewound { events: Vec<serde_json::Value> },
    /// **正在工作**（短暂，不落盘）：主会话据此知道"现在是谁在干活"。
    /// 为什么要有它：成员回合跑在它自己的会话里，主会话在整回合里一个事件都收不到——
    /// 前端只能靠"有增量"去猜运行态，猜不到就不切按钮、也没有占位动画（用户完全不知道在干什么）。
    /// `agent = None` = 空闲（这一回合结束了）。
    Working { agent: Option<String> },
    /// **上下文用量**（落盘，但不是转录行、不进模型上下文）：最近一次模型调用的输入/输出 token。
    /// `line` 是这次调用产出第一行的行号；回档折叠到它之前时这条用量随之失效。
    Usage { usage: ContextUsage, line: u64 },
}

/// 目的：调用失败时的用户可见说明：**如实说原因**，并说清会话没被作废（可以点「继续」重试）。
///   为什么统一在这里生成：讨论、执行、验收、单 agent 都要说同一句话，各写一份必然漂移。
pub fn interrupted_note(reason: &str) -> String {
    format!(
        "[中断] {}；本轮已中断，会话保留——点「继续」可重试。",
        reason
    )
}

/// 目的：生成被用户打断（「中断」按钮 / 「停止会话」）后的用户可见说明：说清**停在哪**、半句去哪了、
///   会话没被作废、怎么接着走。
///   为什么与"失败"分开说：打断是用户的意图，不是故障；两者混成一句话会让用户以为出了问题。
pub fn stopped_note() -> String {
    "[停止] 已按你的要求停下：已经写出来的那部分留在上面（标了「（已中断）」），不算这一轮的回答；会话保留——点「继续」会从这儿重开一轮，也可以直接发消息。"
        .to_string()
}

/// 目的：运行态：**这一刻谁在干活**。它在每个干活的人开始前推一条（带名字），收尾推空闲——
///   前端据此显示"正在工作：某某"与「停止」按钮（见 docs/session/session-model.md 二之二）。
///   为什么要有它：核心自己的模型调用（整理 / 裁决判定 / 验收）也在干活，不推的话界面显示的
///   就一直是上一个成员的名字，用户无法判断会话到底在不在跑。
pub fn working(agent: &str) -> SessionEvent {
    SessionEvent::Working {
        agent: Some(agent.to_string()),
    }
}

/// 目的：收尾 / 空闲 / 等用户（下一棒开始时会再推自己的名字）。
pub fn idle() -> SessionEvent {
    SessionEvent::Working { agent: None }
}

/// 目的：一次工具调用的转录视图：module 为空串 = 内置工具（read/write）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolCallView {
    /// 目的：发言席（agent 实例名）。
    pub speaker: String,
    /// 目的：工具所属模块；空串 = 内置工具。
    pub module: String,
    pub name: String,
    pub ok: bool,
    /// 目的：回执的轻重（info / warning / error）——界面按它上色，不看正文猜。
    #[serde(default)]
    pub severity: crate::kernel::api::note::Severity,
    /// 目的：模型给的参数 JSON 原文。
    pub args: String,
    /// 目的：进程/工具给的原始输出（不含按语言渲染的句子）。
    pub output: String,
    /// 目的：回执说明——稳定码 + 参数 + 轻重；呈现层按语言渲染，模型通道由核心渲染。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<crate::kernel::api::note::Note>,
    /// 目的：供应商给的调用 id：重建时靠它把结果消息与助手消息里的调用对上（手写信封通道为空）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub call_id: String,
    /// 目的：这次调用属于哪一次模型回复（= 该回复第一行的稳定 id）：重建按它分组，回档按它原子截断。
    #[serde(default)]
    pub reply: u64,
    /// 目的：这次调用产生的观察事实（读过 / 写过 / 改过哪些行、当时整份版本）；发送视图据此判写入门禁。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observations: Vec<crate::units::tools::api::ObservationFact>,
    /// 目的：这次调用若是一次 shell，执行层当场算好的结构化事实（耗时 / 围栏准备 / 落点 / 退出码 / 状态 / 截断）；
    ///   实况卡片与会话回放都读它，不解析回执文本。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<crate::units::tools::api::ShellFacts>,
}

impl ToolCallView {
    /// 目的：给人看的标签：有模块就是 模块.工具名，内置工具就是工具名。
    pub fn label(&self) -> String {
        if self.module.is_empty() {
            self.name.clone()
        } else {
            format!("{}.{}", self.module, self.name)
        }
    }
}

/// 目的：一条转录行：id = 会话内稳定序号（自 0 递增，回放可复现）。
///   一行 = 一轮模型调用；工具调用另占一行并带上调用视图。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LineView {
    pub id: u64,
    /// 目的：这一行属于哪次模型回复（同一次回复的所有行同号；值 = 该回复第一行的 id）。
    ///   重建上下文时靠它把"一条助手消息 + N 条结果"重新拼回去，回档也按它原子截断。
    #[serde(default)]
    pub reply: u64,
    /// 目的：**正文**：这一行说的内容。**行首标签不在这里**——说话人与动词是结构化字段
    ///   （`speaker` / `verb`），渲染时才拼回 `[谁:动词] 正文`（见 `render`）。
    ///   为什么分开：呈现层与状态派生读字段，不再从正文里抠标签（见 docs/session/session-model.md 二）。
    pub line: String,
    /// 目的：**说话人**：agent 实例名 / `用户` / 核心自己的标签（`轮次` / `代拟` / `节点`…）；空 = 无标签。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub speaker: String,
    /// 目的：**标签的第二段**（`[说话人:动词]` 的那个动词）：say / agree / leave / ask / 需求 / 开始 / 撤回…；空 = 没有。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub verb: String,
    /// 目的：**行的种类**：`msg`（模型发言）/ `user`（用户说的）/ `system`（系统注入或核心自己的行）/
    ///   `tool`（工具行，另有 `tool` 视图）/ `round`（讨论的轮次分隔行，正文是轮次号）。
    ///   呈现、重建与状态派生都读它——不靠匹配行文本。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    /// 目的：思维链（若该轮模型给出）；前端永远默认折叠，点击才展开。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// 目的：本行所属回复的**助手消息原文**（重建上下文用；每条回复只落一份，挂在该回复首行）。
    /// 约束：信封模式下含信封 JSON，原生模式下即可见正文；呈现层只在整轮气泡里展示一次。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub raw: String,
    /// 目的：该行是一次工具调用时带上调用视图；普通文本行没有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<ToolCallView>,
    /// 目的：该行是"信封缺失、按发言原文收录"的降级行。
    ///   **结构化信号**：呈现层据此做样式，不靠匹配行文本里的说明文案。
    #[serde(default, skip_serializing_if = "is_false")]
    pub degraded: bool,
    /// 目的：这一行是**被中断的半句**：用户中途打断生成，已经写出的那部分**留档给人看**。
    ///   约束：它**不进模型上下文**（重建按它跳过，见 turn::turn::service::rebuild）：点「继续」是
    ///   从断点**重开一轮**，不是把半句当发言续写。呈现层据此换样式，不靠匹配正文里的「（已中断）」。
    #[serde(default, skip_serializing_if = "is_false")]
    pub cut: bool,
    /// 目的：这一行是**系统消息**（系统注入的提醒/边界这类"不是用户说的、也不是模型说的"内容）。
    ///   **结构化信号**：呈现层据此换样式，不靠匹配文本——否则系统消息在前端与记录里长得像用户发的。
    #[serde(default, skip_serializing_if = "is_false")]
    pub system: bool,
    /// 目的：这一行是**核心派给这个 agent 的任务**（派发行）：界面上是系统行（说话人是核心，不冒充用户），
    ///   上下文里以 **user 角色**转发——派活是一次"回合"，而会话协议要求请求里至少有一条 user 消息
    ///   （一条 user 都没有的请求会被供应商整条拒收，实测）。
    ///   **结构化信号**：重建/回放据此产出发出去的那**同一条消息**，不靠匹配文本。
    ///   见 docs/session/session-model.md 二"系统消息"与四之二"节点执行只有一条管道"。
    #[serde(default, skip_serializing_if = "is_false")]
    pub task: bool,
    /// 目的：这一行属于哪个**回合**（讨论的一次发言回合；0 = 不属任何回合，如需求/用户行）。
    ///   两边的转录行靠它对齐：回档主会话时，各 agent 会话按同一个回合 id 同步截断
    ///   （见 docs/session/session-model.md 五）。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub turn: u64,
    /// 目的：控制行的结构化载荷（如 `compacted` 的 `up_to`）；普通消息行没有。
    /// 约束：呈现与重建只读它，不从正文里解析控制字段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

impl LineView {
    /// 目的：**模型发言行**：谁说的 + 动词（空 = 该身份没有动词，如单 agent 的正文行）。
    pub fn speech(speaker: &str, verb: &str, line: String) -> LineView {
        LineView {
            speaker: speaker.to_string(),
            verb: verb.to_string(),
            kind: "msg".to_string(),
            line,
            ..Default::default()
        }
    }

    /// 目的：**用户说的行**：第二段是核心给用户行起的标签（需求 / 开始 / 撤回 / 同意方案…），空 = 普通发言。
    pub fn user(verb: &str, line: String) -> LineView {
        LineView {
            speaker: "用户".to_string(),
            verb: verb.to_string(),
            kind: "user".to_string(),
            line,
            ..Default::default()
        }
    }

    /// 目的：**核心自己的行**：系统注入的提醒/边界（`speaker` 空 = 没有说话人），
    ///   或核心的分隔/说明行（如"代拟"、"节点"）。正文即所见。
    pub fn system(speaker: &str, line: String) -> LineView {
        LineView {
            speaker: speaker.to_string(),
            kind: "system".to_string(),
            system: true,
            line,
            ..Default::default()
        }
    }

    /// 目的：构造一条**内容类系统提示**行（转录一等行，**不进模型上下文**）。
    /// 约束：与 `system` 的区别——`system` 是上下文里的系统消息；`notice` 只给人看。
    pub fn notice(line: String) -> LineView {
        LineView {
            kind: "notice".to_string(),
            line,
            ..Default::default()
        }
    }

    /// 目的：**核心写下的记录行**：计划 / 验收清单 / 交付这类"核心的裁决事实"，带说话人，只给人看。
    ///   约束：种类仍是 `notice`——"只给人看、不进模型上下文"的唯一判据就是它（重建按 kind 跳过，
    ///   见 turn::turn::service::rebuild）；**不能**用 `system`，那会变成上下文里的系统消息。
    ///   与 `notice` 的区别只有说话人：`notice` 是没有署名的系统提示，`record` 署名到核心的某个身份。
    pub fn record(speaker: &str, line: String) -> LineView {
        LineView {
            speaker: speaker.to_string(),
            kind: "notice".to_string(),
            line,
            ..Default::default()
        }
    }

    /// 目的：**讨论的轮次分隔行**：正文 = 轮次号（结构化，不再从"轮次 2"里抠数字）。
    pub fn round(n: usize) -> LineView {
        LineView {
            speaker: "轮次".to_string(),
            kind: "round".to_string(),
            line: n.to_string(),
            ..Default::default()
        }
    }

    /// 目的：渲染成**文本**：`[谁:动词] 正文` / `[谁] 正文` / `[轮次 N]` / 正文。
    ///   提示词里的转录、回档重建、回放比较都读它——"行怎么变成文本"只有这一处。
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

/// 目的：取流水里**最后一次**压缩行（`up_to`、摘要、行 id）。
/// 约束：回档到压缩行之前时它已随转录被截掉，所以「没有它」就是「回到压缩前」。
pub fn last_compaction(events: &[serde_json::Value]) -> Option<(u64, String, u64)> {
    let mut found = None;
    for ev in events {
        if ev.get("type").and_then(|t| t.as_str()) != Some("transcript") {
            continue;
        }
        let Some(lines) = ev.get("lines").and_then(|l| l.as_array()) else {
            continue;
        };
        for l in lines {
            if l.get("kind").and_then(|k| k.as_str()) != Some("compacted") {
                continue;
            }
            let up_to = l
                .get("payload")
                .and_then(|p| p.get("up_to"))
                .and_then(|u| u.as_u64())
                .unwrap_or(0);
            let summary = l.get("line").and_then(|s| s.as_str()).unwrap_or("");
            let id = l.get("id").and_then(|i| i.as_u64()).unwrap_or(0);
            if up_to > 0 && !summary.is_empty() {
                found = Some((up_to, summary.to_string(), id));
            }
        }
    }
    found
}

impl SessionEvent {
    /// 目的：线格式：Web 长轮询与会话历史落盘共用同一形态（转录即内容，落盘即回放）。
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            SessionEvent::Notice(n) => serde_json::json!({ "type": "notice", "text": n }),
            SessionEvent::Transcript(lines) => {
                serde_json::json!({ "type": "transcript", "lines": lines })
            }
            SessionEvent::Push { kind, payload } => match payload {
                serde_json::Value::Object(m) => {
                    let mut v = serde_json::Value::Object(m.clone());
                    if let Some(o) = v.as_object_mut() {
                        o.insert("type".to_string(), serde_json::json!(kind));
                    }
                    v
                }
                other => serde_json::json!({ "type": kind, "payload": other }),
            },
            SessionEvent::Ended => serde_json::json!({ "type": "ended" }),
            SessionEvent::Delta {
                speaker,
                kind,
                text,
            } => {
                serde_json::json!({ "type": "delta", "speaker": speaker, "kind": kind, "text": text })
            }
            SessionEvent::Rewound { events } => {
                serde_json::json!({ "type": "rewound", "events": events })
            }
            SessionEvent::Working { agent } => {
                serde_json::json!({ "type": "working", "agent": agent })
            }
            SessionEvent::Usage { usage, line } => {
                let mut v = serde_json::json!({
                    "type": "usage",
                    "prompt_tokens": usage.prompt_tokens,
                    "completion_tokens": usage.completion_tokens,
                    "total_tokens": usage.total_tokens,
                    "window": usage.window,
                    "exact": usage.exact,
                    "line": line,
                });
                if let Some(o) = v.as_object_mut() {
                    if let Some(n) = usage.cache_read {
                        o.insert("cache_read".to_string(), serde_json::json!(n));
                    }
                    if let Some(n) = usage.cache_creation {
                        o.insert("cache_creation".to_string(), serde_json::json!(n));
                    }
                }
                v
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
            }),
        }
    }
}

impl SessionEvent {
    /// 目的：造一条机制事件：`kind` 即线上 `type`，`payload` 铺在顶层（与落盘/长轮询同一形态）。
    pub fn push(kind: &str, payload: serde_json::Value) -> SessionEvent {
        SessionEvent::Push {
            kind: kind.to_string(),
            payload,
        }
    }
}

/// 目的：裁决队里的一关——**只装数据**：机制名、机制材料、卡、要附言的选项。
/// 约束：队列只按卡校验、按 kind/payload 重建；它不解释这一关是哪一种（解释归发起方机制）。
#[derive(Debug, Clone)]
pub struct Gate {
    /// 目的：这一关的机制名（落盘与重建按它认门）。
    pub kind: String,
    /// 目的：这一关的**机制载荷**（重启后重建这一关的材料），原样进裁决事件与等待者。
    pub payload: serde_json::Value,
    /// 目的：卡片（信封 + 消息 + 选项）；卡号由队列发，这里的不被信任。
    pub card: DecisionCard,
    /// 目的：要附言的选项——id → 要附言的原文（空 = 都不需要）。
    pub notes: Vec<(String, String)>,
}

impl Gate {
    /// 目的：这一关的某个选项**要不要附言**：要就给出"为什么"的原文（队列在出队前按它校验）。
    pub fn note_requirement(&self, option: &str) -> Option<String> {
        self.notes
            .iter()
            .find(|(id, _)| id == option)
            .map(|(_, why)| why.clone())
    }

    /// 目的：这一关作为**等待者**的形态（谁在等、问的什么 + 它自己的重建材料）。
    pub fn waiter(&self, id: &str) -> DecisionWaiter {
        DecisionWaiter {
            id: id.to_string(),
            envelope: self.card.envelope.clone(),
            title: self.card.message.title.clone(),
            gate: self.kind.clone(),
            payload: self.payload.clone(),
        }
    }

    /// 目的：这一关的卡片，卡号以队列发的为准（卡内 id 只是占位）。
    pub fn card_with(&self, id: &str) -> DecisionCard {
        let mut card = self.card.clone();
        card.id = id.to_string();
        card
    }

    /// 目的：推给用户的裁决事件（卡片 + 机制自己的重建材料 + 后面还在等的那几张）。
    /// 参数：waiting = 排在队首之后的等待者（先来后到）；队列里只有它一张就是空。
    pub fn event(&self, id: &str, waiting: &[DecisionWaiter]) -> SessionEvent {
        SessionEvent::DecisionCard {
            card: self.card_with(id),
            gate: self.kind.clone(),
            payload: self.payload.clone(),
            waiting: waiting.to_vec(),
        }
    }

    /// 目的：快照形态（会话视图里的 pending）——与推的事件**同一份事实**。
    pub fn to_json(&self, id: &str, waiting: &[DecisionWaiter]) -> serde_json::Value {
        self.event(id, waiting).to_json()
    }
}

/// 目的：工具层自己发起的一关（围栏这类）：消息与选项**由发起方给**，通道不解释内容。
/// 约束：这一关不重建——它等在工作线程上，重启后等待方已不存在（重建出来也没人做主）。
pub fn tool_ask(ask: &crate::kernel::api::types::Ask) -> Gate {
    Gate {
        kind: "tool_ask".to_string(),
        payload: serde_json::to_value(ask).unwrap_or(serde_json::Value::Null),
        card: DecisionCard {
            id: String::new(),
            envelope: DecisionEnvelope {
                role: ask.role.clone(),
                name: ask.name.clone(),
            },
            message: DecisionMessage {
                title: ask.title.clone(),
                body: ask.body.clone(),
                detail: ask.detail.clone(),
            },
            options: ask
                .options
                .iter()
                .map(|(id, label)| OptionView {
                    id: id.clone(),
                    label: label.clone(),
                })
                .collect(),
        },
        notes: Vec::new(),
    }
}

/// 目的：工具的全名（内置 / 核心自有 = 工具名；模块工具 = 模块.工具）：卡片与记录都用这一种写法。
pub fn qualify(module: Option<&str>, tool: &str) -> String {
    match module {
        Some(m) => format!("{}.{}", m, tool),
        None => tool.to_string(),
    }
}
