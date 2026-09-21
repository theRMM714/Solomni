//! 单 agent 会话：上下文历史自有，通道来自注入的网关。
//! 发言席只有 agent：id = agent 实例名，既是说话人标签也是历史回放的依据。
//! 支持工具循环（联动 engine::converse_with）；转录行带会话内稳定 id（自 0 递增），
//! 并记录每行对应的历史长度，供回档精确回退。

use crate::core::engine::{MemberTools, Round, MAX_TOOL_CALLS};
use crate::core::events::{LineView, Live, SessionEvent, ToolCallView};
use crate::core::ports::Chunk;
use crate::core::ports::{BoxedChat, Msg};

/// 把"保留前 keep 行"对齐到**回复边界**：keep 落在某次回复内部时，退到该回复的第一行之前。
///
/// 为什么必须对齐：一次回复的消息是「一条助手消息 + N 条结果」，截在中间会留下孤儿结果
/// （协议要求结果紧跟发起它的助手消息）。转录的截断与内存历史的截断**必须用同一个函数**，
/// 否则前端看到的事件流与模型上下文会不一致。
pub(crate) fn keep_whole_replies(line_reply: &[u64], keep: usize) -> usize {
    let mut keep = keep.min(line_reply.len());
    if keep > 0 && keep < line_reply.len() {
        let losing = line_reply[keep];
        while keep > 0 && line_reply[keep - 1] == losing {
            keep -= 1;
        }
    }
    keep
}

/// 一个 agent 的会话：模块数不限（形态只在校验与界面标签上区分）。
pub struct AgentSession {
    /// agent 实例名（说话人标签；重建时也按它命名）。
    id: String,
    history: Vec<Msg>,
    chat: BoxedChat,
    note: Option<String>,
    /// 工具环境：内置文件工具按该 agent 的沙箱放行 + 该 agent 模块声明的外部工具。
    tools: Option<MemberTools>,
    /// @ 引用的说明文案（提示词册）；改写在入历史与转录之前做。
    refs: crate::core::prompt::RefsPrompts,
    /// @ 改写要用的真实根（本工作共享区 + 自己的私有沙箱）。
    roots: crate::core::refs::RefRoots,
    /// 模型侧运行时文案（提示词册）；本会话要用的那几条。
    tool_texts: crate::core::prompt::ToolTexts,
    /// 下一条转录行的 id。
    next_line: u64,
    /// 每行 id 对应「该行完成时的历史长度」，回档按它截断历史。
    marks: Vec<usize>,
    /// 每行属于哪次模型回复（id 相同 = 同一次回复）。回档**按回复原子**截断靠它：
    /// 截在一次回复中间会留下"孤儿工具结果"，而协议要求结果紧跟发起它的那条助手消息。
    line_reply: Vec<u64>,
    /// 正在落行的回复 id（每轮开始时设置；line() 用它，免得每个调用点都传一遍）。
    cur_reply: u64,
}

impl AgentSession {
    // 组合根注入的构造函数：参数天然多，收口成参数对象只是把参数挪个地方、并让装配更难读。
    // 这是有意的设计取舍（见 docs/testing/quality-isolation.md 的 allow 清单），不是没修。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: &str,
        system: String,
        chat: BoxedChat,
        note: Option<String>,
        tools: Option<MemberTools>,
        refs: crate::core::prompt::RefsPrompts,
        roots: crate::core::refs::RefRoots,
        tool_texts: crate::core::prompt::ToolTexts,
    ) -> AgentSession {
        AgentSession {
            id: id.to_string(),
            history: vec![Msg::system(system)],
            chat,
            note,
            tools,
            refs,
            roots,
            tool_texts,
            next_line: 0,
            marks: Vec::new(),
            line_reply: Vec::new(),
            cur_reply: 0,
        }
    }

    /// 从落盘事件重建（继续/回档历史会话用）。
    // 组合根注入的构造函数：参数天然多，收口成参数对象只是把参数挪个地方、并让装配更难读。
    // 这是有意的设计取舍（见 docs/testing/quality-isolation.md 的 allow 清单），不是没修。
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        id: &str,
        history: Vec<Msg>,
        marks: Vec<usize>,
        line_reply: Vec<u64>,
        chat: BoxedChat,
        note: Option<String>,
        tools: Option<MemberTools>,
        refs: crate::core::prompt::RefsPrompts,
        roots: crate::core::refs::RefRoots,
        tool_texts: crate::core::prompt::ToolTexts,
    ) -> AgentSession {
        AgentSession {
            id: id.to_string(),
            next_line: marks.len() as u64,
            history,
            chat,
            note,
            tools,
            refs,
            roots,
            tool_texts,
            marks,
            line_reply,
            cur_reply: 0,
        }
    }

    /// 这条会话**正在用**的工具调用形态（系统提示就是按它拼的）。
    pub fn tool_mode(&self) -> crate::core::providers::ToolMode {
        self.tools.as_ref().map(|t| t.mode).unwrap_or_default()
    }

    /// 开场事件（通道回落告知）。
    pub fn open(&self) -> Vec<SessionEvent> {
        self.note
            .clone()
            .map(|n| vec![SessionEvent::Notice(n)])
            .unwrap_or_default()
    }

    /// 生成一条转录行，并记下它完成时的历史长度（回档按 marks 逐行精确回退）与它属于哪次回复。
    fn line(
        &mut self,
        line: String,
        reasoning: Option<String>,
        tool: Option<ToolCallView>,
    ) -> LineView {
        let reply = self.cur_reply;
        let v = LineView {
            id: self.next_line,
            reply,
            line,
            reasoning,
            tool,
            degraded: false,
        };
        self.next_line += 1;
        self.line_reply.push(reply);
        self.marks.push(self.history.len());
        v
    }

    /// 末条是否为用户发言（继续能不能直接发请求的判据）。
    pub fn last_is_user(&self) -> bool {
        matches!(self.history.last().map(|m| m.role.as_str()), Some("user"))
    }

    /// 回档：只保留前 keep_id 行（= 删掉该行及其后）；历史与 marks 同步截断。
    /// keep_id = 0 → 转录清空，历史只剩 system（marks 也清空）。
    /// **按回复原子**：截在一次回复内部会留下"孤儿工具结果"（协议要求结果紧跟发起它的助手消息），
    /// 所以 keep_id 落在某次回复中间时，这条回复整条丢掉（退到它的第一行之前）。
    pub fn rewind(&mut self, keep_id: u64) {
        // 回档把转录截掉了：那段"我完整读过哪些文件"的读取证据随之作废（保守，宁肯让模型重读）。
        if let Some(t) = self.tools.as_mut() {
            t.observations.clear();
        }
        let keep = keep_whole_replies(&self.line_reply, keep_id as usize);
        if keep == 0 {
            self.marks.clear();
            self.history.truncate(1);
            self.next_line = 0;
            return;
        }
        let hist = self.marks[keep - 1];
        self.marks.truncate(keep);
        self.history.truncate(hist);
        self.next_line = keep as u64;
    }

    /// 发言：先把 @ 引用改写成寻址 → 压入用户消息 → 逐轮（文本行 / 工具行）落转录。
    /// 改写在这一处完成，所以转录行与进上下文的消息是同一份文本（转录即内容）。
    pub fn say(&mut self, text: &str, live: &mut Live, sink: &mut dyn FnMut(SessionEvent)) {
        let text = crate::core::refs::rewrite(text, Some(&self.id), &self.roots, &self.refs);
        self.history.push(Msg::user(text.clone()));
        // 用户行不属于任何模型回复：给它**自己的行号**当回复号（与重建时的规则一致），
        // 否则它会继承上一轮的回复号，回档时与上一轮误并成一组。
        self.cur_reply = self.next_line;
        let user_line = self.line(format!("[用户] {}", text), None, None);
        sink(SessionEvent::Transcript(vec![user_line]));
        self.rounds_events(live, sink);
    }

    /// 继续：末条已是用户发言，直接用现有历史问模型（不新增用户消息）。
    pub fn continue_reply(&mut self, live: &mut Live, sink: &mut dyn FnMut(SessionEvent)) {
        self.rounds_events(live, sink);
    }

    /// 把一次问询的逐轮产出落成转录行：一轮的正文/思维链出文本行，工具另占一条工具行。
    /// marks 逐行精确（回档按行截断）；工具轮的文本行与工具行同属一轮，
    /// 所以历史统一在工具行推进（这一轮只贡献 assistant(raw) + [工具结果]），实时与重建两边一致。
    /// 逐轮外送：**一轮跑完就出这一轮的行**（以前攒到回合收尾才一次性出，工具轮会把上一轮的
    /// 流式文本从界面上抹掉）。行在回调里**只构造一次**；`run` 返回后只补记账——`marks` 是回档
    /// 依据，必须保持"文本行的 mark 在 text_msgs 之前、工具行的 mark 在两个 msgs 之后"这个原时序。
    fn rounds_events(&mut self, live: &mut Live, sink: &mut dyn FnMut(SessionEvent)) {
        let label = self.id.clone();
        let texts = self.tool_texts.clone();
        let stopped = live.cancelled();
        let next_line = std::cell::Cell::new(self.next_line);
        let per_round: std::cell::RefCell<Vec<Vec<LineView>>> = std::cell::RefCell::new(Vec::new());
        let error: std::cell::RefCell<Option<String>> = std::cell::RefCell::new(None);
        let mut on_round = |round: &Round, s: &mut dyn FnMut(SessionEvent)| {
            if let Some(err) = round.error.clone() {
                *error.borrow_mut() = Some(err);
                return;
            }
            let views = build_round_lines(&label, &texts, round, stopped, &next_line);
            if !views.is_empty() {
                s(SessionEvent::Transcript(views.clone()));
            }
            per_round.borrow_mut().push(views);
        };
        let rounds = self.run(live, &mut on_round, sink);
        self.next_line = next_line.get();

        // 只补记账（不再构造行、不再外送）：顺序与旧逻辑逐字对应。
        for (round, views) in rounds.iter().zip(per_round.into_inner()) {
            if error.borrow().is_some() {
                break;
            }
            self.cur_reply = round.reply;
            let has_line = !round.text.trim().is_empty() || !round.reasoning.trim().is_empty();
            let mut it = views.into_iter();
            match &round.tool {
                Some(run) => {
                    if has_line {
                        if let Some(v) = it.next() {
                            self.line_reply.push(v.reply);
                            self.marks.push(self.history.len());
                        }
                    }
                    for m in &round.text_msgs {
                        self.history.push(m.clone());
                    }
                    for m in &run.msgs {
                        self.history.push(m.clone());
                    }
                    if let Some(v) = it.next() {
                        self.line_reply.push(v.reply);
                        self.marks.push(self.history.len());
                    }
                }
                None => {
                    // 与旧逻辑同一时序：先扩展历史，再记这一行的 mark。
                    for m in &round.text_msgs {
                        self.history.push(m.clone());
                    }
                    if let Some(v) = it.next() {
                        self.line_reply.push(v.reply);
                        self.marks.push(self.history.len());
                    }
                }
            }
        }
        if let Some(err) = error.borrow().as_ref() {
            sink(SessionEvent::Notice(crate::core::events::interrupted_note(
                err,
            )));
        }
        if stopped {
            sink(SessionEvent::Notice(
                "[已停止] 生成已按你的要求中止（保留已产出的部分）".to_string(),
            ));
        }
    }

    /// 以现有历史跑一次工具循环；流式时逐片外送短暂 Delta（信封正文不外流，避免糊屏）。
    fn run(
        &mut self,
        live: &mut Live,
        on_round: &mut crate::core::engine::RoundSink<'_>,
        sink: &mut dyn FnMut(SessionEvent),
    ) -> Vec<Round> {
        let label = self.id.clone();
        let llm = live.llm;
        let cancel = std::sync::Arc::clone(&live.cancel);
        // 两个回调（流式分片 / 工具完成）都要外送短暂事件：把 emit 借出来共享（顺序因此天然正确）。
        let emit = std::cell::RefCell::new(&mut *live.emit);
        let mut acc = String::new();
        {
            let AgentSession {
                history,
                chat,
                tools,
                ..
            } = self;
            crate::core::engine::converse_with(
                chat.as_mut(),
                tools.as_mut(),
                history.clone(),
                llm,
                &label,
                &mut |chunk| {
                    let mut kind = "text";
                    let mut piece = String::new();
                    match &chunk {
                        Chunk::Start => {
                            acc.clear();
                            kind = "start";
                        }
                        Chunk::Text(t) => {
                            let (send, next) = stream_piece(&acc, t);
                            piece = send;
                            acc = next;
                        }
                        Chunk::Reasoning(r) => {
                            kind = "reasoning";
                            piece = r.clone();
                        }
                    }
                    (emit.borrow_mut())(SessionEvent::Delta {
                        speaker: label.clone(),
                        kind: kind.to_string(),
                        text: piece,
                    });
                    !cancel.load(std::sync::atomic::Ordering::Relaxed)
                },
                &mut |view: &ToolCallView| {
                    (emit.borrow_mut())(SessionEvent::ToolCall(view.clone()));
                },
                on_round,
                sink,
            )
        }
    }
}

/// 一轮的转录行：文本行（有正文/思维链时）+ 工具行（有工具时）。
/// **不依赖 `&mut self`**：它由逐轮回调在 `converse_with` 内部调用，那时 `self` 已被拆开。
/// 行号从 `next_line` 递增（回调里记不了账，所以由调用方在回合收尾时按同一批行补 marks）。
fn build_round_lines(
    id: &str,
    texts: &crate::core::prompt::ToolTexts,
    round: &Round,
    stopped: bool,
    next_line: &std::cell::Cell<u64>,
) -> Vec<LineView> {
    let text = round.text.trim().to_string();
    let has_line = !text.is_empty() || !round.reasoning.trim().is_empty();
    let truncated = round.truncated();
    let mut reasoning = if round.reasoning.trim().is_empty() {
        None
    } else {
        Some(round.reasoning.clone())
    };
    let make = |line: String, reasoning: Option<String>, tool: Option<ToolCallView>| {
        let num = next_line.get();
        next_line.set(num + 1);
        LineView {
            id: num,
            reply: round.reply,
            line,
            reasoning,
            tool,
            degraded: false,
        }
    };
    let mut out = Vec::new();
    let text_line = |reasoning: &mut Option<String>, out: &mut Vec<LineView>| {
        if !has_line {
            return;
        }
        let mut line = format!("[{}]", id);
        if !text.is_empty() {
            line.push(' ');
            line.push_str(&text);
        }
        if stopped {
            line.push_str(&texts.stopped_suffix);
        }
        if truncated {
            line.push_str(&texts.truncated_suffix);
        }
        out.push(make(line, reasoning.take(), None));
    };
    match &round.tool {
        Some(run) => {
            // 先出「思考+正文」文本行（只有信封没有正文/思维链时不出空行）。
            text_line(&mut reasoning, &mut out);
            let status = if run.view.ok { "成功" } else { "失败" };
            // 没有文本行时思维链挂到工具行上，不丢。
            out.push(make(
                format!("[{}] 工具 {} → {}", id, run.view.label(), status),
                reasoning.take(),
                Some(run.view.clone()),
            ));
        }
        None => text_line(&mut reasoning, &mut out),
    }
    out
}

/// 流式外送规则：信封之前照常外送，一旦累积文本里出现 "{" 就不再外送后续片段
/// （模型可能在同一轮里先写正文再发 tool 信封——信封绝不能当正文流上屏）。
/// 返回（本片可外送的部分, 新的累积文本）。纯函数，便于单测。
pub(crate) fn stream_piece(acc: &str, piece: &str) -> (String, String) {
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

// 测试访问器：验证职责提示词已入历史首条（回归：会话曾丢失 system 提示词）。
#[cfg(test)]
impl AgentSession {
    pub fn history(&self) -> &[Msg] {
        &self.history
    }
}

// MAX_TOOL_CALLS 供引擎循环使用；此处引用以保持常量归属清晰。
const _: () = {
    let _ = MAX_TOOL_CALLS;
};
