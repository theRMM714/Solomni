//! 目的：单 agent 会话——上下文历史自有，通道来自注入的网关。
//! 管：发言席（只有 agent，id = agent 实例名）、工具循环、转录行与会话内稳定 id、每行对应的历史长度（供回档精确回退）、压缩点。
//! 不管：事件词汇与裁决队列（在 `events.rs` / `decisions.rs`）、历史落盘（在 `detail/`）。
//! 联动：由 engine 驱动（`engine::converse_with`）；回档算术在 `rewind.rs`，历史视图在 `history.rs`。
use crate::units::business::agentloop::domain::member_tools::MemberTools;
use crate::units::business::llm::api::{BoxedChat, Msg, TokenUsage};
use crate::units::business::session_model::api::{UsageTotals, default_freshness_keep_percent};
use crate::units::business::transcript::api::{LineView, SessionEvent, ToolCallView};

/// 目的：把"保留前 keep 行"对齐到**回复边界**：keep 落在某次回复内部时，退到该回复的第一行之前。
///   为什么必须对齐：一次回复的消息是「一条助手消息 + N 条结果」，截在中间会留下孤儿结果
///   （协议要求结果紧跟发起它的助手消息）。转录的截断与内存历史的截断**必须用同一个函数**，
///   否则前端看到的事件流与模型上下文会不一致。
pub fn keep_whole_replies(line_reply: &[u64], keep: usize) -> usize {
    let mut keep = keep.min(line_reply.len());
    if keep > 0 && keep < line_reply.len() {
        let losing = line_reply[keep];
        while keep > 0 && line_reply[keep - 1] == losing {
            keep -= 1;
        }
    }
    keep
}

pub mod prompt;

pub use prompt::{SessionParams, TurnRun};

// 提示拼装只被装配侧用到：测试与 `prompt` 能力经由本模块的 `prompt` 子模块取，不在此重导出。
/// 目的：一个 agent 的会话：模块数不限（形态只在校验与界面标签上区分）。
pub struct AgentSession {
    /// 目的：agent 实例名（说话人标签；重建时也按它命名）。
    pub(crate) id: String,
    /// 会话参数（派生的前提）：不占对话的位置，每次调用现渲染。
    params: SessionParams,
    /// 目的：**只有对话**：user / assistant / tool（+ 核心注入的系统消息）。
    ///   身份与环境不在这里——它们由 params 现渲染（见 docs/tools/tools-and-roles.md 二）。
    pub(crate) dialogue: Vec<Msg>,
    pub(crate) chat: BoxedChat,
    /// 目的：工具环境：内置文件工具按该 agent 的沙箱放行 + 该 agent 模块声明的外部工具。
    pub(crate) tools: Option<MemberTools>,
    /// 目的：压缩回合的指令（提示词段；构造时取自册子）。
    pub(crate) compact_prompt: String,
    /// 目的：下一条转录行的 id。
    pub(crate) next_line: u64,
    /// 目的：每行 id 对应「该行完成时的历史长度」，回档按它截断历史。
    pub(crate) marks: Vec<usize>,
    /// 目的：每行属于哪次模型回复（id 相同 = 同一次回复）。回档**按回复原子**截断靠它：
    ///   截在一次回复中间会留下"孤儿工具结果"，而协议要求结果紧跟发起它的那条助手消息。
    pub(crate) line_reply: Vec<u64>,
    /// 目的：每一行对应的**行 id**（与 `marks` / `line_reply` 同长）。
    /// 约束：留档折叠后行 id 会有空洞，压缩切分必须按 id 记账，不能拿行下标冒充 id。
    pub(crate) line_ids: Vec<u64>,
    /// 目的：正在落行的回复 id（每轮开始时设置；line() 用它，免得每个调用点都传一遍）。
    pub(crate) cur_reply: u64,
    /// 正在落行的**回合 id**（讨论的回合标记用它；单 agent 回合为 0）。
    cur_turn: u64,
    /// 目的：自动压缩的**字符预算**（≈ 模型窗口 × 设置百分比 × 4）；0 = 关。
    /// 约束：拿不到供应商 usage 时用它兜底（见 docs/session/session-model.md 六）。
    pub(crate) compact_at: usize,
    /// 目的：自动压缩的 token 阈值（= 窗口 × 百分比）；0 = 关。有精确 usage 时优先读它。
    pub(crate) compact_tokens: u64,
    /// 目的：压缩百分比（0 = 关；状态栏小卡片要显示它）。
    pub(crate) compact_percent: u8,
    /// 目的：本会话当前模型的上下文窗口（状态栏百分比的分母）。
    pub(crate) context_window: u64,
    /// 目的：最近一次模型调用的用量；None = 还没调用过。
    pub(crate) last_usage: Option<crate::units::business::transcript::api::ContextUsage>,
    /// 目的：本会话**这次生成里**精确报回的用量增量（核心线程交回会话时并进 meta 的累计）。
    /// 约束：只由 `note_usage` 累加、由 `take_usage_delta` 取走清零；它不参与上下文判定。
    usage_delta: UsageTotals,
    /// 目的：压缩点（第一行保留行的 id）；0 = 没压过。
    /// 约束：此前的内容已被摘要取代，对话里补不回来。
    pub(crate) compacted_upto: u64,
    /// 目的：最近一条压缩记录行的 id；0 = 没压过。
    /// 约束：回档按它判断「压缩记录是否被截掉」。
    pub(crate) compacted_line: u64,
    /// 目的：压缩时至少保留最近多少内容（占模型窗口的百分比）；0 = 只留摘要。
    pub(crate) compact_keep_percent: u8,
    /// 目的：压缩通道工厂（触发时现建；None = 只用会话自己的通道，测试/旧装配）。
    compact_factory: Option<std::sync::Arc<dyn CompactChatFactory>>,
    /// 目的：本会话自己的模型 id（用于判断压缩模型是否与会话同模型）；None = 核心默认。
    model: Option<String>,
    /// 目的：本会话解析好的思考参数片段（登记处按"这个 agent 的模型 + 工作根的档位"给）。
    /// 约束：只影响**出站请求**；会话重建时由生成前的准备段重新解析，不落盘。
    thinking: Option<std::sync::Arc<serde_json::Value>>,
    /// 目的：**过期内容自动清理**开关（落盘在 meta，见 docs/session/session-model.md）。
    /// 约束：false = 只记事实、发送视图原样；true = 发送前把过期读取换成占位符。
    /// 它必须跟着会话走：回放与实时产出同一条请求，否则缓存与可复现性都坏。
    context_freshness: bool,
    /// 目的：**过期内容保底**：发送视图里前这么多内容原样保留（0 = 不保底）。
    /// 约束：与开关一样由工作根 meta 派生；替换总量太小（不够付一次缓存失效）时整体不替换。
    freshness_keep_percent: u8,
    /// 目的：最近一次发送视图的替换统计（面板显示"本次替换 N 处 · 省 T tokens"）。
    /// 约束：只反映**这一次**发送；开关关、判定失败或还没发过时为 None。
    freshness_stats: Option<FreshnessStats>,
    /// 目的：本会话里"读过什么"的台账（读事实 + 当时那条工具结果消息的正文）。
    /// 约束：**派生视图的输入**，不是对话内容；压缩会移走旧消息，所以按正文匹配、不按下标。
    pub(crate) reads: Vec<ReadAt>,
}

/// 目的：一条读事实与它落成的那条工具结果消息的正文。
/// 约束：正文用于在发送视图里定位消息（对话会被压缩移位，下标不可靠）；
///   reply 是这条工具结果所在回复的行 id，压缩/回档据此同步作废（与观察账本同一水位）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadAt {
    pub body: String,
    pub reply: u64,
    pub fact: crate::units::business::workspace::api::ReadFact,
}

/// 目的：一次压缩的切分计划。
/// 约束：`up_to` 是第一行保留行的行 id（此前的行被摘要取代）；`cut` 是被移出对话的下标边界。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionPlan {
    /// 目的：第一行保留行的行 id。
    pub up_to: u64,
    /// 目的：被移出对话的下标边界（含摘要消息的偏移）。
    pub cut: usize,
}

/// 目的：独立总结模型的一条临时通道（只在压缩触发时建）。
pub struct CompactChatSpec {
    /// 目的：这次压缩用的临时通道。
    pub chat: BoxedChat,
    /// 目的：登记处的模型 id（报错与日志用）。
    pub model: String,
    /// 目的：这个模型的上下文窗口；被总结前缀超过它时如实报错。
    pub window: u64,
}

/// 目的：压缩通道的工厂（组合根注入；会话只认这个口，不持登记处）。
/// 约束：返回 `None` = 会话同模型，复用会话自己的通道与思考档。
pub trait CompactChatFactory: Send + Sync {
    /// 目的：在压缩触发时现建总结通道。
    /// 返回：`None` = 会话同模型；`Some` = 独立总结模型。
    /// 错误：模型 / 通道不可用时如实返回原因。
    fn build(
        &self,
        module: &str,
        session_model: Option<&str>,
    ) -> Result<Option<CompactChatSpec>, String>;
}

/// 目的：过期读取的占位符——模型看到"这段内容不可信，要就重读"，而不是看不到它存在。
/// 参数：ranges 是这条工具结果里所有过期段（同一份读的分段、或多次重叠读），按显示顺序排好。
pub fn expired_note(path: &str, ranges: &[(u64, u64)]) -> String {
    let mut parts = Vec::with_capacity(ranges.len());
    for (from, to) in ranges {
        if from == to {
            parts.push(from.to_string());
        } else {
            parts.push(format!("{from}-{to}"));
        }
    }
    format!(
        "[已过期：{} 第{}行，读取后文件已修改；需要时重新 read]",
        path,
        parts.join("、")
    )
}

/// 目的：一次发送视图的过期替换统计（面板显示"本次替换 N 处 · 省 T tokens"）。
/// 约束：skipped = 判出了过期但省下的量不够付一次缓存失效，整体回退成原样。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct FreshnessStats {
    /// 目的：这次判定过的读条数。
    pub judged: u64,
    /// 目的：实际换掉的条数（整体回退时为 0）。
    pub replaced: u64,
    /// 目的：换掉后估计省下的输入 token（4 字符 ≈ 1 token，与压缩估算同一口径）。
    pub saved_tokens: u64,
    /// 目的：这次用的保底百分比（0 = 不保底）。
    pub keep_percent: u8,
    /// 目的：省下的量太小，这次没有替换。
    pub skipped: bool,
}

/// 目的：一次替换至少省这么多字符（≈2K token）才值得付一次缓存前缀失效。
pub const FRESHNESS_MIN_SAVED_CHARS: usize = 8000;

/// 目的：**发送视图**的派生——判成过期的那条工具结果换成占位符，其余逐字保留。
/// 约束：按**正文**定位消息（压缩会把旧消息移走、下标不可靠）；正文找不到就跳过，
///   绝不动别的行。reads 与 stale 同序，只包含确实判定过的读；同一条消息的多段读
///   按消息归并成一个占位符（judged 按段计，replaced 按消息计）。
///   保底：keep_chars 是前缀字符预算，从头累加到跨过它的消息之前一律原样（保住前缀缓存）。
pub fn freshness_view(
    dialogue: &[Msg],
    reads: &[(&ReadAt, bool)],
    keep_chars: usize,
) -> (Vec<Msg>, FreshnessStats) {
    let mut stats = FreshnessStats::default();
    let mut msgs = dialogue.to_vec();
    // 保底前缀的边界：第一条可以替换的消息下标。0 = 不保底（从第一条起都能换）；
    // 预算盖住整段对话时停在末尾（一条都不换）。
    let mut boundary = 0usize;
    if keep_chars > 0 {
        boundary = msgs.len();
        let mut acc = 0usize;
        for (i, m) in msgs.iter().enumerate() {
            if acc >= keep_chars {
                boundary = i;
                break;
            }
            acc += m.content.chars().count();
        }
    }
    // 同一条工具结果消息可能带多段读（分段读、或同一版本被读过多次）：
    // 按消息归并，把该消息所有过期段合成一个占位符，只替换一次。
    type StaleGroup<'a> = (usize, &'a str, Vec<(u64, u64)>);
    let mut groups: Vec<StaleGroup<'_>> = Vec::new();
    for (r, stale) in reads {
        if !*stale {
            continue;
        }
        stats.judged += 1;
        let Some(idx) = msgs.iter().position(|m| m.content == r.body) else {
            continue;
        };
        if idx < boundary {
            continue;
        }
        match groups.iter_mut().find(|(g, _, _)| *g == idx) {
            Some((_, _, ranges)) => ranges.push((r.fact.from, r.fact.to)),
            None => groups.push((idx, r.fact.path.as_str(), vec![(r.fact.from, r.fact.to)])),
        }
    }
    groups.sort_by_key(|(idx, _, _)| *idx);
    let mut saved_chars = 0usize;
    for (idx, path, mut ranges) in groups {
        ranges.sort_unstable();
        let mut merged: Vec<(u64, u64)> = Vec::with_capacity(ranges.len());
        for (from, to) in ranges {
            match merged.last_mut() {
                Some(last) if from <= last.1.saturating_add(1) => last.1 = last.1.max(to),
                _ => merged.push((from, to)),
            }
        }
        let note = expired_note(path, &merged);
        saved_chars += msgs[idx]
            .content
            .chars()
            .count()
            .saturating_sub(note.chars().count());
        msgs[idx].content = note;
        stats.replaced += 1;
    }
    // 省得太少就不换：换一次要付一次前缀缓存失效，不值。
    if stats.replaced > 0 && saved_chars < FRESHNESS_MIN_SAVED_CHARS {
        return (
            dialogue.to_vec(),
            FreshnessStats {
                judged: stats.judged,
                replaced: 0,
                saved_tokens: 0,
                keep_percent: stats.keep_percent,
                skipped: true,
            },
        );
    }
    stats.saved_tokens = (saved_chars / 4) as u64;
    (msgs, stats)
}

/// 目的：摘要消息（发送视图里是 **user 角色**）：`compact` 与重建共用一份口径，实时与回放才逐条相同。
pub fn summary_message(summary: &str) -> Msg {
    Msg::user(format!("[此前内容摘要]\n{}", summary))
}

impl AgentSession {
    /// 目的：本会话的**对话**（测试据此断言"用户说过的话，下一回合带上了"）。
    ///   身份与环境不在里面——它们由 params 现渲染（见 SessionParams::identity）。
    #[cfg(test)]
    pub fn dialogue(&self) -> &[crate::units::business::llm::api::Msg] {
        &self.dialogue
    }

    /// 目的：会话参数：驱动据此现渲染身份块。
    pub fn params(&self) -> &SessionParams {
        &self.params
    }

    /// 目的：这一回合坐的席位 id（流程换段时由驱动按回合现换，跑完还原）。
    pub fn seat(&self) -> &str {
        &self.params.seat
    }

    /// 目的：换席——只在回合内由驱动调用，回合结束必须还原，否则下一回合会带着上一席的身份。
    pub(crate) fn set_seat(&mut self, seat: &str) {
        self.params.seat = seat.to_string();
    }

    /// 目的：**坐到另一席上**：身份与工具面同换（席位是回合的值，会话只是这一回合坐在它上面）。
    ///   工具面交给会话自己的装配机现拼（`MemberTools::install_seat`），这里不另写一份名单。
    /// 约束：会话没有工具环境时只换身份；席位没变时是 no-op。
    pub(crate) fn enter_seat(&mut self, seat: &str) {
        self.set_seat(seat);
        if let Some(tools) = self.tools.as_mut() {
            tools.install_seat(seat);
        }
    }

    /// 目的：改写要用的真实根：由参数派生，不另存一份。写法与工作环境块**同一座桥**（虚拟机档 = guest 写法）。
    pub(crate) fn roots(&self) -> crate::units::business::prompt::api::RefRoots {
        self.params.ref_roots()
    }

    // 组合根注入的构造函数：参数天然多，收口成参数对象只是把参数挪个地方、并让装配更难读。
    // 这是有意的设计取舍（见 docs/testing/quality-isolation.md 的 allow 清单），不是没修。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: &str,
        params: SessionParams,
        chat: BoxedChat,
        tools: Option<MemberTools>,
        compact_prompt: String,
    ) -> AgentSession {
        let mut s = AgentSession {
            cur_turn: 0,
            compact_at: 0,
            compact_tokens: 0,
            compact_percent: 0,
            compact_keep_percent: 0,
            context_window: 0,
            last_usage: None,
            usage_delta: UsageTotals::default(),
            compacted_upto: 0,
            compacted_line: 0,
            compact_factory: None,
            model: None,
            thinking: None,
            context_freshness: false,
            freshness_keep_percent: default_freshness_keep_percent(),
            freshness_stats: None,
            reads: Vec::new(),
            id: id.to_string(),
            params,
            dialogue: Vec::new(),
            chat,
            tools,
            compact_prompt,
            next_line: 0,
            marks: Vec::new(),
            line_reply: Vec::new(),
            line_ids: Vec::new(),
            cur_reply: 0,
        };
        s.set_next_line(0);
        s
    }

    /// 从落盘事件重建（继续/回档历史会话用）。
    // 组合根注入的构造函数：参数天然多，收口成参数对象只是把参数挪个地方、并让装配更难读。
    // 这是有意的设计取舍（见 docs/testing/quality-isolation.md 的 allow 清单），不是没修。
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        id: &str,
        params: SessionParams,
        dialogue: Vec<Msg>,
        marks: Vec<usize>,
        line_reply: Vec<u64>,
        line_ids: Vec<u64>,
        compacted_upto: u64,
        compacted_line: u64,
        chat: BoxedChat,
        tools: Option<MemberTools>,
        compact_prompt: String,
    ) -> AgentSession {
        let next = marks.len() as u64;
        let mut s = AgentSession {
            cur_turn: 0,
            compact_at: 0,
            compact_tokens: 0,
            compact_percent: 0,
            compact_keep_percent: 0,
            context_window: 0,
            last_usage: None,
            usage_delta: UsageTotals::default(),
            compacted_upto,
            compacted_line,
            compact_factory: None,
            model: None,
            thinking: None,
            context_freshness: false,
            freshness_keep_percent: default_freshness_keep_percent(),
            freshness_stats: None,
            reads: Vec::new(),
            id: id.to_string(),
            next_line: next,
            params,
            dialogue,
            chat,
            tools,
            compact_prompt,
            marks,
            line_reply,
            line_ids,
            cur_reply: 0,
        };
        s.set_next_line(next);
        s
    }

    /// 目的：这条会话**正在用**的工具调用形态（身份块里的调用约定按它现渲染）。
    pub fn tool_mode(&self) -> crate::units::pure::registry_model::api::ToolMode {
        self.tools.as_ref().map(|t| t.face.mode).unwrap_or_default()
    }

    /// 目的：改形态：**只改这一格**（登记处派生出来的参数），不重建会话。
    pub fn set_tool_mode(&mut self, mode: crate::units::pure::registry_model::api::ToolMode) {
        if let Some(t) = self.tools.as_mut() {
            t.face.mode = mode;
        }
    }

    /// 目的：写入本会话的思考参数片段（生成前的准备段按当前登记处解析）；None = 不发。
    /// 约束：只改这一格，不重建会话——压缩回合与下一轮对话都用最新的值。
    pub fn set_thinking(&mut self, thinking: Option<std::sync::Arc<serde_json::Value>>) {
        self.thinking = thinking;
    }

    /// 目的：本会话当前的思考参数片段（压缩等会话内部调用读它）。
    pub fn thinking(&self) -> Option<std::sync::Arc<serde_json::Value>> {
        self.thinking.clone()
    }

    /// 目的：记下本会话自己的模型 id（压缩判断“是否同模型”用）。
    pub fn set_model(&mut self, model: Option<String>) {
        self.model = model;
    }

    /// 目的：本会话自己的模型 id；None = 核心默认。
    pub fn model_id(&self) -> Option<&str> {
        self.model.as_deref()
    }

    /// 目的：按落盘转录续上回复 id（回放已有回复后，新回复不能与旧回复并成一组）。
    ///   约束：只有重建路径要它；没有工具面的会话（核心压缩）无回复号可续，因此是空操作。
    pub fn set_reply_seq(&mut self, seq: u64) {
        if let Some(t) = self.tools.as_mut() {
            t.reply_seq = seq;
        }
    }

    /// 目的：按落盘转录回填观察账本（重启后的写门禁仍看得到活动窗口里的读）。
    /// 约束：只给重建路径；没有工具面的会话（核心压缩）无账本可填，因此是空操作。
    pub fn restore_observations(
        &mut self,
        facts: Vec<crate::units::business::tools::api::ObservationFact>,
    ) {
        if let Some(t) = self.tools.as_mut() {
            t.observations.restore_facts(facts);
        }
    }

    /// 目的：过期内容自动清理的开关（用户在界面上控制成本；落盘在 meta）。
    pub fn set_context_freshness(&mut self, on: bool) {
        self.context_freshness = on;
    }

    /// 目的：本会话是否开着过期清理。
    pub fn context_freshness(&self) -> bool {
        self.context_freshness
    }

    /// 目的：过期内容保底百分比（0 = 不保底）：由工作根 meta 派生，界面只读。
    pub fn set_freshness_keep_percent(&mut self, percent: u8) {
        self.freshness_keep_percent = percent;
    }

    /// 目的：本会话的保底百分比。
    pub fn freshness_keep_percent(&self) -> u8 {
        self.freshness_keep_percent
    }

    /// 目的：最近一次发送视图的替换统计（面板读；没发过时为 None）。
    pub fn freshness_stats(&self) -> Option<FreshnessStats> {
        self.freshness_stats
    }

    /// 目的：写回这一次发送视图的统计（只由驱动在发请求前调）。
    pub(crate) fn set_freshness_stats(&mut self, stats: Option<FreshnessStats>) {
        self.freshness_stats = stats;
    }

    /// 目的：记下"读过这个范围"（发送视图据此判断过期）。
    /// 约束：body 必须是那条工具结果消息的**正文**——发送时按它定位消息，压缩移位后照样找得到。
    ///   同一次读（工作树、路径、版本、行区间都相同）只留一条：重建会重放全部历史，
    ///   同一标的被读过多次时以最新那条正文与回复为准（最可能还在发送视图里）。
    pub fn note_read_at(
        &mut self,
        body: String,
        reply: u64,
        fact: crate::units::business::workspace::api::ReadFact,
    ) {
        let same = |r: &ReadAt| {
            r.fact.worktree == fact.worktree
                && r.fact.path == fact.path
                && r.fact.blob == fact.blob
                && r.fact.from == fact.from
                && r.fact.to == fact.to
        };
        if let Some(r) = self.reads.iter_mut().find(|r| same(r)) {
            r.body = body;
            r.reply = reply;
            return;
        }
        self.reads.push(ReadAt { body, reply, fact });
    }

    /// 目的：本会话的读台账（发送视图与重建共用）。
    pub fn reads(&self) -> &[ReadAt] {
        &self.reads
    }

    /// 目的：在压缩触发时现建总结通道；None = 会话同模型。
    pub fn compact_chat(&self) -> Result<Option<CompactChatSpec>, String> {
        match self.compact_factory.as_ref() {
            Some(f) => f.build(&self.id, self.model.as_deref()),
            None => Ok(None),
        }
    }

    /// 目的：注入压缩通道工厂（组合根提供；触发时现建，设置改了也不用刷新会话）。
    pub fn set_compact_factory(&mut self, factory: std::sync::Arc<dyn CompactChatFactory>) {
        self.compact_factory = Some(factory);
    }

    /// 目的：注入一条**系统消息**（提醒/边界这类"不是用户说的、也不是模型说的"内容）：
    ///   上下文里是 system 角色，转录里是系统行。
    ///   它**不得在界面与记录里长得像用户发的**——身份是核心，界面按系统行样式（见 session-model.md 二）。
    pub fn note_system(&mut self, text: &str) -> Vec<SessionEvent> {
        self.dialogue.push(Msg::system(text.to_string()));
        let v = self.line(text.to_string(), "system", "", "", None, None);
        vec![SessionEvent::Transcript(vec![v])]
    }

    /// 目的：注入一条**内容类系统提示**——转录里是一等行（有 id、可回档/折叠、能删除），**不进模型上下文**。
    /// 约束：自己成一条回复（reply = 自己的 id），回档到它按回复原子截断。
    pub fn note_notice(&mut self, text: &str) -> Vec<SessionEvent> {
        let v = self.notice_line(text);
        vec![SessionEvent::Transcript(vec![v])]
    }

    /// 目的：分配一条**控制行**（id / marks / line_reply / line_ids 与普通行同一套账），不碰对话。
    /// 约束：控制行不产生对话消息；回档按普通行原子截断。
    fn control_line(
        &mut self,
        kind: &str,
        text: String,
        system: bool,
        payload: Option<serde_json::Value>,
    ) -> LineView {
        let id = self.next_line;
        let v = LineView {
            id,
            reply: id,
            line: text,
            kind: kind.to_string(),
            system,
            payload,
            turn: self.cur_turn,
            ..Default::default()
        };
        self.next_line += 1;
        if let Some(t) = self.tools.as_ref() {
            t.line
                .store(self.next_line, std::sync::atomic::Ordering::Relaxed);
        }
        self.line_reply.push(id);
        self.line_ids.push(id);
        self.marks.push(self.dialogue.len());
        v
    }

    /// 目的：分配一条 notice 行（内容类系统提示；不碰对话）。
    fn notice_line(&mut self, text: &str) -> LineView {
        self.control_line("notice", text.to_string(), false, None)
    }

    /// 目的：在发送视图压缩后落一条**压缩记录行**：正文 = 摘要，payload.up_to = 第一行保留行。
    /// 约束：它不产生对话消息；删除/留档它本身 = 解除这次压缩。
    pub fn note_compacted(&mut self, up_to: u64, summary: &str) -> LineView {
        let v = self.control_line(
            "compacted",
            summary.to_string(),
            true,
            Some(serde_json::json!({ "up_to": up_to })),
        );
        self.compacted_upto = up_to;
        self.compacted_line = v.id;
        v
    }

    /// 目的：注入一条**派发任务**（核心给这个 agent 派的活，见 session-model.md 四之二）。
    ///   界面：**系统行**——说话人是核心，不冒充用户；上下文：**user 角色**——派活是一次"回合"，
    ///   会话协议要求请求里至少有一条 user 消息（全是 system 的请求会被供应商整条拒收）。
    ///   两处口径随行落档（`system` + `task`），重建/回放才产得出**同一条**消息。
    pub fn note_task(&mut self, text: &str) -> Vec<SessionEvent> {
        self.dialogue.push(Msg::user(text.to_string()));
        // 派发行不属于任何模型回复：给它自己的行号当回复号（与重建规则一致）。
        self.cur_reply = self.next_line;
        let v = self.line(text.to_string(), "system", "", "", None, None);
        vec![SessionEvent::Transcript(vec![LineView { task: true, ..v }])]
    }

    /// 目的：设本会话的上下文预算：窗口、压缩阈值百分比（0 = 关）、保留百分比与字符兜底一起算好。
    /// 约束：装配、重建、改模型都走这一处；last_usage 已有时把窗口换成当前模型的窗口。
    pub fn set_context_budget(&mut self, window: u64, percent: u8, keep_percent: u8) {
        self.context_window = window;
        self.compact_percent = percent;
        self.compact_keep_percent = keep_percent;
        let pct = percent as u64;
        self.compact_tokens = window * pct / 100;
        self.compact_at = (self.compact_tokens * 4) as usize;
        if let Some(u) = self.last_usage.as_mut() {
            u.window = window;
        }
    }

    /// 目的：本会话的上下文窗口（token）；0 = 未配置（工具预算不收紧）。
    pub fn context_window(&self) -> u64 {
        self.context_window
    }

    /// 目的：最近一次调用的用量（状态栏与压缩判定读它）。
    pub fn context_usage(&self) -> Option<crate::units::business::transcript::api::ContextUsage> {
        self.last_usage
    }

    /// 目的：写入最近一次调用的用量；窗口以当前模型为准（模型改过也显示同一个窗口）。
    pub fn set_last_usage(
        &mut self,
        mut usage: Option<crate::units::business::transcript::api::ContextUsage>,
    ) {
        if let Some(u) = usage.as_mut()
            && self.context_window > 0
        {
            u.window = self.context_window;
        }
        self.last_usage = usage;
    }

    /// 目的：记一次**供应商精确报回**的调用用量（回合内每轮、压缩调用都算），供累计写进 meta。
    /// 约束：估算的用量不要走这里——账面只记真实花费。
    pub(crate) fn note_usage(&mut self, usage: TokenUsage) {
        self.usage_delta.calls += 1;
        self.usage_delta.prompt += usage.prompt_tokens;
        self.usage_delta.completion += usage.completion_tokens;
        self.usage_delta.cache_read += usage.cache_read.unwrap_or(0);
        self.usage_delta.cache_creation += usage.cache_creation.unwrap_or(0);
    }

    /// 目的：取走本次生成累计的用量增量并清零（核心线程在会话交回 / 落盘时并进 meta）。
    /// 返回：还没记过任何调用时是全零（`UsageTotals::is_empty`）。
    pub(crate) fn take_usage_delta(&mut self) -> UsageTotals {
        std::mem::take(&mut self.usage_delta)
    }

    /// 目的：发送视图的字符数（拿不到供应商 usage 时的压缩兜底）。
    pub fn dialogue_chars(&self) -> usize {
        self.dialogue
            .iter()
            .map(|m| m.content.chars().count())
            .sum()
    }

    /// 目的：按当前发送视图估一份上下文用量（没有供应商 usage 时用；界面会标"估算"）。
    pub fn estimate_usage(&self) -> crate::units::business::transcript::api::ContextUsage {
        crate::units::business::transcript::api::ContextUsage::estimated(
            self.dialogue_chars(),
            self.context_window,
        )
    }

    /// 目的：算这次压缩的切分点：保留段从哪一行开始（行 id + 对话下标）。
    /// 参数：`protect_from` 是本轮新注入的用户行 / 任务行的 id——它必须原样保留，不能被压进摘要。
    /// 约束：从尾部累计估算 token（字符 ÷ 4）到保留预算；切分点对齐完整回复组。`keep = 0` = 全压。
    pub fn plan_compaction(&self, protect_from: Option<u64>) -> Option<CompactionPlan> {
        if self.dialogue.is_empty() || self.marks.is_empty() {
            return None;
        }
        if self.line_ids.len() != self.marks.len() {
            return None;
        }
        let full = || CompactionPlan {
            up_to: self.next_line,
            cut: self.dialogue.len(),
        };
        let keep = self.compact_keep_percent;
        if keep == 0 || self.context_window == 0 {
            // 只留摘要也必须保护本轮新输入：切在它前面，把它留在尾部。
            if let Some(id) = protect_from
                && let Some(k) = self.line_ids.iter().position(|x| *x == id)
            {
                if k == 0 {
                    return None;
                }
                let cut = self.marks[k - 1];
                if cut == 0 || cut >= self.dialogue.len() {
                    return None;
                }
                if self.compacted_upto > 0 && cut <= 1 {
                    return None; // 前缀里只剩上一份摘要：再压也不会变小。
                }
                return Some(CompactionPlan {
                    up_to: self.line_ids[k],
                    cut,
                });
            }
            return Some(full());
        }
        let budget_tokens = self.context_window.saturating_mul(keep as u64) / 100;
        let budget_chars = budget_tokens.saturating_mul(4).min(usize::MAX as u64) as usize;
        if budget_chars == 0 {
            return Some(full());
        }
        // prefix[i] = dialogue[..i] 的字符数；行 i 的字符数 = prefix[marks[i]] - prefix[marks[i-1]]。
        let mut prefix = Vec::with_capacity(self.dialogue.len() + 1);
        prefix.push(0usize);
        let mut acc = 0usize;
        for m in &self.dialogue {
            acc += m.content.chars().count();
            prefix.push(acc);
        }
        let line_chars = |i: usize| {
            let start = if i == 0 { 0 } else { self.marks[i - 1] };
            let end = self.marks[i];
            prefix[end].saturating_sub(prefix[start])
        };
        let rows = self.marks.len();
        let mut split = rows;
        let mut tail = 0usize;
        for i in (0..rows).rev() {
            tail += line_chars(i);
            split = i;
            if tail >= budget_chars {
                break;
            }
        }
        if split == 0 {
            return None; // 整段都在保留预算内：没有更早的内容可压。
        }
        // 不切开一次回复：切分点向前退到该回复组的第一行（保留量只会更多）。
        while split > 0 && self.line_reply[split] == self.line_reply[split - 1] {
            split -= 1;
        }
        // 本轮新注入的用户行 / 任务行必须原样保留。
        if let Some(id) = protect_from
            && let Some(k) = self.line_ids.iter().position(|x| *x == id)
        {
            split = split.min(k);
        }
        if split == 0 {
            return None;
        }
        let cut = self.marks[split - 1];
        if cut == 0 || cut >= self.dialogue.len() {
            return None;
        }
        if self.compacted_upto > 0 && cut <= 1 {
            return None; // 前缀里只剩上一份摘要：再压也不会变小。
        }
        Some(CompactionPlan {
            up_to: self.line_ids[split],
            cut,
        })
    }

    /// 目的：把 `plan.cut` 之前的内容移出**发送视图**，用一份摘要代替，并落一条压缩记录行。
    /// 返回：切分点无效（0）时 `None`；否则 `Some(记录行)`。
    /// 约束：改的是"发给模型什么"，不是"留下什么"；见 docs/session/session-model.md 六。
    pub fn apply_compaction(&mut self, plan: &CompactionPlan, summary: &str) -> Option<LineView> {
        if plan.cut == 0 {
            return None;
        }
        let cut = plan.cut.min(self.dialogue.len());
        self.dialogue.drain(0..cut);
        // 摘要放在**对话最前面**（身份与环境由参数现渲染，不占对话的位置）：
        // 此后模型只看到"身份 + 摘要 + 之后保留的内容"。
        self.dialogue.insert(0, summary_message(summary));
        for m in self.marks.iter_mut() {
            *m = m.saturating_sub(cut) + 1;
        }
        // 被压进摘要的那段工具行不再发给模型：它们的观察证据与读台账一并作废（重读比误改便宜）。
        if let Some(t) = self.tools.as_mut() {
            t.observations.retain_from(plan.up_to);
        }
        self.reads.retain(|r| r.reply >= plan.up_to);
        Some(self.note_compacted(plan.up_to, summary))
    }

    /// 目的：给接下来的行打上回合 id（节点执行也用整场工作的同一套计数：回档才对得上）。
    pub fn set_turn(&mut self, turn_id: u64) {
        self.cur_turn = turn_id;
    }

    /// 目的：回档重建后把行号与**工具行锚**一起推到"全量下一个 id"（永不回退）。
    pub fn set_next_line(&mut self, next: u64) {
        self.next_line = next;
        if let Some(t) = self.tools.as_ref() {
            t.line.store(next, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// 目的：生成一条转录行，并记下它完成时的历史长度（回档按 marks 逐行精确回退）与它属于哪次回复。
    pub(crate) fn line(
        &mut self,
        line: String,
        kind: &str,
        speaker: &str,
        verb: &str,
        reasoning: Option<String>,
        tool: Option<ToolCallView>,
    ) -> LineView {
        let reply = self.cur_reply;
        let v = LineView {
            id: self.next_line,
            reply,
            line,
            speaker: speaker.to_string(),
            verb: verb.to_string(),
            kind: kind.to_string(),
            reasoning,
            raw: String::new(),
            tool,
            degraded: false,
            cut: false,
            system: kind == "system",
            task: false,
            turn: self.cur_turn,
            payload: None,
        };
        self.next_line += 1;
        if let Some(t) = self.tools.as_ref() {
            t.line
                .store(self.next_line, std::sync::atomic::Ordering::Relaxed);
        }
        self.line_reply.push(reply);
        self.line_ids.push(v.id);
        self.marks.push(self.dialogue.len());
        v
    }

    /// 目的：末条是否为用户发言（继续能不能直接发请求的判据）。
    pub fn last_is_user(&self) -> bool {
        matches!(self.dialogue.last().map(|m| m.role.as_str()), Some("user"))
    }

    /// 目的：**删除模式**的精确回退（不重建，保住通道）。
    /// 参数：`keep_id` = 只保留前 keep_id 行（= 删该行及其后）；0 = 转录清空、对话清空。
    /// 约束：**按回复原子**截断（不留下孤儿工具结果）；`line_reply`、`marks`、`dialogue` 与
    ///   工具行锚必须同步回退——`keep_whole_replies` 依赖 `line_reply` 与 `marks` 逐行一一对应。
    pub fn apply_rewind(&mut self, keep_id: u64) {
        // 转录里那段"我完整读过哪些文件"的证据随删除作废（保守，宁肯让模型重读）。
        if let Some(t) = self.tools.as_mut() {
            t.observations.clear();
        }
        self.reads.clear();
        let rows = self.marks.len();
        self.line_reply.truncate(rows);
        self.line_ids.truncate(rows);
        let keep = if keep_id == u64::MAX {
            // `align_keep` 的「不截」语义：保留全部，不能拿 line_reply 的长度当行数。
            rows
        } else {
            keep_whole_replies(&self.line_reply, keep_id as usize).min(rows)
        };
        self.line_reply.truncate(keep);
        self.line_ids.truncate(keep);
        if keep == 0 {
            self.marks.clear();
            self.dialogue.clear();
            self.set_next_line(0);
            return;
        }
        if keep == rows {
            // 「保留全部」：内容不动；line_reply 已在上面与真实行数对齐。
            return;
        }
        let hist = self.marks[keep - 1];
        self.marks.truncate(keep);
        self.dialogue.truncate(hist);
        // 工具行锚也是派生状态：必须走 set_next_line 更新，否则回档后第一轮工具调用
        // （如 work_commit）仍会按旧行号写共享区提交锚。
        self.set_next_line(keep as u64);
    }
}

/// 目的：流式外送规则：信封之前照常外送，一旦累积文本里出现 "{" 就不再外送后续片段
///   （模型可能在同一轮里先写正文再发 tool 信封——信封绝不能当正文流上屏）。
///   返回（本片可外送的部分, 新的累积文本）。纯函数，便于单测。
pub fn stream_piece(acc: &str, piece: &str) -> (String, String) {
    let send = if acc.contains('{') {
        String::new()
    } else {
        match piece.find('{') {
            Some(i) => piece[..i].to_string(),
            None => piece.to_string(),
        }
    };
    (send, format!("{}{}", acc, piece))
}

/// 目的：这一片要不要外送成 Delta 事件——`start` 要（前端据此重置本轮流），**空片段不要**
///   （信封被抑制的片段、被合并掉的重复片段都不该刷事件流）。
/// 约束：与 `stream_piece` 同处一条流式规则；只认 kind 与文本，不判断内容。
pub fn delta_visible(kind: &str, text: &str) -> bool {
    kind == "start" || !text.is_empty()
}

/// 目的：工作名的缺省与唯一化：`base` 去空白；为空用 `fallback`；重名追加 `-2` / `-3`（进程内唯一即可）。
///   **纯策略**：`exists` 由调用方给——会话存在性归存储，本函数不关心它从哪来。
///   归属：命名是**会话自己的策略**（它拥有会话身份），不是前端的便利函数。
/// 约束：缺省与去空白是本函数的事；后缀探测是内核的名字契约（`kernel::api::name::dedup_suffix`）。
pub fn unique_work_name(base: &str, fallback: &str, exists: impl Fn(&str) -> bool) -> String {
    let seed = if base.trim().is_empty() {
        fallback
    } else {
        base.trim()
    };
    crate::kernel::api::name::dedup_suffix(seed, exists)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact_with(from: u64, to: u64) -> crate::units::business::workspace::api::ReadFact {
        crate::units::business::workspace::api::ReadFact {
            worktree: String::new(),
            path: "docs/README.md".to_string(),
            from,
            to,
            blob: "abc".to_string(),
        }
    }

    fn fact() -> crate::units::business::workspace::api::ReadFact {
        fact_with(3, 9)
    }

    /// 目的：只装读台账的会话——不接工具、不发请求，用来验台账的作废时机。
    fn ledger_session() -> AgentSession {
        let params = SessionParams {
            agent: "a".to_string(),
            work_name: "w".to_string(),
            shared: std::path::PathBuf::from("shared"),
            private: std::path::PathBuf::from("private"),
            module_dirs: std::collections::BTreeMap::new(),
            chara: String::new(),
            modules: Vec::new(),
            seat_system: None,
            seat: "solo".to_string(),
            guest: None,
        };
        AgentSession::new(
            "s",
            params,
            Box::new(crate::units::business::llm::detail::fake_chat::FakeChat::new(Vec::new())),
            None,
            String::new(),
        )
    }

    /// 目的：读台账与观察账本同水位——同版本同区间去重，压缩丢旧行，回档全清。
    #[test]
    fn read_ledger_follows_compaction_and_rewind() {
        let mut s = ledger_session();
        s.dialogue.push(Msg::user("旧内容".to_string()));
        s.dialogue.push(Msg::user("新内容".to_string()));
        s.note_read_at("旧".to_string(), 3, fact_with(1, 2));
        s.note_read_at("新".to_string(), 9, fact_with(3, 9));
        s.note_read_at("新（重读）".to_string(), 11, fact_with(3, 9));
        assert_eq!(s.reads().len(), 2, "同版本同区间再读只更新那一条");
        assert_eq!(s.reads()[1].body, "新（重读）");
        assert_eq!(s.reads()[1].reply, 11, "记最新的行号");

        s.apply_compaction(&CompactionPlan { up_to: 5, cut: 1 }, "摘要");
        assert_eq!(s.reads().len(), 1, "压缩点之前读完的随摘要作废");
        assert_eq!(s.reads()[0].reply, 11);

        s.apply_rewind(0);
        assert!(s.reads().is_empty(), "回档清空读台账");
    }

    // 省够门槛才有替换：正文得够长，短正文会整体回退（见 FRESHNESS_MIN_SAVED_CHARS）。
    fn long_body(tag: &str) -> String {
        format!("[工具结果] read\n{}", tag.repeat(3000))
    }

    #[test]
    fn freshness_view_replaces_only_the_stale_tool_result() {
        let stale = Msg::user(long_body("old "));
        let fresh = Msg::user(long_body("new "));
        let dialogue = vec![Msg::user("hi"), stale.clone(), fresh.clone()];
        let r1 = ReadAt {
            body: stale.content.clone(),
            reply: 1,
            fact: fact(),
        };
        let r2 = ReadAt {
            body: fresh.content.clone(),
            reply: 1,
            fact: fact(),
        };
        let (out, stats) = freshness_view(&dialogue, &[(&r1, true), (&r2, false)], 0);
        assert_eq!(out.len(), 3);
        assert_eq!(
            out[1].content,
            "[已过期：docs/README.md 第3-9行，读取后文件已修改；需要时重新 read]"
        );
        assert_eq!(out[2].content, fresh.content);
        assert_eq!(stats.judged, 1);
        assert_eq!(stats.replaced, 1);
        assert!(stats.saved_tokens > 2000);
        assert!(!stats.skipped);
    }

    #[test]
    fn freshness_view_merges_the_segments_of_one_tool_result() {
        // 同一条工具结果里的多段读：相邻/重叠的段合并，隔开的段用顿号分开，只替换一次。
        let body = Msg::user(long_body("old "));
        let r1 = ReadAt {
            body: body.content.clone(),
            reply: 1,
            fact: fact_with(3, 5),
        };
        let r2 = ReadAt {
            body: body.content.clone(),
            reply: 1,
            fact: fact_with(6, 9),
        };
        let (out, stats) =
            freshness_view(std::slice::from_ref(&body), &[(&r1, true), (&r2, true)], 0);
        assert_eq!(
            out[0].content,
            "[已过期：docs/README.md 第3-9行，读取后文件已修改；需要时重新 read]"
        );
        assert_eq!(stats.judged, 2);
        assert_eq!(stats.replaced, 1);

        let r3 = ReadAt {
            body: body.content.clone(),
            reply: 1,
            fact: fact_with(3, 5),
        };
        let r4 = ReadAt {
            body: body.content.clone(),
            reply: 1,
            fact: fact_with(8, 9),
        };
        let (out, _) = freshness_view(std::slice::from_ref(&body), &[(&r3, true), (&r4, true)], 0);
        assert_eq!(
            out[0].content,
            "[已过期：docs/README.md 第3-5、8-9行，读取后文件已修改；需要时重新 read]"
        );
    }

    #[test]
    fn freshness_view_skips_a_body_that_compaction_removed() {
        let r = ReadAt {
            body: "gone".to_string(),
            reply: 1,
            fact: fact(),
        };
        let (out, stats) = freshness_view(&[Msg::user("hi")], &[(&r, true)], 0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].content, "hi");
        assert_eq!(stats.judged, 1);
        assert_eq!(stats.replaced, 0);
    }

    #[test]
    fn freshness_view_keeps_the_floor_prefix_untouched() {
        let stale = Msg::user(long_body("old "));
        let fresh = Msg::user(long_body("new "));
        let dialogue = vec![Msg::user("hi"), stale.clone(), fresh.clone()];
        let r = ReadAt {
            body: stale.content.clone(),
            reply: 1,
            fact: fact(),
        };
        // 保底预算盖住整段对话：一条都不换。
        let keep = dialogue
            .iter()
            .map(|m| m.content.chars().count())
            .sum::<usize>();
        let (out, stats) = freshness_view(&dialogue, &[(&r, true)], keep);
        assert_eq!(out[1].content, stale.content);
        assert_eq!(stats.replaced, 0);
    }

    #[test]
    fn freshness_view_skips_when_savings_do_not_pay_the_cache_loss() {
        let stale = Msg::user("[工具结果] read\nold text");
        let r = ReadAt {
            body: stale.content.clone(),
            reply: 1,
            fact: fact(),
        };
        let dialogue = vec![stale.clone()];
        let (out, stats) = freshness_view(&dialogue, &[(&r, true)], 0);
        assert_eq!(out[0].content, stale.content);
        assert_eq!(stats.judged, 1);
        assert_eq!(stats.replaced, 0);
        assert!(stats.skipped);
    }
}
