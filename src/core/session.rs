//! 单 agent 会话：上下文历史自有，通道来自注入的网关。
//! 发言席只有 agent：id = agent 实例名，既是说话人标签也是历史回放的依据。
//! 支持工具循环（联动 engine::converse_with）；转录行带会话内稳定 id（自 0 递增），
//! 并记录每行对应的历史长度，供回档精确回退。

use crate::core::engine::{MemberTools, Round, MAX_TOOL_CALLS};
use crate::core::events::{LineView, Live, SessionEvent, ToolCallView};
use crate::core::ports::Chunk;
use crate::core::ports::{BoxedChat, Msg};

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
}

impl AgentSession {
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
        AgentSession { id: id.to_string(), history: vec![Msg::system(system)], chat, note, tools, refs, roots, tool_texts, next_line: 0, marks: Vec::new() }
    }

    /// 从落盘事件重建（继续/回档历史会话用）。
    pub fn restore(
        id: &str,
        history: Vec<Msg>,
        marks: Vec<usize>,
        chat: BoxedChat,
        note: Option<String>,
        tools: Option<MemberTools>,
        refs: crate::core::prompt::RefsPrompts,
        roots: crate::core::refs::RefRoots,
        tool_texts: crate::core::prompt::ToolTexts,
    ) -> AgentSession {
        AgentSession { id: id.to_string(), next_line: marks.len() as u64, history, chat, note, tools, refs, roots, tool_texts, marks }
    }

    /// 开场事件（通道回落告知）。
    pub fn open(&self) -> Vec<SessionEvent> {
        self.note.clone().map(|n| vec![SessionEvent::Notice(n)]).unwrap_or_default()
    }

    /// 生成一条转录行，并记下它完成时的历史长度（回档按 marks 逐行精确回退）。
    fn line(&mut self, line: String, reasoning: Option<String>, tool: Option<ToolCallView>) -> LineView {
        let v = LineView { id: self.next_line, line, reasoning, tool, degraded: false };
        self.next_line += 1;
        self.marks.push(self.history.len());
        v
    }

    /// 末条是否为用户发言（继续能不能直接发请求的判据）。
    pub fn last_is_user(&self) -> bool {
        matches!(self.history.last().map(|m| m.role.as_str()), Some("user"))
    }

    /// 回档：只保留前 keep_id 行（= 删掉该行及其后）；历史与 marks 同步截断。
    /// keep_id = 0 → 转录清空，历史只剩 system（marks 也清空）。
    pub fn rewind(&mut self, keep_id: u64) {
        // 回档把转录截掉了：那段"我完整读过哪些文件"的读取证据随之作废（保守，宁肯让模型重读）。
        if let Some(t) = self.tools.as_mut() {
            t.observations.clear();
        }
        let keep = (keep_id as usize).min(self.marks.len());
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
    pub fn say(&mut self, text: &str, live: &mut Live) -> Vec<SessionEvent> {
        let text = crate::core::refs::rewrite(text, Some(&self.id), &self.roots, &self.refs);
        self.history.push(Msg::user(text.clone()));
        let user_line = self.line(format!("[用户] {}", text), None, None);
        let mut out: Vec<SessionEvent> = vec![SessionEvent::Transcript(vec![user_line])];
        out.extend(self.rounds_events(live));
        out
    }

    /// 继续：末条已是用户发言，直接用现有历史问模型（不新增用户消息）。
    pub fn continue_reply(&mut self, live: &mut Live) -> Vec<SessionEvent> {
        self.rounds_events(live)
    }

    /// 把一次问询的逐轮产出落成转录行：一轮的正文/思维链出文本行，工具另占一条工具行。
    /// marks 逐行精确（回档按行截断）；工具轮的文本行与工具行同属一轮，
    /// 所以历史统一在工具行推进（这一轮只贡献 assistant(raw) + [工具结果]），实时与重建两边一致。
    fn rounds_events(&mut self, live: &mut Live) -> Vec<SessionEvent> {
        let rounds = self.run(live);
        let stopped = live.cancelled();
        let mut out: Vec<SessionEvent> = Vec::new();
        for round in rounds {
            let text = round.text.trim().to_string();
            let has_line = !text.is_empty() || !round.reasoning.trim().is_empty();
            let mut reasoning = if round.reasoning.trim().is_empty() { None } else { Some(round.reasoning.clone()) };
            match round.tool {
                Some(run) => {
                    // 先出「思考+正文」文本行（只有信封没有正文/思维链时不出空行）。
                    if has_line {
                        let mut line = format!("[{}]", self.id);
                        if !text.is_empty() {
                            line.push(' ');
                            line.push_str(&text);
                        }
                        if stopped {
                            line.push_str(&self.tool_texts.stopped_suffix);
                        }
                        out.push(SessionEvent::Transcript(vec![self.line(line, reasoning.take(), None)]));
                    }
                    for m in round.text_msgs {
                        self.history.push(m);
                    }
                    for m in run.msgs {
                        self.history.push(m);
                    }
                    let status = if run.view.ok { "成功" } else { "失败" };
                    // 没有文本行时思维链挂到工具行上，不丢。
                    let line = format!("[{}] 工具 {} → {}", self.id, run.view.label(), status);
                    out.push(SessionEvent::Transcript(vec![self.line(line, reasoning.take(), Some(run.view))]));
                }
                None => {
                    for m in round.text_msgs {
                        self.history.push(m);
                    }
                    if has_line {
                        let mut line = format!("[{}]", self.id);
                        if !text.is_empty() {
                            line.push(' ');
                            line.push_str(&text);
                        }
                        if stopped {
                            line.push_str(&self.tool_texts.stopped_suffix);
                        }
                        out.push(SessionEvent::Transcript(vec![self.line(line, reasoning.take(), None)]));
                    }
                }
            }
        }
        if stopped {
            out.push(SessionEvent::Notice("[已停止] 生成已按你的要求中止（保留已产出的部分）".to_string()));
        }
        out
    }

    /// 以现有历史跑一次工具循环；流式时逐片外送短暂 Delta（信封正文不外流，避免糊屏）。
    fn run(&mut self, live: &mut Live) -> Vec<Round> {
        let label = self.id.clone();
        let stream = live.stream;
        let cancel = std::sync::Arc::clone(&live.cancel);
        // 两个回调（流式分片 / 工具完成）都要外送短暂事件：把 emit 借出来共享（顺序因此天然正确）。
        let emit = std::cell::RefCell::new(&mut *live.emit);
        let mut acc = String::new();
        {
            let AgentSession { history, chat, tools, .. } = self;
            crate::core::engine::converse_with(
                chat.as_mut(),
                tools.as_mut(),
                history.clone(),
                stream,
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
                    (emit.borrow_mut())(SessionEvent::Delta { speaker: label.clone(), kind: kind.to_string(), text: piece });
                    !cancel.load(std::sync::atomic::Ordering::Relaxed)
                },
                &mut |view: &ToolCallView| {
                    (emit.borrow_mut())(SessionEvent::ToolCall(view.clone()));
                },
            )
        }
    }
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
