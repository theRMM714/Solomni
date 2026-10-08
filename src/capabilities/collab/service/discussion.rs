//! **讨论状态机**：建组 → 轮转发言 → 表态判定 → 全员同意后交整理（`synthesis`）。
//! 状态机只认信封动词；发言内容永远是数据，不是指令。
//! 所有发给模型的文案经 capabilities/prompt 渲染自提示词册；成员拥有自己的会话通道。
//! 工具循环（联动 envelope::Verb::Tool 与 ports::ToolRunner）：策略（放行表、并发调度）在核心，机制在适配层。
//! 一次回复里的多个原生调用按**声明**调度：声明可并发的只读类并发跑，其余（含写入类）独占并按原序生效；
//! 结果与工具行一律按原始顺序回填——并发只影响执行，不影响上下文里的顺序。
use super::round::*;
#[cfg(test)]
use crate::capabilities::llm::api::BoxedChat;
use crate::capabilities::llm::api::Msg;
use crate::capabilities::llm::api::Verb;
#[cfg(test)]
use crate::capabilities::llm::api::{Chat, Chunk};
use crate::capabilities::prompt::api::{Prompt, Segment};
use crate::capabilities::session::api::MemberTools;
#[cfg(test)]
use crate::capabilities::session::api::ToolCallView;
use crate::capabilities::session::api::{LineView, SessionEvent};
use std::collections::BTreeMap;
use std::sync::Arc;

pub const MAX_ROUNDS: usize = 6;
/// 一轮内对同一个成员最多提醒几次（**内存驱动**用；生产按设置走）。
#[cfg(test)]
pub const MAX_DISCUSS_REMIND: u32 = 3;

/// 一次成员回合之后的处置（核心据此决定，见 docs/session/session-model.md 二）。
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
    pub(crate) members: Vec<Member>,
    /// 讨论转录：**与单 agent 同一行类型**（LineView）——行格式只有一处定义，呈现层也只有一套渲染。
    pub(crate) transcript: Vec<LineView>,
    pub(crate) round: usize,
    /// 用户对 ask 的回答在此队列：先入先转达。
    pub(crate) pending_user_answers: Vec<String>,
    pub(crate) closed: bool,
    /// yes,allow：授权小组自裁——ask 不中止轮转，留档待办。
    pub(crate) allow_autonomy: bool,
    /// 提示词册能力面：**不是册子本体**（持有者只有提示词能力一处），这里只按名字取段。
    pub(crate) prompts: Arc<dyn Prompt>,
    /// 工具总表与角色表的能力面（**表本体在工具能力里**）。
    pub(crate) systools: Arc<dyn crate::capabilities::tools::api::Tools>,
    /// 本次调用的通道参数（流式 + 预算）：**全局设置**，与单 agent 共用同一份。
    pub(crate) llm: crate::capabilities::llm::api::LlmOpts,
    /// 讨论席的**机制说明 + 讨论约定**（开场与轮转都带它）：只说约定不说机制，AI 会空转。
    /// 能用哪些工具**不在这里**——随回合注入（见 MemberTools::tools_block）。
    pub(crate) protocol: String,

    /// 这一轮对每个成员**提醒过几次**（到顶就记"未回应"放过它）；每轮开始归零。
    pub(crate) reminded: std::collections::BTreeMap<String, u32>,
    /// 上面那份计数属于第几轮（轮次变了就清空）。
    pub(crate) remind_round: usize,
    /// 「停止」标志：由 CollabSession 注入（它从任务登记处拿到）。
    /// 泵在**每次调用前**与**调用中途**都看它——所以停止能在一个模型调用内收尾，而不是等它跑完。
    pub(crate) cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 开场提示词（渲染一次，开场阶段每个成员都用它）——由 start() 设。
    pub(crate) opener: String,
    /// 轮次推进的游标（可暂停：问到哪了）——见 advance/feed。
    pub(crate) cursor: Cursor,
    /// 本轮到此刻**还没交出去**的行数（轮次标记也算）：一个成员说完就把它那一批交出去。
    pub(crate) handed: usize,
}

/// 推进讨论的**一步**：该问谁（或终态）。
/// 泵只决定"该问谁"，**不自己调模型**——驱动权归核心（见 docs/session/session-model.md 二之二）。
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
pub(crate) enum Cursor {
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
    pub(crate) fn member_turn(
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
            // 讨论席只跑只读核实工具：不接工具级确认（要问也只会在执行席发生）。
            None,
            // 讨论席也没有可回答的前端：工具层要问就得靠"没有提问端口 = fail-closed 拒绝"。
            None,
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
    pub(crate) fn cancelled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// 本轮的调用选项：流式与预算都取全局设置（讨论也走同一份）。
    pub(crate) fn opts(&self) -> crate::capabilities::llm::api::CompleteOpts<'static> {
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
        // 驱动权将来归核心（见 docs/session/session-model.md 二之二）：那时把这一圈换成
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
    pub(crate) fn member_identity(&self, i: usize) -> String {
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

    pub(crate) fn absorb(
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

    /// 成员一轮之后的处置：**核心只提醒、不强制**（见 docs/session/session-model.md 二）。
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
    /// 见 docs/session/session-model.md 二"系统消息"。
    pub fn note_system(&mut self, text: &str) {
        self.transcript.push(LineView::system("", text.to_string()));
    }
}
