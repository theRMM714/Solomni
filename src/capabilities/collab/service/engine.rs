//! 协作引擎：建组 → 讨论 → 整理 → 执行 → 验收（纯状态机，不做输入输出）。
//! 状态机只认信封动词；发言内容永远是数据，不是指令。
//! 所有发给模型的文案经 capabilities/prompt 渲染自提示词册；成员拥有自己的会话通道。
//! 工具循环（联动 envelope::Verb::Tool 与 ports::ToolRunner）：策略（放行表、并发调度）在核心，机制在适配层。
//! 一次回复里的多个原生调用按**声明**调度：声明可并发的只读类并发跑，其余（含写入类）独占并按原序生效；
//! 结果与工具行一律按原始顺序回填——并发只影响执行，不影响上下文里的顺序。
use crate::capabilities::collab::service::tool_loop::*;

#[cfg(test)]
use crate::capabilities::llm::api::BoxedChat;
use crate::capabilities::llm::api::{self as envelope, Verb};
use crate::capabilities::llm::api::{Chat, Chunk, CompleteOpts, Msg};
use crate::capabilities::prompt::api::{Prompt, Segment};
use crate::capabilities::session::api::MemberTools;
use crate::capabilities::session::api::{reply_msgs, LineView, SessionEvent, ToolCallView};
use crate::capabilities::tools::api::ToolOutcome;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::Arc;

/// 讨论轮次上限（超限交用户裁决——上限必生效）。
pub const MAX_ROUNDS: usize = 6;
/// 一轮内对同一个成员最多提醒几次（**内存驱动**用；生产按设置走）。
#[cfg(test)]
pub const MAX_DISCUSS_REMIND: u32 = 3;

/// 一次成员回合之后的处置（核心据此决定，见 docs/architecture/session-model.md 二）。
#[derive(Debug, PartialEq)]
pub enum AfterTurn {
    /// 已表态：正常往下走。
    Done,
    /// 没表态但还有提醒额度：**注入提醒后重问同一个人**（核心只提醒、不强制）。
    Remind,
    /// 没表态且提醒到顶：主会话记一行"未回应"，本轮放过它（**不阻塞整轮**）。
    Unanswered,
}

/// 线上名 → (模块 id, 工具名)：原生协议里没有 module 字段，跨模块同名工具靠它消歧。
pub type WireTools = BTreeMap<String, (Option<String>, String)>;

/// 一次请求要声明的工具表（给供应商的那一份）。
pub type Decls = Vec<crate::capabilities::llm::api::ToolDecl>;

/// 本成员这次请求的**工具声明面**（原生通道用）：发给供应商的声明 + 线上名回译表 + 可并发的线上名。
#[derive(Default)]
pub(crate) struct ToolDecls {
    pub(crate) list: Decls,
    /// 线上名 → (模块 id, 工具名)：原生协议里没有 module 字段，跨模块同名工具靠它消歧。
    pub(crate) wire: WireTools,
}

pub struct Member {
    pub id: String,
    /// 会话参数：身份块由它**每回合现渲染**（不给成员存一份渲染好的文本）。
    pub params: crate::capabilities::session::api::SessionParams,
    /// 该席位的通道形态（登记处派生）：身份块里的调用约定按它渲染。
    pub mode: crate::capabilities::llm::api::ToolMode,
    /// 内存通道：**只有测试用**（生产里回合跑在各自的 agent 会话里，见 session-model.md 二之二）。
    #[cfg(test)]
    pub chat: Option<BoxedChat>,
    pub present: bool,
    pub agreed: bool,
    /// 工具环境；None = 本模块未声明工具（tool 信封按原文收录）。
    pub tools: Option<MemberTools>,
}

impl Member {
    /// 生产构造：成员不持有通道（驱动权在核心，回合在各自的会话里跑）。
    pub fn plain(
        id: &str,
        params: crate::capabilities::session::api::SessionParams,
        mode: crate::capabilities::llm::api::ToolMode,
    ) -> Member {
        Member {
            id: id.to_string(),
            params,
            mode,
            present: true,
            agreed: false,
            tools: None,
            #[cfg(test)]
            chat: None,
        }
    }

    /// 测试构造：带内存通道（配合 cfg(test) 的 open/step/member_turn 直接驱动讨论）。
    #[cfg(test)]
    pub fn new(
        id: &str,
        params: crate::capabilities::session::api::SessionParams,
        mode: crate::capabilities::llm::api::ToolMode,
        chat: BoxedChat,
    ) -> Member {
        Member {
            id: id.to_string(),
            params,
            mode,
            present: true,
            agreed: false,
            tools: None,
            chat: Some(chat),
        }
    }
}

pub enum TurnOut {
    /// 一轮正常走完，转达给用户过目。
    Round,
    /// 调用失败（超时 / 网络）：本轮**中断**——不把失败当发言吸收，交给用户决定何时继续。
    Interrupted(String),
    /// 用户点了「停止」：本轮**停止**——被中断的那条发言**不吸收**（半截发言进转录会把状态算歪）。
    Stopped,
    /// 有模块请教用户：轮转中止，等用户回答。
    AskUser { member: String, question: String },
    /// 留在组的成员全部同意 → 讨论终止。
    Done,
}

pub struct Discussion {
    pub members: Vec<Member>,
    /// 讨论转录：**与单 agent 同一行类型**（LineView）——行格式只有一处定义，呈现层也只有一套渲染。
    pub transcript: Vec<LineView>,
    pub round: usize,
    /// 用户对 ask 的回答在此队列：先入先转达。
    pub pending_user_answers: Vec<String>,
    pub closed: bool,
    /// yes,allow：授权小组自裁——ask 不中止轮转，留档待办。
    pub allow_autonomy: bool,
    /// 提示词册能力面：**不是册子本体**（持有者只有提示词能力一处），这里只按名字取段。
    prompts: Arc<dyn Prompt>,
    /// 工具总表与角色表的能力面（**表本体在工具能力里**）。
    systools: Arc<dyn crate::capabilities::tools::api::Tools>,
    /// 本次调用的通道参数（流式 + 预算）：**全局设置**，与单 agent 共用同一份。
    llm: crate::capabilities::llm::api::LlmOpts,
    /// 讨论席的**机制说明 + 讨论约定**（开场与轮转都带它）：只说约定不说机制，AI 会空转。
    /// 能用哪些工具**不在这里**——随回合注入（见 MemberTools::tools_block）。
    protocol: String,

    /// 这一轮对每个成员**提醒过几次**（到顶就记"未回应"放过它）；每轮开始归零。
    reminded: std::collections::BTreeMap<String, u32>,
    /// 上面那份计数属于第几轮（轮次变了就清空）。
    remind_round: usize,
    /// 「停止」标志：由 CollabSession 注入（它从任务登记处拿到）。
    /// 泵在**每次调用前**与**调用中途**都看它——所以停止能在一个模型调用内收尾，而不是等它跑完。
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 开场提示词（渲染一次，开场阶段每个成员都用它）——由 start() 设。
    opener: String,
    /// 轮次推进的游标（可暂停：问到哪了）——见 advance/feed。
    cursor: Cursor,
    /// 本轮到此刻**还没交出去**的行数（轮次标记也算）：一个成员说完就把它那一批交出去。
    handed: usize,
}

/// 推进讨论的**一步**：该问谁（或终态）。
/// 泵只决定"该问谁"，**不自己调模型**——驱动权归核心（见 docs/architecture/session-model.md 二之二）。
pub enum Adv {
    /// 该问这个成员一回合：身份块 + 本回合提示交给驱动（它取会话、装配、跑模型）。
    /// 身份**每回合现渲染**（由成员的 params + 当前提示词册），不存在谁的会话里。
    Ask {
        i: usize,
        identity: String,
        turn: Vec<Msg>,
    },
    /// 开场问完了（可以进轮次了）。
    Opened,
    /// 终态 / 轮次结束（原样交回上层）。
    Out(TurnOut),
}

/// 轮次推进的游标（状态机可暂停：问到哪了）。
enum Cursor {
    /// 开场：问到第 i 个成员（开场不跳过任何人）。
    Opener(usize),
    /// 本轮还没开始：下一次 advance 写轮次标记与用户回答，再从第一个人问起。
    Fresh,
    /// 本轮问到第 i 个成员。
    At(usize),
}

/// 一个成员回合的结果：两套通道**统一形态**（上层不必再关心是哪条通道）。
pub struct MemberTurn {
    /// 这一回合的表态；**None = 没表态**（散文 / 空回执 / 越权 / 到顶）。
    /// 没表态**不投影主会话**：原文只落进它自己的会话；提醒由核心在边界注入。
    pub verb: Option<Verb>,
    pub text: String,
    pub degraded: bool,
    /// 这一轮的输出被供应商按长度截断了（如实标注，不假装完整）。
    pub truncated: bool,
    /// 这一回合里**核实**留下的行（只读工具调用）。
    /// 内存通道把它并进讨论转录；核心驱动时由核心逐轮落进**该 agent 自己的会话**。
    #[cfg(test)]
    pub lines: Vec<LineView>,
}

impl MemberTurn {
    /// 一回合的**结论**：表态 / 正文 + 两个如实标记。
    /// 思维链与核实行不在这里——它们随行落进**各自的会话**（内存测试通道另走它自己的构造点）。
    pub fn verdict(
        verb: Option<Verb>,
        text: String,
        degraded: bool,
        truncated: bool,
    ) -> MemberTurn {
        MemberTurn {
            verb,
            text,
            degraded,
            truncated,
            #[cfg(test)]
            lines: Vec::new(),
        }
    }
}

impl Discussion {
    /// 内存通道的成员回合（薄包装）：借用本席位自己的 chat/tools，把核实行并进讨论转录。
    /// 核心驱动那条路直接调 turn_with，把核实行落进该 agent 自己的会话。
    #[cfg(test)]
    fn member_turn(
        &mut self,
        i: usize,
        identity: &str,
        turn: Vec<Msg>,
        sink: &mut dyn FnMut(SessionEvent),
    ) -> Result<MemberTurn, String> {
        let opts = self.opts();
        let texts = self.prompts.tools();
        let speaker = self.members[i].id.clone();
        let turn = {
            let m = &mut self.members[i];
            let Member { chat, tools, .. } = m;
            let chat = chat.as_mut().expect("测试通道");
            Self::turn_with(
                &*self.systools,
                "discussant",
                &self.cancel,
                opts,
                &speaker,
                identity,
                &[],
                chat.as_mut(),
                tools.as_mut(),
                turn,
                &texts,
                sink,
            )?
        };
        self.transcript.extend(turn.lines.clone());
        Ok(turn)
    }

    /// 一个成员回合（**关联函数**：chat/tools 由调用方给）：**薄适配**——把本席位自己的通道交给
    /// 单 agent / 节点 / 讨论席**共用的那一条轮循环**（engine::converse_with），装配前把这一回合的
    /// 工具面（角色表发放）装进工具环境，跑完从末轮取表态。
    /// 生产里成员回合跑在各自的 agent 会话里（crate::capabilities::session::api::AgentSession::discussion_turn）——两条路同一条循环。
    /// 允许**先核实**（只读工具），最后用**动词**表态；其余工具一律**如实拒绝**（讨论回合拿不到干活的手段）。
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn turn_with(
        systools: &dyn crate::capabilities::tools::api::Tools,
        role: &str,
        cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
        opts: crate::capabilities::llm::api::CompleteOpts<'static>,
        speaker: &str,
        // 本回合的身份块（驱动按当前提示词册现渲染；不进对话）。
        identity: &str,
        // 该 agent **会话自己的对话**：用户进它会话说的话，下一回合它带着（不分家的核心承诺）。
        dialogue: &[Msg],
        chat: &mut dyn Chat,
        mut tools: Option<&mut MemberTools>,
        turn: Vec<Msg>,
        // 模型侧运行时文案（行尾的"降级/截断"说明按它现渲染）。
        texts: &crate::capabilities::prompt::api::ToolTexts,
        sink: &mut dyn FnMut(SessionEvent),
    ) -> Result<MemberTurn, String> {
        // 工具面**由角色表发放**（动词 + 只读核实工具）：表是唯一真相，代码里不另写一份名单。
        let (face, with_modules) = systools.role_face(role);
        // 这一回合的工具面装进工具环境（声明与执行都读它）；跑完还原——只是借来跑一个回合。
        let saved = tools.as_deref_mut().map(|t| {
            (
                std::mem::replace(&mut t.allowed, face),
                std::mem::replace(&mut t.with_modules, with_modules),
            )
        });
        let llm = crate::capabilities::llm::api::LlmOpts {
            stream: opts.stream,
            timeout_secs: opts.timeout_secs,
        };
        let mut lines: Vec<LineView> = Vec::new();
        let next_line = std::cell::Cell::new(0u64);
        // 工具调用**实时**外送一条（短暂，不落盘）：用户能看到它在读、在查，而不是整回合黑箱。
        let sink_cell = std::cell::RefCell::new(&mut *sink);
        let mut on_tool = |v: &ToolCallView| {
            (sink_cell.borrow_mut())(SessionEvent::ToolCall(v.clone()));
        };
        // 行在回调里**只构造一次**：走与单 agent 同一条构造函数（行格式只有一处定义）。
        // **只要核实留下的行**：表态/正文那条由主会话的投影写（feed/absorb），这里再收一份主转录就重了。
        let mut on_round = |round: &Round, _s: &mut dyn FnMut(SessionEvent)| {
            if round.tool.is_none() {
                return;
            }
            lines.extend(build_round_lines(
                speaker, texts, round, false, &next_line, None,
            ));
        };
        // **逐片外送**：讨论席的发言也要能看到"正在生成"（信封不能当正文流上屏，见 crate::capabilities::session::api::stream_piece）。
        let mut acc = String::new();
        let mut on_chunk = |chunk: Chunk| {
            let mut kind = "text";
            let mut piece = String::new();
            match &chunk {
                Chunk::Start => {
                    acc.clear();
                    kind = "start";
                }
                Chunk::Text(t) => {
                    let (send, next) = crate::capabilities::session::api::stream_piece(&acc, t);
                    piece = send;
                    acc = next;
                }
                Chunk::Reasoning(r) => {
                    kind = "reasoning";
                    piece = r.clone();
                }
            }
            (sink_cell.borrow_mut())(SessionEvent::Delta {
                speaker: speaker.to_string(),
                kind: kind.to_string(),
                text: piece,
            });
            !cancel.load(std::sync::atomic::Ordering::Relaxed)
        };
        // 定稿行不走这个出口（讨论席的发言由主会话投影写）：给它一个占位，别把两处的行写重。
        let mut discard = |_e: SessionEvent| {};
        let rounds = converse_with(
            chat,
            tools.as_deref_mut(),
            identity,
            dialogue.to_vec(),
            llm,
            speaker,
            &mut on_chunk,
            &mut on_tool,
            &mut on_round,
            &mut discard,
            &turn,
            true,
        );
        if let (Some(t), Some((allowed, with_modules))) = (tools, saved) {
            t.allowed = allowed;
            t.with_modules = with_modules;
        }
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("已停止".to_string());
        }
        // 末轮就是这一回合的结论（表态轮，或"没表态"的散文轮）；调用失败如实报错。
        let last = rounds.last().ok_or_else(|| "已停止".to_string())?;
        if let Some(err) = &last.error {
            return Err(err.clone());
        }
        Ok(MemberTurn {
            verb: last.verb,
            text: last.text.clone(),
            degraded: last.degraded,
            truncated: last.truncated(),
            #[cfg(test)]
            lines,
        })
    }
}

impl Discussion {
    pub fn new(
        members: Vec<Member>,
        allow_autonomy: bool,
        prompts: Arc<dyn Prompt>,
        systools: Arc<dyn crate::capabilities::tools::api::Tools>,
        llm: crate::capabilities::llm::api::LlmOpts,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
        protocol: String,
    ) -> Discussion {
        Discussion {
            members,
            transcript: Vec::new(),
            round: 0,
            pending_user_answers: Vec::new(),
            closed: false,
            allow_autonomy,
            prompts,
            systools,
            llm,
            cancel,
            protocol,
            reminded: std::collections::BTreeMap::new(),
            remind_round: 0,
            opener: String::new(),
            cursor: Cursor::Fresh,
            handed: 0,
        }
    }

    /// 开始讨论：渲染开场提示词（一次），游标进入开场阶段。
    /// 与 advance/feed 一起构成可暂停的状态机——**驱动权在核心**（见 session-model.md 二之二）。
    pub fn start(&mut self, task: &str) {
        self.opener = self.prompts.render(
            Segment::DiscussOpener,
            &[
                ("protocol", self.protocol.clone()),
                ("task", task.to_string()),
            ],
        );
        self.handed = self.transcript.len();
        self.cursor = Cursor::Opener(0);
    }

    /// 接上「停止」标志（CollabSession 注入；回档重建后也要重新接）。
    pub fn set_cancel(&mut self, cancel: std::sync::Arc<std::sync::atomic::AtomicBool>) {
        self.cancel = cancel;
    }

    /// 第 i 个成员的 agent 名（核心据此拼出它的会话名）。
    pub fn member_id(&self, i: usize) -> Option<&str> {
        self.members.get(i).map(|m| m.id.as_str())
    }

    /// 「停止」标志（与核心共享同一个）。
    pub fn cancel_flag(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        std::sync::Arc::clone(&self.cancel)
    }

    /// 开场还没开始过（核心驱动的第一步据此调 start）。
    pub fn not_started(&self) -> bool {
        matches!(self.cursor, Cursor::Fresh) && self.round == 0
    }

    /// 是否已被要求停止。
    fn cancelled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// 本轮的调用选项：流式与预算都取全局设置（讨论也走同一份，不再是写死的非流式）。
    fn opts(&self) -> crate::capabilities::llm::api::CompleteOpts<'static> {
        crate::capabilities::llm::api::CompleteOpts::plain(self.llm.stream)
            .with_timeout(self.llm.timeout_secs)
    }

    /// 首轮：聊天约定 + 用户需求（文案经提示词册渲染）。
    #[cfg(test)]
    pub fn open(
        &mut self,
        task: &str,
        on_lines: &mut LineSink<'_>,
        sink: &mut dyn FnMut(SessionEvent),
    ) -> Result<(), String> {
        // 状态机驱动：开场也是"问 → 收 → 再问"，判定全在 advance/feed 里
        //（开场的表态与轮次同一口径：同意 / 离开立刻生效）。
        // 驱动权将来归核心：把这一圈换成核心的 take/put 序列即可。
        self.start(task);
        loop {
            match self.advance() {
                Adv::Ask { i, identity, turn } => {
                    let turn = self.member_turn(i, &identity, turn, sink)?;
                    // 开场也要有"没表态"的处置，否则这个循环会卡在同一个人身上空转。
                    match self.after_turn(i, turn.verb.is_some(), false, MAX_DISCUSS_REMIND) {
                        // 开场不因请教而中止（feed 已按阶段处理）。
                        AfterTurn::Done => {
                            let _ = self.feed(i, turn, 0, on_lines, sink);
                        }
                        // 内存驱动没有会话可注入提醒：直接重问同一个人（计数照走）。
                        AfterTurn::Remind => continue,
                        AfterTurn::Unanswered => {
                            self.note_system(&format!("[{}] 本轮未回应", self.members[i].id));
                            self.skip(i);
                        }
                    }
                }
                // 开场问完（或已是终态）：交回上层，轮次由 step 继续。
                Adv::Opened | Adv::Out(_) => return Ok(()),
            }
        }
    }

    /// 推进一轮：把当前转录并入上下文，依次转达给每个在组且未同意的成员。
    /// 讨论阶段不接工具循环：工具属执行机制，讨论只出主意（最小边界）。
    #[cfg(test)]
    pub fn step(
        &mut self,
        on_lines: &mut LineSink<'_>,
        sink: &mut dyn FnMut(SessionEvent),
    ) -> TurnOut {
        // 这个循环**必须有前进条件**：没表态时由 after_turn 计数（提醒 → 放过），
        // 否则假模型瞬间返回会把这里变成紧凑死循环（真烧过 CPU）。
        // 状态机驱动：本函数只负责"问 → 收 → 再问"，判定全在 advance/feed 里。
        // 驱动权将来归核心（见 docs/architecture/session-model.md 二之二）：那时把这一圈换成
        // 核心的 take/put 序列即可，判定一行不用改。
        loop {
            match self.advance() {
                Adv::Ask { i, identity, turn } => {
                    let turn = match self.member_turn(i, &identity, turn, sink) {
                        Ok(t) => t,
                        Err(_) if self.cancelled() => return TurnOut::Stopped,
                        Err(err) => return TurnOut::Interrupted(err),
                    };
                    let has_verb = turn.verb.is_some();
                    // 与生产路径**同一条策略**：没表态就重问（上限），到顶记"未回应"放过它。
                    match self.after_turn(i, has_verb, false, MAX_DISCUSS_REMIND) {
                        AfterTurn::Done => {
                            if let Some(out) = self.feed(i, turn, 0, on_lines, sink) {
                                return out;
                            }
                        }
                        // 内存驱动没有会话可注入提醒：直接重问同一个人（计数照走）。
                        AfterTurn::Remind => continue,
                        AfterTurn::Unanswered => {
                            self.note_system(&format!("[{}] 本轮未回应", self.members[i].id));
                            self.skip(i);
                        }
                    }
                }
                // 开场刚问完（只有 open 会碰到）：接着进轮次。
                Adv::Opened => continue,
                Adv::Out(out) => return out,
            }
        }
    }

    /// 推进讨论的**一步**：只决定"该问谁"（或给出终态）——**不自己调模型**。
    /// 驱动权归核心（它同时看得到协作会话与各 agent 的会话），见 session-model.md 二之二。
    pub fn advance(&mut self) -> Adv {
        if self.closed {
            return Adv::Out(TurnOut::Done);
        }
        // 已被要求停止：连轮次标记都不留（这一轮根本没开始）。
        if self.cancelled() {
            return Adv::Out(TurnOut::Stopped);
        }
        // 开场：逐个问一遍（开场不跳过任何人），问完进轮次。
        if let Cursor::Opener(i) = self.cursor {
            if i >= self.members.len() {
                self.round = 1;
                self.cursor = Cursor::Fresh;
                return Adv::Opened;
            }
            return Adv::Ask {
                i,
                identity: self.member_identity(i),
                turn: vec![Msg::user(self.opener.clone())],
            };
        }
        if matches!(self.cursor, Cursor::Fresh) {
            // 本轮到此刻还没交出去的行数（轮次标记也算）：一个成员说完就把它那一批交出去。
            self.handed = self.transcript.len();
            // 轮次边界：本轮的发言都在这条之后（回放时据此重算「本轮谁已同意」）。
            self.transcript.push(LineView::round(self.round + 1));
            // 用户回答优先转达。
            if let Some(ans) = self.pending_user_answers.first().cloned() {
                self.pending_user_answers.remove(0);
                self.transcript.push(LineView::user("", ans));
            }
            self.cursor = Cursor::At(0);
        }
        // 同意是**粘住**的：发过 agree 的人不再被追问；离开的人不算在场。
        let mut i = match self.cursor {
            Cursor::At(i) => i,
            Cursor::Fresh => 0,
            Cursor::Opener(i) => i,
        };
        while i < self.members.len() && (!self.members[i].present || self.members[i].agreed) {
            i += 1;
        }
        if i >= self.members.len() {
            // 本轮问完：收敛 / 上限判定，交回上层（下一次从新的一轮开始）。
            self.round += 1;
            self.cursor = Cursor::Fresh;
            if self.members.iter().filter(|m| m.present).all(|m| m.agreed) {
                self.closed = true;
                // 讨论收束 = 进入下一阶段：提醒计数一并归零（它只在本阶段有意义）。
                self.reminded.clear();
                return Adv::Out(TurnOut::Done);
            }
            if self.round > MAX_ROUNDS {
                self.closed = true;
                self.reminded.clear();
                return Adv::Out(TurnOut::Done);
            }
            return Adv::Out(TurnOut::Round);
        }
        self.cursor = Cursor::At(i);
        let identity = self.member_identity(i);
        let step_prompt = self.prompts.render(
            Segment::DiscussStep,
            &[(
                "transcript",
                self.transcript
                    .iter()
                    .map(|l| l.render())
                    .collect::<Vec<_>>()
                    .join("\n"),
            )],
        );
        Adv::Ask {
            i,
            identity,
            turn: vec![Msg::user(step_prompt)],
        }
    }

    /// 这个席位的身份块：**每回合现渲染**（成员的参数 + 当前提示词册 + 登记处给的形态）。
    fn member_identity(&self, i: usize) -> String {
        let m = &self.members[i];
        m.params.identity(&*self.prompts, m.mode)
    }

    /// 收下一个成员回合的结果（状态机的一步）：吸收、外送它那一批行、按动词改状态。
    /// 返回 Some = 该立刻交回上层（请教用户）；None = 继续问下一个。
    pub fn feed(
        &mut self,
        i: usize,
        turn: MemberTurn,
        turn_id: u64,
        on_lines: &mut LineSink<'_>,
        sink: &mut dyn FnMut(SessionEvent),
    ) -> Option<TurnOut> {
        // 开场与轮次共用这一条收尾路径；区别只在"请教要不要中止"（开场不中止）。
        let opener = matches!(self.cursor, Cursor::Opener(_));
        let id = self.members[i].id.clone();
        // **没表态**：不投影主会话（原文只在它自己的会话里）、不改它的状态。
        // 但**照样往后挪一格**——这是安全默认：任何驱动都不会因为"没表态"而卡在同一人身上
        //（要重问的驱动**根本不调 feed**，见 after_turn 的 Remind 分支）。
        let Some(verb) = turn.verb else {
            self.cursor = if opener {
                Cursor::Opener(i + 1)
            } else {
                Cursor::At(i + 1)
            };
            return None;
        };
        let (text, degraded) = (turn.text.clone(), turn.degraded);
        self.absorb(&id, verb, text.clone(), degraded, turn.truncated, turn_id);
        // 逐成员外送：**这个人说完就出它那一行**，不等整轮问完。
        on_lines(&self.transcript[self.handed..], sink);
        self.handed = self.transcript.len();
        // 往后挪一格（advance 下次从下一个人接着问）。
        self.cursor = if opener {
            Cursor::Opener(i + 1)
        } else {
            Cursor::At(i + 1)
        };
        let m = &mut self.members[i];
        match verb {
            Verb::Leave => m.present = false,
            Verb::Agree => m.agreed = true,
            Verb::Ask => {
                // 开场不因请教而中止：开场是各人表态，还没有可讨论的方案。
                if opener {
                    return None;
                }
                if self.allow_autonomy {
                    let note = self.prompts.text(Segment::DiscussAutonomyNote).to_string();
                    // 这是**系统**给的自主说明，不是用户说的。
                    self.transcript.push(LineView::system("", note));
                    return None;
                }
                return Some(TurnOut::AskUser {
                    member: id,
                    question: text,
                });
            }
            Verb::Say | Verb::Tool => {}
        }
        None
    }

    fn absorb(
        &mut self,
        id: &str,
        verb: Verb,
        text: String,
        degraded: bool,
        truncated: bool,
        turn: u64,
    ) {
        // 正文 = 发言本身（说话人与动词在字段里）；降级/截断的说明照样跟在正文后。
        let mut line = text;
        if degraded {
            line.push_str(
                &self
                    .prompts
                    .tools()
                    .render(&self.prompts.tools().discuss_degraded, &[]),
            );
        }
        // 被长度截断：如实写在行尾（与"降级"同一套做法）——模型与用户都看得到
        if truncated {
            line.push_str(
                &self
                    .prompts
                    .tools()
                    .render(&self.prompts.tools().truncated_suffix, &[]),
            );
        }
        let mut v = LineView::speech(id, verb_tag(verb), line);
        v.degraded = degraded;
        v.turn = turn;
        self.transcript.push(v);
    }

    /// 成员一轮之后的处置：**核心只提醒、不强制**（见 docs/architecture/session-model.md 二）。
    /// 用户主动中止时计数不再工作（不注入提醒）；每轮开始时提醒次数归零。
    pub fn after_turn(
        &mut self,
        i: usize,
        has_verb: bool,
        user_stopped: bool,
        remind_cap: u32,
    ) -> AfterTurn {
        if self.round != self.remind_round {
            self.reminded.clear();
            self.remind_round = self.round;
        }
        let id = self.members[i].id.clone();
        if has_verb {
            self.reminded.remove(&id);
            return AfterTurn::Done;
        }
        if user_stopped {
            return AfterTurn::Done;
        }
        let n = self.reminded.entry(id.clone()).or_insert(0);
        if *n < remind_cap {
            *n += 1;
            return AfterTurn::Remind;
        }
        self.reminded.remove(&id);
        AfterTurn::Unanswered
    }

    /// 放过一个没表态的成员：不吸收、只把游标往后挪一格（提醒到顶时用）。
    pub fn skip(&mut self, i: usize) {
        self.cursor = if matches!(self.cursor, Cursor::Opener(_)) {
            Cursor::Opener(i + 1)
        } else {
            Cursor::At(i + 1)
        };
    }

    /// 记一行**系统消息**（提醒/边界这类不是谁说的内容）到主会话转录。
    /// 见 docs/architecture/session-model.md 二"系统消息"。
    pub fn note_system(&mut self, text: &str) {
        self.transcript.push(LineView::system("", text.to_string()));
    }

    /// 全员同意后：核心整理——总结讨论，为每个留下的成员写执行任务提示词。
    /// 整理：核心 AI 总结讨论并出**任务链**（见 docs/architecture/task-chain.md）。
    /// 产出走 **plan 工具调用**（核心操作不手写 JSON）；没调用或载荷不合法就**如实报错**。
    pub fn synthesize(
        &self,
        core_chat: &mut dyn Chat,
        mode: crate::capabilities::llm::api::ToolMode,
        verify: Option<&mut MemberTools>,
        // 核心这一轮的行（工具行 + 思维链 + 正文）推给谁：落不落由那个会话模块定。
        sink: &mut dyn FnMut(SessionEvent),
    ) -> Result<
        (
            String,
            crate::capabilities::taskchain::api::TaskChain,
            String,
        ),
        String,
    > {
        let roster = self
            .members
            .iter()
            .filter(|m| m.present)
            .map(|m| m.id.clone())
            .collect::<Vec<_>>()
            .join("、");
        let user = self.prompts.render(
            Segment::SynthesizeUser,
            &[
                ("roster", roster),
                (
                    "transcript",
                    self.transcript
                        .iter()
                        .map(|l| l.render())
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
            ],
        );
        let msgs = vec![
            Msg::system(self.prompts.text(Segment::SynthesizeSystem).to_string()),
            Msg::user(user),
        ];
        if self.cancelled() {
            return Err("已停止".to_string());
        }
        // 核心操作走工具调用：载荷形状与从前一致（plan + nodes），只是入口变成 plan 工具。
        let payload = crate::capabilities::session::api::core_operation(
            &*self.systools,
            "planner",
            "plan",
            mode,
            core_chat,
            &msgs,
            self.opts(),
            Some(&self.cancel),
            verify,
            sink,
        )?;
        if self.cancelled() {
            return Err("已停止".to_string());
        }
        let parsed: SynthReply = serde_json::from_value(payload)
            .map_err(|e| format!("plan 工具的载荷不合法（{}）", e))?;
        let chain = crate::capabilities::taskchain::api::TaskChain {
            nodes: parsed
                .nodes
                .into_iter()
                .map(|n| crate::capabilities::taskchain::api::TaskNode {
                    id: n.id,
                    title: n.title,
                    objective: n.objective,
                    assignee: n.assignee,
                    deps: n.deps,
                    status: crate::capabilities::taskchain::api::NodeStatus::Pending,
                    sub_session: None,
                    report: None,
                    acceptance: None,
                    reported: false,
                })
                .collect(),
        };
        Ok((parsed.plan, chain, parsed.advice))
    }
}

/// 核心整理的结构化回执（形状见 prompts/roles/planner.yaml 的 synthesize）。
#[derive(Debug, Clone, Deserialize)]
struct SynthReply {
    plan: String,
    #[serde(default)]
    nodes: Vec<SynthNode>,
    /// 核心给用户的**建议**（推荐现在开工还是先改方案）；没有就是空串。
    #[serde(default)]
    advice: String,
}

/// 任务链里的一个节点（核心给的是"意图"，状态与子会话由核心自己管）。
#[derive(Debug, Clone, Deserialize)]
struct SynthNode {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    objective: String,
    #[serde(default)]
    assignee: String,
    #[serde(default)]
    deps: Vec<String>,
}

/// 验收清单条目：核心输出的结构化核对结果。
#[derive(Debug, Clone, Deserialize)]
pub struct CheckItem {
    pub item: String,
    pub status: String,
    #[serde(default)]
    pub evidence: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    /// `status = fail` 时**要返工的节点 id**（核心按链校验；填错/漏填会让核心重填）。
    /// 为什么是节点 id 而不是人名：链是节点级的，一个 agent 可能负责多个节点（见 task-chain.md 五）。
    #[serde(default)]
    pub rework: Option<String>,
}

/// 执行与验收：成员按任务干活并回报；核心对照回报产出结构化清单。
pub struct Execution {
    pub reports: BTreeMap<String, String>,
    /// 验收原始输出（解析失败时如实呈现）。
    pub checklist_raw: String,
    /// 结构化清单；空 = 解析失败（all_pass 保守判否）。
    pub items: Vec<CheckItem>,
    /// 执行/验收途中调用失败（超时 / 网络）：非空 = 本轮**中断**，不交付。
    /// 上层据此如实告知用户；会话保持可继续（用户点「继续」重新推进）。
    pub error: Option<String>,
    /// 被用户「停止」：非空 = 本轮**停止**，未收完的回报**不入册**（半截回报进转录会误导验收）。
    pub stopped: bool,
    /// 「停止」标志（泵注入）：每次模型调用前与调用中途都看它。
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Execution {
    pub fn new() -> Execution {
        Execution {
            reports: BTreeMap::new(),
            checklist_raw: String::new(),
            items: Vec::new(),
            error: None,
            stopped: false,
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// 是否已被要求停止（总验收共用）。
    fn cancelled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::Relaxed)
    }

    // 参数是一组必须一路透传的出口（chat / plan / 对照表 / 重填说明 / 提示词册 / 通道 / 核实环境）：
    // 与 judge_clear / review_nodes 同一取舍（见 docs/testing/quality-isolation.md）。
    #[allow(clippy::too_many_arguments)]
    /// 总验收（一次调用）。`nodes` = "节点 id — 负责人"对照表（模型只能从这里选 `rework`）；
    /// `retry` = 上一次填错了要它重填的话（核心据此**一直重填**到合法为止，不设次数上限）。
    pub fn review(
        &mut self,
        core_chat: &mut dyn Chat,
        plan: &str,
        nodes: &str,
        retry: Option<&str>,
        prompt: &dyn Prompt,
        systools: &dyn crate::capabilities::tools::api::Tools,
        llm: crate::capabilities::llm::api::LlmOpts,
        mode: crate::capabilities::llm::api::ToolMode,
        verify: Option<&mut MemberTools>,
        // 核心这一轮的行推给谁（总验收也要能看到它在核对什么）。
        sink: &mut dyn FnMut(SessionEvent),
    ) {
        let reports = self
            .reports
            .iter()
            .map(|(id, r)| format!("[{}] {}\n", id, r))
            .collect::<Vec<_>>()
            .join("");
        let user = prompt.render(
            Segment::ReviewUser,
            &[
                ("plan", plan.to_string()),
                ("reports", reports),
                ("nodes", nodes.to_string()),
            ],
        );
        let mut msgs = vec![
            Msg::system(prompt.text(Segment::ReviewSystem).to_string()),
            Msg::user(user),
        ];
        if let Some(again) = retry {
            msgs.push(Msg::user(again.to_string()));
        }
        if self.cancelled() {
            self.stopped = true;
            return;
        }
        let opts = crate::capabilities::llm::api::CompleteOpts::plain(llm.stream)
            .with_timeout(llm.timeout_secs);
        // 核心操作走工具调用：总验收清单由 checklist 工具承载。
        let made = crate::capabilities::session::api::core_operation(
            systools,
            "orchestrator",
            "checklist",
            mode,
            core_chat,
            &msgs,
            opts,
            Some(&self.cancel),
            verify,
            sink,
        );
        if self.cancelled() {
            self.stopped = true;
            return;
        }
        match made {
            Ok(payload) => {
                // 载荷里的 items 数组；形状不对就如实记空清单（all_pass 保守判否）。
                self.items = payload
                    .get("items")
                    .cloned()
                    .and_then(|v| serde_json::from_value::<Vec<CheckItem>>(v).ok())
                    .unwrap_or_default();
                self.checklist_raw = payload.to_string();
            }
            Err(err) => {
                self.error = Some(err);
            }
        }
    }

    /// `status = fail` 且填了 `rework` 的节点（按清单顺序去重）：核心据此**只重派这些**。
    pub fn rework_targets(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for it in &self.items {
            if it.status.eq_ignore_ascii_case("fail") {
                if let Some(r) = it.rework.as_deref() {
                    if !r.trim().is_empty() && !out.iter().any(|x| x == r) {
                        out.push(r.to_string());
                    }
                }
            }
        }
        out
    }

    /// 这次判定里的问题（空 = 合法）：fail 必须填 `rework`，且只能是 `known` 里的节点 id。
    /// 核心据此**要求模型重填**（一直重填，不设上限）——绝不静默丢掉一条判定。
    pub fn rework_problems(&self, known: &[String]) -> Vec<String> {
        let mut out = Vec::new();
        for it in &self.items {
            if !it.status.eq_ignore_ascii_case("fail") {
                continue;
            }
            match it.rework.as_deref().map(str::trim) {
                Some(r) if !r.is_empty() => {
                    if !known.iter().any(|k| k == r) {
                        out.push(format!("条目「{}」填的 rework={} 不在节点表里", it.item, r));
                    }
                }
                _ => out.push(format!("条目「{}」没过（fail）但没填 rework", it.item)),
            }
        }
        out
    }

    pub fn all_pass(&self) -> bool {
        // 清单为空（解析失败）= 保守判否；有清单则逐项全过才通过。
        !self.items.is_empty()
            && self
                .items
                .iter()
                .all(|i| i.status.eq_ignore_ascii_case("pass"))
    }
}

/// 一次工具调用的产出：调用视图 + 它压进历史的消息。
pub struct ToolRun {
    pub view: ToolCallView,
    /// 该工具行压进历史的消息（[工具结果] …）。
    pub msgs: Vec<Msg>,
}

/// 拼一次模型调用的消息：**身份 + 本回合工具 + 对话 + 本回合提示**。
///
/// 为什么只有这一处：身份与工具块都是**派生**的（登记处 + 提示词册 + 这一回合的身份），
/// 它们不占对话的位置——对话里只有真正发生过的事（谁说了什么、调了什么工具）。
/// 实时与重建都从这里拼，所以"回放与实时产出同样的消息"只约束对话本身。
pub fn assemble(
    identity: &str,
    tools: Option<&MemberTools>,
    ids: &[String],
    with_modules: bool,
    dialogue: &[Msg],
    turn: &[Msg],
) -> Vec<Msg> {
    let mut out: Vec<Msg> = Vec::with_capacity(dialogue.len() + turn.len() + 2);
    out.push(Msg::system(identity));
    if let Some(ctx) = tools {
        let block = ctx.tools_block(ids, with_modules);
        if !block.is_empty() {
            out.push(Msg::system(block));
        }
    }
    out.extend(dialogue.iter().cloned());
    out.extend(turn.iter().cloned());
    out
}

/// 逐轮产出回调：拿到刚定稿的一轮 + 本次的出口。
/// 出口当参数传而不是让回调捕获它——否则回调借着 sink，调用方随后用不了同一个 sink。
pub type RoundSink<'a> = dyn FnMut(&Round, &mut dyn FnMut(SessionEvent)) + 'a;

/// 讨论行的**逐成员外送回调**：拿到刚定稿的行 + 本次的出口。
/// 出口当参数传而不是让回调捕获它——否则回调借着 sink，`step`/`open` 的调用方随后用不了它。
pub type LineSink<'a> = dyn FnMut(&[LineView], &mut dyn FnMut(SessionEvent)) + 'a;

/// 表态动词的**行标签**：讨论转录行写成 `[谁:say] 内容`（行格式与回档解析同一处定义）。
pub(crate) fn verb_tag(v: Verb) -> &'static str {
    match v {
        Verb::Say => "say",
        Verb::Ask => "ask",
        Verb::Leave => "leave",
        Verb::Agree => "agree",
        Verb::Tool => "tool",
    }
}

/// 原生通道的工具名 → 讨论动词：**只认协作动词**，其余一律不认识（不认识 = 越权，如实拒绝）。
pub fn verb_of(name: &str) -> Option<Verb> {
    match name {
        "say" => Some(Verb::Say),
        "agree" => Some(Verb::Agree),
        "leave" => Some(Verb::Leave),
        "ask" => Some(Verb::Ask),
        _ => None,
    }
}

/// 原生调用的参数里取正文（供应商给的是一段 JSON 文本；取不到就是空串——不猜）。
pub fn arg_text(args_json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(args_json)
        .ok()
        .and_then(|v| {
            v.get("text")
                .and_then(|t| t.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_default()
}

/// 一轮模型调用的产出（一轮 = 一条文本转录行；有工具时紧跟一条工具行）。
/// 原始输出不进这里：工具轮由 ToolCallView.raw 承载、文本轮进上下文的就是解析后的文本。
pub struct Round {
    /// 这一轮属于哪次模型回复（一次回复可能产出多条工具行）。
    pub reply: u64,
    /// 解析后的可见文本（信封缺失时即原文）；工具轮为空串（它说的就是那封信封）。
    pub text: String,
    /// 该轮思维链（没给就是空串）。
    pub reasoning: String,
    /// 该轮压进历史的消息：工具轮 = [assistant(raw)]，末轮 = [assistant(text)]。
    pub text_msgs: Vec<Msg>,
    pub tool: Option<ToolRun>,
    /// 供应商给的结束原因（原样；没给 = 空串）：核心据此分辨"写完停"还是"被长度截断"。
    pub finish: String,
    /// 这次调用失败了（超时 / 网络）：非空 = **没有拿到模型回复**，这一轮不该落转录。
    /// 上层据此如实告知用户并中断本轮（用户可以点「继续」重试）。
    pub error: Option<String>,
    /// 这一轮是一次**表态**（讨论席的协作动词）：它是该回合的收尾轮，行上带动词标签。
    /// 执行席不表态（恒为 None）。
    pub verb: Option<Verb>,
    /// 这一轮的回复**没写成信封**（散文 / 信封写坏）：如实传给上层，不假装它是一次规范回复。
    pub degraded: bool,
}

impl Round {
    /// 这一轮的输出是不是被供应商按长度截断了。
    pub fn truncated(&self) -> bool {
        crate::capabilities::llm::api::truncated(&self.finish)
    }
}

/// stream/on 透传给通道（呈现层在 on 里外送 Delta）；on 返回 false = 用户要求中止。
/// on_tool 在每个工具跑完后立刻回调（工具行与文本行因此天然有序）。
/// 终止保证：超限后告知一次并强制收尾；其后再来 tool 信封按原文作答，不再执行。
// 逐轮外送要的四个出口（分片 / 工具 / 逐轮 / 提示词）都是回调，收口成参数对象只是把参数挪个地方、
// 并让"谁在什么时候拿到什么"更难读。这是有意的设计取舍（同 docs/testing/quality-isolation.md 的 allow 清单）。
#[allow(clippy::too_many_arguments)]
pub fn converse_with(
    chat: &mut dyn Chat,
    mut tools: Option<&mut MemberTools>,
    identity: &str,
    dialogue: Vec<Msg>,
    llm: crate::capabilities::llm::api::LlmOpts,
    speaker: &str,
    on: &mut dyn FnMut(Chunk) -> bool,
    on_tool: &mut dyn FnMut(&ToolCallView),
    on_round: &mut RoundSink<'_>,
    sink: &mut dyn FnMut(SessionEvent),
    // 本回合的提示（讨论席的开场/轮转词；执行席常为空——它的指令在派发行里）。
    turn: &[Msg],
    // 本回合认不认**协作表态**（讨论席认：见到动词就是这一回合的发言，收尾）。
    verbs: bool,
) -> Vec<Round> {
    // 身份 + 本回合工具 + 对话 + 本回合提示：**唯一的装配点**。
    // 工具面取自这一席位（执行席的表现 + 它自己模块的工具）；讨论席的动词面也在这里（见 crate::capabilities::session::api::TurnRun）。
    let (ids, with_modules) = match tools.as_ref() {
        Some(ctx) => (ctx.allowed.clone(), ctx.with_modules),
        None => (Vec::new(), false),
    };
    let mut msgs = assemble(
        identity,
        tools.as_deref(),
        &ids,
        with_modules,
        &dialogue,
        turn,
    );
    // 观察账本随会话保存（回档时清空），这里不动它——它的语义是"这一段转录里的读取证据"。
    let mut rounds: Vec<Round> = Vec::new();
    // 逐轮产出：**一轮跑完就把它交出去**（调用方据此立刻外送与落盘，不必等整个回合结束）。
    // 用宏而不是逐个改写 push 点：5 个分支都要"先回调、再入册"，写死五遍迟早漏一处。
    macro_rules! push_round {
        ($r:expr) => {{
            let r = $r;
            on_round(&r, sink);
            rounds.push(r);
        }};
    }
    // 没有工具环境时的回复号来源（见下面 reply_id）。
    let mut local_reply = 0u64;
    // 本代是否被用户中途停止（通道回调返回 false）：循环顶部据此退出（见下）。
    let mut aborted = false;
    loop {
        // 用户在生成中途点了「停止」（通道回调返回 false）：**这一轮到此为止**，不再发起下一次调用。
        // 没有这条出口，被停的生成会以"每次调用立刻返回"的速度空转（真机上烧过一次 CPU）；
        // 此前是靠工具调用上限兜底的——上限删掉后这条出口必须自己站住。
        if aborted {
            return rounds;
        }
        // 这一回复的稳定 id：一次模型回复一个号，本次问询里的多条工具行共用它。
        // 没有工具环境时给本代内的局部号即可（那条路径不落转录行、也不分组）。
        let reply_id = match tools.as_deref_mut() {
            Some(ctx) => ctx.next_reply(),
            None => {
                local_reply += 1;
                local_reply
            }
        };
        // 形态与工具声明面：由本成员的通道形态决定（envelope = 不声明，走手写信封；native = 声明本成员的工具）
        let (mode, decls) = match tools.as_deref_mut() {
            Some(ctx) if ctx.mode == crate::capabilities::llm::api::ToolMode::Native => {
                let mode = ctx.mode;
                (mode, tool_decls(ctx))
            }
            Some(ctx) => (ctx.mode, ToolDecls::default()),
            None => (
                crate::capabilities::llm::api::ToolMode::Envelope,
                ToolDecls::default(),
            ),
        };
        // 逐轮累积思维链（原文以通道返回值为准：非流式通道不回 Chunk）。
        let mut reasoning = String::new();
        let done = {
            let mut sink = |chunk: Chunk| {
                match &chunk {
                    Chunk::Start => reasoning.clear(),
                    Chunk::Text(_) => {}
                    Chunk::Reasoning(r) => reasoning.push_str(r),
                }
                let keep = on(chunk);
                if !keep {
                    aborted = true;
                }
                keep
            };
            let opts = CompleteOpts {
                stream: llm.stream,
                tools: if decls.list.is_empty() {
                    None
                } else {
                    Some(&decls.list)
                },
                timeout_secs: llm.timeout_secs,
            };
            chat.complete(&msgs, opts, &mut sink)
        };
        // 调用失败（超时 / 网络）：**不是模型的回复**——这一轮不解析信封、不执行工具、不落转录，
        // 只把原因带回，让上层如实告知用户并中断本轮（用户可以点「继续」重试）。
        // 为什么必须短路：错误文本若被当成发言吸收，核心按转录派生的"下一步该谁说话"就歪了。
        if let Some(err) = done.error.clone() {
            push_round!(Round {
                reply: reply_id,
                text: String::new(),
                reasoning: String::new(),
                text_msgs: Vec::new(),
                tool: None,
                finish: String::new(),
                error: Some(err),
                verb: None,
                degraded: false,
            });
            return rounds;
        }
        // 非流式供应商不回 Chunk，使用响应里的思维链。
        if reasoning.is_empty() {
            reasoning = done.reasoning.clone();
        }
        // 结束原因如实带回：被长度截断要落日志——事后才判定得出"是截断还是模型自己写错"。
        let finish = done.finish.clone();
        let truncated = done.truncated();
        let calls = done.calls.clone();
        let raw = done.raw;
        if truncated {
            if let Some(t) = tools.as_deref_mut() {
                t.log.warn(
                    "engine::converse",
                    &format!(
                        "模型输出被长度截断（finish_reason={}）：第 {} 轮，正文 {} 字",
                        finish,
                        rounds.len() + 1,
                        raw.chars().count()
                    ),
                );
            }
        }
        let mut reply = envelope::parse(&raw);
        // 手写信封不合法时**先**问修复端口：只做无歧义的修补（默认实现只转义字符串里的裸控制字符）。
        // 修好并重新解析成合法工具信封 = 本轮照常执行工具；修不了就走原来的"失败工具行"路径。
        // 封顶后不修（与"封顶后不再执行工具"同一口径）。
        // 用户中止的生成不修：半截信封是"被停下来"的产物，不是模型的意图——绝不据此执行工具。
        // 自由格式工具（patch）也不修：它的正文在信封之外，转义控制字符会把补丁里的换行弄坏。
        let freeform_tool = reply
            .tools
            .iter()
            .any(|t| crate::capabilities::tools::api::is_freeform(&t.name));
        let mut repaired: Option<String> = None;
        if !aborted && !freeform_tool {
            if let Some(kind) = reply.tools.first().and_then(|t| t.malformed.clone()) {
                if let Some(ctx) = tools.as_deref_mut() {
                    let out = ctx.llm.repair(&raw, &kind);
                    if let Some(text) = out.repaired.as_deref() {
                        let again = envelope::parse(text);
                        if !again.tools.is_empty()
                            && again.tools.iter().all(|t| t.malformed.is_none())
                        {
                            reply = again;
                            repaired = Some(out.what.join("；"));
                        }
                    }
                }
            }
        }
        // ── 讨论席的**表态**：这一轮就是它的发言——不执行任何调用，本回合到此收尾 ──
        // 与单 agent 的末轮同一形态（正文一行 + 历史一条 assistant），不同的只是行上带动词标签。
        // 两条通道各认各的：native 从结构化槽位认动词，信封从信封自己的动词字段认（散文不算表态）。
        if verbs {
            let from_envelope = |r: &envelope::Reply| -> Option<(Verb, String, bool)> {
                if r.degraded || r.verb == Verb::Tool {
                    None
                } else {
                    Some((r.verb, r.text.clone(), false))
                }
            };
            let said: Option<(Verb, String, bool)> =
                if mode == crate::capabilities::llm::api::ToolMode::Native {
                    calls
                        .iter()
                        .find_map(|c| verb_of(&c.name).map(|v| (v, arg_text(&c.args_json), false)))
                        .or_else(|| from_envelope(&reply))
                } else {
                    from_envelope(&reply)
                };
            if let Some((verb, text, degraded)) = said {
                let has_line = !text.trim().is_empty() || !reasoning.trim().is_empty();
                let text_msgs = if has_line {
                    vec![Msg::assistant(text.trim().to_string())]
                } else {
                    Vec::new()
                };
                push_round!(Round {
                    reply: reply_id,
                    text,
                    reasoning,
                    text_msgs,
                    tool: None,
                    finish,
                    error: None,
                    verb: Some(verb),
                    degraded,
                });
                return rounds;
            }
        }
        // ── 原生通道：工具调用来自供应商的结构化槽位（不解析信封）──
        if mode == crate::capabilities::llm::api::ToolMode::Native {
            if let Some(ctx) = tools.as_deref_mut() {
                // ① 有原生调用：逐个执行，各成一条工具行；助手消息如实记下"它调了什么"（回放与下一轮都看得到）
                if !calls.is_empty() {
                    // 先定好每个调用落在哪个工具、参数是什么（patch 的正文在 body 参数里，
                    // 原生协议要求参数是 JSON 对象；转义交给供应商的解码器）。
                    let plan: Vec<(Option<String>, String, String)> = calls
                        .iter()
                        .map(|c| {
                            let (module, tool) = decls
                                .wire
                                .get(&c.name)
                                .cloned()
                                .unwrap_or((None, c.name.clone()));
                            let args = if crate::capabilities::tools::api::is_freeform(&tool) {
                                serde_json::from_str::<serde_json::Value>(&c.args_json)
                                    .ok()
                                    .and_then(|v| {
                                        v.get("body")
                                            .and_then(|b| b.as_str())
                                            .map(|s| s.to_string())
                                    })
                                    .unwrap_or_default()
                            } else {
                                c.args_json.clone()
                            };
                            (module, tool, args)
                        })
                        .collect();
                    // 调度：**连续**声明可并发的调用合成一批并发跑，其余各自独占（写入类因此是批次之间的屏障）。
                    // 结果按原始下标返回，随后一律按原序回填——并发只影响执行，不影响上下文里的顺序。
                    note_unauthorized(ctx, &plan, sink);
                    let done = run_batch(ctx, &plan);
                    // 先按原序把工具行建好（执行已经做完），再让**唯一那处**构造函数产出这一回复的消息：
                    // 实时与重建走同一个函数，"重建上下文与实时一致"因此是结构保证的。
                    let texts = &ctx.sandbox.texts;
                    let reply_text = reply.text.clone();
                    let views: Vec<ToolCallView> = calls
                        .iter()
                        .enumerate()
                        .map(|(i, c)| {
                            let (label, outcome) = done[i].clone();
                            ToolCallView {
                                speaker: speaker.to_string(),
                                module: label,
                                name: plan[i].1.clone(),
                                ok: outcome.ok,
                                args: plan[i].2.clone(),
                                output: outcome.output,
                                // 助手消息正文 = 这一回复的原文（正文与调用进的是同一条消息）
                                raw: reply_text.clone(),
                                call_id: c.id.clone(),
                                reply: reply_id,
                            }
                        })
                        .collect();
                    let msgs_of = reply_msgs(ctx.mode, &reply_text, &views, texts);
                    for m in &msgs_of {
                        msgs.push(m.clone());
                    }
                    for (i, view) in views.into_iter().enumerate() {
                        on_tool(&view);
                        push_round!(Round {
                            reply: reply_id,
                            // 正文只挂在本回复的第一条工具行上（只显示一条，不重复）
                            text: if i == 0 {
                                reply_text.clone()
                            } else {
                                String::new()
                            },
                            reasoning: std::mem::take(&mut reasoning),
                            text_msgs: if i == 0 {
                                vec![msgs_of[0].clone()]
                            } else {
                                Vec::new()
                            },
                            tool: Some(ToolRun {
                                view,
                                msgs: vec![msgs_of[i + 1].clone()],
                            }),
                            finish: finish.clone(),
                            error: None,
                            verb: None,
                            degraded: false,
                        });
                    }
                    continue;
                }
                // ② 没有原生调用却写了信封：**不执行**（两套形态互斥），但也不静默丢掉意图
                {
                    if let Some(inv) = reply.tools.first().cloned() {
                        let texts = &ctx.sandbox.texts;
                        let view = ToolCallView {
                            speaker: speaker.to_string(),
                            module: inv.module.clone().unwrap_or_default(),
                            name: inv.name.clone(),
                            ok: false,
                            args: inv.args_json.clone(),
                            output: texts.native_no_envelope.clone(),
                            raw: raw.clone(),
                            call_id: String::new(),
                            reply: reply_id,
                        };
                        on_tool(&view);
                        // 这条没有合法原生 id（模型是手写的信封）：走文本形状，不能发 role=tool。
                        let msgs_of = reply_msgs(mode, &raw, std::slice::from_ref(&view), texts);
                        for m in &msgs_of {
                            msgs.push(m.clone());
                        }
                        push_round!(Round {
                            reply: reply_id,
                            text: reply.text.clone(),
                            reasoning,
                            text_msgs: vec![msgs_of[0].clone()],
                            tool: Some(ToolRun {
                                view,
                                msgs: vec![msgs_of[1].clone()],
                            }),
                            finish: finish.clone(),
                            error: None,
                            verb: None,
                            degraded: false,
                        });
                        continue;
                    }
                }
            }
        }
        // 信封这一线的分派依据：不合法时恰好一条（见 envelope::build_invokes），合法时为空。
        let malformed = reply.tools.first().and_then(|t| t.malformed.clone());
        match malformed.clone() {
            // 信封不合法（缺 name / 混用两种形态 / calls 为空 / 没写完…）：**一个工具都不执行**，
            // 但记一条失败的工具行把"哪里不合法"回注给模型（下一轮自己改）。同样计入上限，不会死循环。
            _ if malformed.is_some() && tools.is_some() => {
                let inv = reply.tools.first().cloned().expect("上臂已判非空");
                let ctx = tools.as_deref_mut().expect("上臂已判存在");
                // 回执按判定出的类别给修法（未闭合 / 裸控制字符 / 语法错 / 字段不合法）。
                let mut why = crate::capabilities::llm::api::malformed_report(
                    &ctx.sandbox.texts,
                    inv.malformed.as_ref().expect("上臂已判存在"),
                );
                // 供应商说是长度截断：那"写坏 JSON"就不是模型的错，改法也不同（分次写/拆小步骤）。
                if truncated {
                    why.push('\n');
                    why.push_str(&ctx.sandbox.texts.malformed_truncated);
                }
                let view = ToolCallView {
                    speaker: speaker.to_string(),
                    module: inv.module.clone().unwrap_or_default(),
                    name: inv.name.clone(),
                    ok: false,
                    args: inv.args_json.clone(),
                    output: why,
                    raw: raw.clone(),
                    call_id: String::new(),
                    reply: reply_id,
                };
                on_tool(&view);
                let texts = &ctx.sandbox.texts;
                let msgs_of = reply_msgs(mode, &raw, std::slice::from_ref(&view), texts);
                for m in &msgs_of {
                    msgs.push(m.clone());
                }
                push_round!(Round {
                    reply: reply_id,
                    text: reply.text.clone(),
                    reasoning,
                    text_msgs: vec![msgs_of[0].clone()],
                    tool: Some(ToolRun {
                        view,
                        msgs: vec![msgs_of[1].clone()],
                    }),
                    finish: finish.clone(),
                    error: None,
                    verb: None,
                    degraded: true,
                });
            }
            // 合法信封：**一次回复里的多个调用一起执行**（同一套声明并发调度），各成一条工具行。
            _ if tools.is_some() && !reply.tools.is_empty() => {
                let ctx = tools.as_deref_mut().expect("上臂已判存在");
                let invokes = reply.tools.clone();
                // 自由格式工具（patch）只能单发：它的输入是**信封之后的那段正文**（不必转义），
                // 显示正文只认信封**之前**那段——补丁内容不该被当成 AI 发言渲染出来。
                if invokes.len() == 1
                    && crate::capabilities::tools::api::is_freeform(&invokes[0].name)
                {
                    reply.text = invokes[0].lead.clone();
                }
                let plan: Vec<(Option<String>, String, String)> = invokes
                    .iter()
                    .map(|t| {
                        let freeform = crate::capabilities::tools::api::is_freeform(&t.name);
                        let args = if freeform {
                            t.body.clone()
                        } else {
                            t.args_json.clone()
                        };
                        (t.module.clone(), t.name.clone(), args)
                    })
                    .collect();
                // 内置工具（read/write/edit/search）优先且不属于任何模块；外部工具按模块定 cwd。
                note_unauthorized(ctx, &plan, sink);
                let done = run_batch(ctx, &plan);
                // 修过信封就如实标注在回执最前面（模型与用户都能看到核心没有瞎猜）
                let annotate = |outcome: ToolOutcome| -> ToolOutcome {
                    match repaired.as_deref() {
                        Some(what) if !what.is_empty() => ToolOutcome {
                            ok: outcome.ok,
                            output: format!(
                                "{}\n{}",
                                ctx.sandbox.texts.render(
                                    &ctx.sandbox.texts.envelope_repaired,
                                    &[("what", what.to_string())]
                                ),
                                outcome.output
                            ),
                        },
                        _ => outcome,
                    }
                };
                let texts = &ctx.sandbox.texts;
                let views: Vec<ToolCallView> = plan
                    .iter()
                    .enumerate()
                    .map(|(i, (_module, tool, _args))| {
                        let (label, outcome) = done[i].clone();
                        let outcome = annotate(outcome);
                        ToolCallView {
                            speaker: speaker.to_string(),
                            module: label,
                            name: tool.clone(),
                            ok: outcome.ok,
                            args: invokes[i].args_json.clone(),
                            output: outcome.output,
                            raw: raw.clone(),
                            call_id: String::new(),
                            reply: reply_id,
                        }
                    })
                    .collect();
                let msgs_of = reply_msgs(mode, &raw, &views, texts);
                for m in &msgs_of {
                    msgs.push(m.clone());
                }
                // 按原序回填：每条调用一条工具行（多调用时正文只挂第一条）。
                // text = 信封之外的那段正文（可能为空；信封 JSON 已被 parse 剥掉，永不进 text）。
                for (i, view) in views.into_iter().enumerate() {
                    on_tool(&view);
                    push_round!(Round {
                        reply: reply_id,
                        text: if i == 0 {
                            reply.text.clone()
                        } else {
                            String::new()
                        },
                        reasoning: std::mem::take(&mut reasoning),
                        text_msgs: if i == 0 {
                            vec![msgs_of[0].clone()]
                        } else {
                            Vec::new()
                        },
                        tool: Some(ToolRun {
                            view,
                            msgs: vec![msgs_of[i + 1].clone()],
                        }),
                        finish: finish.clone(),
                        error: None,
                        verb: None,
                        degraded: false,
                    });
                }
            }
            // 无工具环境 / 模型不再发起调用：按原文口径如实收录（信封已被剥掉，显示文本里不会有 JSON），循环终止。
            _ => {
                // 只有会出文本行（有正文或思维链）时才往历史里放这条 assistant，
                // 否则实时历史会比重建历史多一条空消息。
                let text = reply.text;
                let has_line = !text.trim().is_empty() || !reasoning.trim().is_empty();
                let text_msgs = if has_line {
                    vec![Msg::assistant(text.trim().to_string())]
                } else {
                    Vec::new()
                };
                push_round!(Round {
                    reply: reply_id,
                    text,
                    reasoning,
                    text_msgs,
                    tool: None,
                    finish,
                    error: None,
                    verb: None,
                    degraded: reply.degraded || reply.verb == Verb::Tool,
                });
                return rounds;
            }
        }
    }
}

// —— 回合驱动：以会话状态跑一次轮循环 ——
//
// **为什么这些方法定义在引擎里**：它们是「驱动」（问模型 → 解析信封 → 调工具 → 落行），
// 会话本身只留状态与簿记。反过来（会话驱动引擎）会形成 `engine ⇄ session` 环。
// 字段以 `pub(super)` 开放：两者同在 `capabilities` 之下（驱动与它会话状态都归会话能力），这是有意的取舍。
// 见 docs/architecture/refactor-plan.md §4.2 批次 12。

// —— 转录行构造：**行格式只有这一处定义** ——
//
// 定义在引擎里：它由逐轮回调在 `converse_with` 内部调用，那时会话已被拆开；
// 会话只提供 `stream_piece` 与状态。
/// 一轮的转录行：文本行（有正文/思维链时）+ 工具行（有工具时）。
/// **不依赖 `&mut self`**：它由逐轮回调在 `converse_with` 内部调用，那时 `self` 已被拆开。
/// 行号从 `next_line` 递增（回调里记不了账，所以由调用方在回合收尾时按同一批行补 marks）。
/// `turn` = 这一行属于哪个回合（讨论席的回合号）；None = 用该轮自己的回复号（单 agent 每轮各成回合）。
/// **行格式只有这一处定义**：单 agent 与讨论席的行都从这里出（回档按同一口径解析回发言）。
pub fn build_round_lines(
    id: &str,
    texts: &crate::capabilities::prompt::api::ToolTexts,
    round: &Round,
    stopped: bool,
    next_line: &std::cell::Cell<u64>,
    turn: Option<u64>,
) -> Vec<LineView> {
    let text = round.text.trim().to_string();
    let has_line = !text.is_empty() || !round.reasoning.trim().is_empty();
    let truncated = round.truncated();
    let mut reasoning = if round.reasoning.trim().is_empty() {
        None
    } else {
        Some(round.reasoning.clone())
    };
    // 回合号：讨论席一轮一个回合号（整场工作单调递增）；单 agent 的每一轮各成"回合"（回档按它对齐）。
    let turn = turn.unwrap_or(round.reply);
    // 说话人与动词是**结构化字段**（正文里不再带 [谁:动词] 标签）；渲染由 LineView::render 拼回。
    let speaker = id.to_string();
    let verb = round
        .verb
        .map(crate::capabilities::collab::service::engine::verb_tag)
        .unwrap_or_default();
    let make = |line: String,
                verb: &str,
                kind: &str,
                reasoning: Option<String>,
                tool: Option<ToolCallView>| {
        let num = next_line.get();
        next_line.set(num + 1);
        LineView {
            id: num,
            reply: round.reply,
            line,
            speaker: speaker.clone(),
            verb: verb.to_string(),
            kind: kind.to_string(),
            reasoning,
            tool,
            degraded: false,
            system: false,
            task: false,
            turn,
        }
    };
    let mut out = Vec::new();
    let text_line = |reasoning: &mut Option<String>, out: &mut Vec<LineView>| {
        // 工具轮没有正文时，思维链必须挂到工具行，不能额外造一条空回答行。
        if !has_line || (text.is_empty() && round.tool.is_some()) {
            return;
        }
        // 正文 = 内容本身（说话人/动词在字段里）；被停/被截断的说明照样跟在正文后。
        let mut line = text.clone();
        if stopped {
            line.push_str(&texts.stopped_suffix);
        }
        if truncated {
            line.push_str(&texts.truncated_suffix);
        }
        out.push(make(line, verb, "msg", reasoning.take(), None));
    };
    match &round.tool {
        Some(run) => {
            // 先出「思考+正文」文本行（只有信封没有正文/思维链时不出空行）。
            text_line(&mut reasoning, &mut out);
            let status = if run.view.ok { "成功" } else { "失败" };
            // 没有文本行时思维链挂到工具行上，不丢。
            // 工具行自带调用视图（呈现层按卡片渲染）：说话人已知，动词留空（不是一次表态）。
            out.push(make(
                format!("工具 {} → {}", run.view.label(), status),
                "",
                "tool",
                reasoning.take(),
                Some(run.view.clone()),
            ));
        }
        None => text_line(&mut reasoning, &mut out),
    }
    out
}
