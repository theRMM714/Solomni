//! 单 agent 会话：上下文历史自有，通道来自注入的网关。
//! 发言席只有 agent：id = agent 实例名，既是说话人标签也是历史回放的依据。
//! 支持工具循环（联动 engine::converse_with）；转录行带会话内稳定 id（自 0 递增），
//! 并记录每行对应的历史长度，供回档精确回退。

use crate::core::engine::{MemberTools, Round};
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

/// 会话参数：拼请求要的**前提**（身份、环境）。**不是对话**——不占对话列表的位置，
/// 每次模型调用由驱动现渲染（见 `identity`）。
///
/// 为什么单独有个位置：参数是**派生**的（登记处 + 提示词册 + 工作区路径）。把它渲染成
/// "第 0 条消息"存进会话，就等于让参数冒充对话：回档、压缩、重建都得单独照顾那一条，
/// 参数一改还得把整个会话重建一遍。
#[derive(Debug, Clone)]
pub struct SessionParams {
    /// agent 实例名（身份块的 {{agent}}，也是重建时的命名依据）。
    pub agent: String,
    /// 工作名（环境块的 {{work_name}}）。
    pub work_name: String,
    /// 共享区（真实根）。
    pub shared: std::path::PathBuf,
    /// 该 agent 的私有沙箱（真实根）。
    pub private: std::path::PathBuf,
    /// 模块 id → 目录（环境块列出的模块目录）。
    pub module_dirs: std::collections::BTreeMap<String, std::path::PathBuf>,
    /// 模块能力包：id + 模块 system（顺序即装配顺序）。
    pub modules: Vec<(String, String)>,
}

impl SessionParams {
    /// 从（agent、沙箱、模块清单）装配：沙箱给真实根与模块目录，模块清单给能力包。
    /// 这是**唯一**的装配口径——会话的建立与重建都走它，参数不会两处各拼一套。
    pub fn from_workspace(
        agent: &str,
        sb: &crate::core::workspace::Sandbox,
        modules: &[crate::core::module::Module],
    ) -> SessionParams {
        SessionParams {
            agent: agent.to_string(),
            work_name: sb.work_name.clone(),
            shared: sb.shared.clone(),
            private: sb.private.clone(),
            module_dirs: sb.modules.clone(),
            modules: modules
                .iter()
                .map(|m| (m.manifest.id.clone(), m.manifest.system.clone()))
                .collect(),
        }
    }

    /// 身份块：**每次调用现渲染**（模板与文案取当前提示词册，通道形态取当前登记处）。
    pub fn identity(
        &self,
        prompts: &crate::core::prompt::Prompts,
        mode: crate::core::providers::ToolMode,
    ) -> String {
        let env = crate::core::systool::env_block(prompts, self);
        crate::core::module::agent_system(prompts, &self.agent, &self.modules, &env, mode)
    }
}

/// 一个 agent 的会话：模块数不限（形态只在校验与界面标签上区分）。
pub struct AgentSession {
    /// agent 实例名（说话人标签；重建时也按它命名）。
    id: String,
    /// 会话参数（派生的前提）：不占对话的位置，每次调用现渲染。
    params: SessionParams,
    /// **只有对话**：user / assistant / tool（+ 核心注入的系统消息）。
    /// 身份与环境不在这里——它们由 params 现渲染（见 docs/architecture/tools-and-roles.md 二）。
    dialogue: Vec<Msg>,
    chat: BoxedChat,
    note: Option<String>,
    /// 工具环境：内置文件工具按该 agent 的沙箱放行 + 该 agent 模块声明的外部工具。
    tools: Option<MemberTools>,
    /// @ 引用的说明文案（提示词册）；改写在入历史与转录之前做。
    refs: crate::core::prompt::RefsPrompts,
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
    /// 正在落行的**回合 id**（讨论的回合标记用它；单 agent 回合为 0）。
    cur_turn: u64,
    /// 自动压缩的**字符预算**（≈ 模型窗口 × 设置百分比 × 4）；0 = 关。
    /// 到点就在这一轮开始前先压一次（见 docs/architecture/session-model.md 六）。
    compact_at: usize,
}

impl AgentSession {
    /// 本会话的**对话**（讨论回合要把它带上：用户在这个会话里说的话，下一回合它就该记得）。
    /// 身份与环境不在里面——它们由 params 现渲染（见 SessionParams::identity）。
    pub fn dialogue(&self) -> &[crate::core::ports::Msg] {
        &self.dialogue
    }

    /// 会话参数：驱动据此现渲染身份块。
    pub fn params(&self) -> &SessionParams {
        &self.params
    }

    /// @ 改写要用的真实根：由参数派生，不另存一份。
    fn roots(&self) -> crate::core::refs::RefRoots {
        crate::core::refs::RefRoots {
            work: self.params.shared.clone(),
            private: Some(self.params.private.clone()),
        }
    }

    /// 讨论回合要**在这个会话里**跑：一次借出通道与工具环境。
    /// 为什么一次借两样：分开借会同时可变借用 self（编译不过），而它们本就是同一回合的两半。
    pub fn parts_mut(&mut self) -> (&mut dyn crate::core::ports::Chat, Option<&mut MemberTools>) {
        (self.chat.as_mut(), self.tools.as_mut())
    }

    // 组合根注入的构造函数：参数天然多，收口成参数对象只是把参数挪个地方、并让装配更难读。
    // 这是有意的设计取舍（见 docs/testing/quality-isolation.md 的 allow 清单），不是没修。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: &str,
        params: SessionParams,
        chat: BoxedChat,
        note: Option<String>,
        tools: Option<MemberTools>,
        refs: crate::core::prompt::RefsPrompts,
        tool_texts: crate::core::prompt::ToolTexts,
    ) -> AgentSession {
        AgentSession {
            cur_turn: 0,
            compact_at: 0,
            id: id.to_string(),
            params,
            dialogue: Vec::new(),
            chat,
            note,
            tools,
            refs,
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
        params: SessionParams,
        dialogue: Vec<Msg>,
        marks: Vec<usize>,
        line_reply: Vec<u64>,
        chat: BoxedChat,
        note: Option<String>,
        tools: Option<MemberTools>,
        refs: crate::core::prompt::RefsPrompts,
        tool_texts: crate::core::prompt::ToolTexts,
    ) -> AgentSession {
        AgentSession {
            cur_turn: 0,
            compact_at: 0,
            id: id.to_string(),
            next_line: marks.len() as u64,
            params,
            dialogue,
            chat,
            note,
            tools,
            refs,
            tool_texts,
            marks,
            line_reply,
            cur_reply: 0,
        }
    }

    /// 这条会话**正在用**的工具调用形态（身份块里的调用约定按它现渲染）。
    pub fn tool_mode(&self) -> crate::core::providers::ToolMode {
        self.tools.as_ref().map(|t| t.mode).unwrap_or_default()
    }

    /// 改形态：**只改这一格**（登记处派生出来的参数），不重建会话。
    pub fn set_tool_mode(&mut self, mode: crate::core::providers::ToolMode) {
        if let Some(t) = self.tools.as_mut() {
            t.mode = mode;
        }
    }

    /// 开场事件（通道回落告知）。
    pub fn open(&self) -> Vec<SessionEvent> {
        self.note
            .clone()
            .map(|n| vec![SessionEvent::Notice(n)])
            .unwrap_or_default()
    }

    /// 讨论回合的产出落进**本会话**：回合标记 + 核实行 + 它自己的发言。
    /// 一个 agent 的会话是它在这场工作里的完整经历（见 docs/architecture/session-model.md）：
    /// 主会话只留"谁说了什么"，核实（只读工具）的痕迹留在各自会话里。
    pub fn note_turn(
        &mut self,
        round: usize,
        turn_id: u64,
        verb: &str,
        text: &str,
        tools: &[crate::core::engine::DiscLine],
    ) -> Vec<SessionEvent> {
        // 这一回合的行都带上回合 id（回档时两边按它对上）。
        self.cur_turn = turn_id;
        let mut views = Vec::new();
        views.push(self.line(format!("[回合 t{}｜第 {} 轮]", turn_id, round), None, None));
        for l in tools {
            views.push(self.line(l.text.clone(), None, l.tool.clone()));
        }
        views.push(self.line(format!("[{}:{}] {}", self.id, verb, text), None, None));
        // 回合结束后清掉：后面的单 agent 回合各自另算。
        self.cur_turn = 0;
        vec![SessionEvent::Transcript(views)]
    }

    /// 压缩回合：把提示词追加到历史之后、**只声明 compact 工具**，跑一次模型；拿到摘要就返回。
    /// 两条通道都认：native 从结构化槽位取，信封通道从正文里的信封取（与讨论回合同口径）。
    pub fn compact_turn(
        &mut self,
        prompt: &str,
        decl: Option<&crate::core::ports::ToolDecl>,
        identity: &str,
    ) -> Result<String, String> {
        // 整条消息**只有这一处装配**：身份 + 本回合工具（只有 compact）+ 对话 + 压缩提示。
        let msgs = crate::core::engine::assemble(
            identity,
            self.tools.as_ref(),
            &["compact".to_string()],
            false,
            &self.dialogue,
            &[Msg::user(prompt.to_string())],
        );
        let mut opts = crate::core::ports::CompleteOpts::plain(false);
        if let Some(d) = decl {
            opts.tools = Some(std::slice::from_ref(d));
        }
        let mut keep = |_c: crate::core::ports::Chunk| true;
        let done = self.chat.complete(&msgs, opts, &mut keep);
        if let Some(err) = done.error {
            return Err(err);
        }
        let (name, args) = match done.calls.first() {
            Some(c) => (c.name.clone(), c.args_json.clone()),
            None => {
                let r = crate::core::envelope::parse(&done.raw);
                let t = r
                    .tools
                    .first()
                    .ok_or_else(|| "压缩回合没有调用 compact（没给出摘要）".to_string())?;
                (t.name.clone(), t.args_json.clone())
            }
        };
        if name != "compact" {
            return Err(format!("压缩回合该只调 compact，实际调了 {}", name));
        }
        let v: serde_json::Value = serde_json::from_str(&args)
            .map_err(|e| format!("压缩参数不合法（{}）：{}", e, args))?;
        let summary = v
            .get("summary")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        if summary.trim().is_empty() {
            return Err("压缩回合给出的摘要是空的".to_string());
        }
        Ok(summary)
    }

    /// 注入一条**系统消息**：上下文里是 system 角色，转录里是系统行。
    /// **系统/核心发的消息不得用用户身份**——无论进上下文还是进界面（见 session-model.md 二"系统消息"）。
    pub fn note_system(&mut self, text: &str) -> Vec<SessionEvent> {
        self.dialogue.push(Msg::system(text.to_string()));
        let v = self.line(text.to_string(), None, None);
        vec![SessionEvent::Transcript(vec![LineView {
            system: true,
            ..v
        }])]
    }

    /// 设自动压缩的字符预算（装配时按模型窗口 × 设置百分比算出来；0 = 关）。
    pub fn set_compact_budget(&mut self, chars: usize) {
        self.compact_at = chars;
    }

    /// 到点自动压一次：估算历史字符数（≈ tokens × 4），超预算就跑一个压缩回合。
    /// 压不动就**如实通知并继续用完整上下文**（不静默降级、不假装压过）。
    fn maybe_compact(&mut self, identity: &str, sink: &mut dyn FnMut(SessionEvent)) {
        if self.compact_at == 0 {
            return;
        }
        let chars: usize = self
            .dialogue
            .iter()
            .map(|m| m.content.chars().count())
            .sum();
        if chars <= self.compact_at {
            return;
        }
        let decl = self
            .tools
            .as_ref()
            .and_then(|t| t.sandbox.builtin_tools.get("compact"))
            .map(|s| s.decl("compact"));
        let prompt = self.tool_texts.compact_prompt.clone();
        let up_to = self.next_line;
        match self.compact_turn(&prompt, decl.as_ref(), identity) {
            Ok(summary) => {
                self.compact(up_to, &summary);
                sink(SessionEvent::Compacted { up_to, summary });
            }
            Err(err) => sink(SessionEvent::Notice(format!(
                "[警告] 自动压缩没成功：{}（继续用完整上下文）",
                err
            ))),
        }
    }

    /// 当前下一条转录行的 id（压缩点用它：把此前的行全部移出发送视图）。
    pub fn next_line_id(&self) -> u64 {
        self.next_line
    }

    /// 上下文压缩：把 up_to 之前的行移出**发送视图**（转录不动、用户照样能看），用一份摘要代替。
    /// 见 docs/architecture/session-model.md 六：改的是"发给模型什么"，不是"留下什么"。
    pub fn compact(&mut self, up_to: u64, summary: &str) {
        // 按行找到历史里的截断点（marks 就是"行 → 该行完成时的历史长度"）。
        let keep = self
            .marks
            .iter()
            .enumerate()
            .filter(|(i, _)| (*i as u64) < up_to)
            .count();
        let hist = if keep == 0 { 0 } else { self.marks[keep - 1] };
        self.dialogue.truncate(hist);
        // 摘要放在**对话最前面**（身份与环境由参数现渲染，不占对话的位置）：
        // 此后模型只看到"身份 + 摘要 + 之后的内容"。
        self.dialogue
            .insert(0, Msg::user(format!("[此前内容摘要]\n{}", summary)));
        // 插了一条消息，marks 里"行完成时的历史长度"整体后移一格。
        for m in self.marks.iter_mut() {
            *m += 1;
        }
    }

    /// 给接下来的行打上回合 id（节点执行也用整场工作的同一套计数：回档才对得上）。
    pub fn set_turn(&mut self, turn_id: u64) {
        self.cur_turn = turn_id;
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
            system: false,
            turn: self.cur_turn,
        };
        self.next_line += 1;
        self.line_reply.push(reply);
        self.marks.push(self.dialogue.len());
        v
    }

    /// 末条是否为用户发言（继续能不能直接发请求的判据）。
    pub fn last_is_user(&self) -> bool {
        matches!(self.dialogue.last().map(|m| m.role.as_str()), Some("user"))
    }

    /// 回档：只保留前 keep_id 行（= 删掉该行及其后）；对话与 marks 同步截断。
    /// keep_id = 0 → 转录清空，对话也清空（marks 同清）；身份与环境不在这里，不受影响。
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
            self.dialogue.clear();
            self.next_line = 0;
            return;
        }
        let hist = self.marks[keep - 1];
        self.marks.truncate(keep);
        self.dialogue.truncate(hist);
        self.next_line = keep as u64;
    }

    /// 发言：先把 @ 引用改写成寻址 → 压入用户消息 → 逐轮（文本行 / 工具行）落转录。
    /// 改写在这一处完成，所以转录行与进上下文的消息是同一份文本（转录即内容）。
    pub fn say(
        &mut self,
        text: &str,
        identity: &str,
        live: &mut Live,
        sink: &mut dyn FnMut(SessionEvent),
    ) {
        // 到点先压一次：**同一个工作线程内**跑，不阻塞核心。
        self.maybe_compact(identity, sink);
        let text = crate::core::refs::rewrite(text, Some(&self.id), &self.roots(), &self.refs);
        self.dialogue.push(Msg::user(text.clone()));
        // 用户行不属于任何模型回复：给它**自己的行号**当回复号（与重建时的规则一致），
        // 否则它会继承上一轮的回复号，回档时与上一轮误并成一组。
        self.cur_reply = self.next_line;
        let user_line = self.line(format!("[用户] {}", text), None, None);
        sink(SessionEvent::Transcript(vec![user_line]));
        self.rounds_events(identity, live, sink);
    }

    /// **系统注入**：上下文里是 system 角色，转录里是系统行，随后正常问模型。
    /// 系统/核心发的消息一律走这条（不得借用 say——那是用户发言）。
    pub fn inject_system(
        &mut self,
        text: &str,
        identity: &str,
        live: &mut Live,
        sink: &mut dyn FnMut(SessionEvent),
    ) {
        self.maybe_compact(identity, sink);
        self.dialogue.push(Msg::system(text.to_string()));
        // 系统行不属于任何模型回复：给它自己的行号当回复号（与重建规则一致）。
        self.cur_reply = self.next_line;
        let v = self.line(text.to_string(), None, None);
        sink(SessionEvent::Transcript(vec![LineView {
            system: true,
            ..v
        }]));
        self.rounds_events(identity, live, sink);
    }

    /// 继续：末条已是用户发言，直接用现有对话问模型（不新增用户消息）。
    pub fn continue_reply(
        &mut self,
        identity: &str,
        live: &mut Live,
        sink: &mut dyn FnMut(SessionEvent),
    ) {
        self.rounds_events(identity, live, sink);
    }

    /// 把一次问询的逐轮产出落成转录行：一轮的正文/思维链出文本行，工具另占一条工具行。
    /// marks 逐行精确（回档按行截断）；工具轮的文本行与工具行同属一轮，
    /// 所以历史统一在工具行推进（这一轮只贡献 assistant(raw) + [工具结果]），实时与重建两边一致。
    /// 逐轮外送：**一轮跑完就出这一轮的行**（以前攒到回合收尾才一次性出，工具轮会把上一轮的
    /// 流式文本从界面上抹掉）。行在回调里**只构造一次**；`run` 返回后只补记账——`marks` 是回档
    /// 依据，必须保持"文本行的 mark 在 text_msgs 之前、工具行的 mark 在两个 msgs 之后"这个原时序。
    fn rounds_events(
        &mut self,
        identity: &str,
        live: &mut Live,
        sink: &mut dyn FnMut(SessionEvent),
    ) {
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
        let rounds = self.run(identity, live, &mut on_round, sink);
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
                            self.marks.push(self.dialogue.len());
                        }
                    }
                    for m in &round.text_msgs {
                        self.dialogue.push(m.clone());
                    }
                    for m in &run.msgs {
                        self.dialogue.push(m.clone());
                    }
                    if let Some(v) = it.next() {
                        self.line_reply.push(v.reply);
                        self.marks.push(self.dialogue.len());
                    }
                }
                None => {
                    // 与旧逻辑同一时序：先扩展历史，再记这一行的 mark。
                    for m in &round.text_msgs {
                        self.dialogue.push(m.clone());
                    }
                    if let Some(v) = it.next() {
                        self.line_reply.push(v.reply);
                        self.marks.push(self.dialogue.len());
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

    /// 以现有对话跑一次工具循环；流式时逐片外送短暂 Delta（信封正文不外流，避免糊屏）。
    /// identity = 本回合的身份块（由驱动按当前提示词册现渲染；不进对话）。
    fn run(
        &mut self,
        identity: &str,
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
                dialogue,
                chat,
                tools,
                ..
            } = self;
            crate::core::engine::converse_with(
                chat.as_mut(),
                tools.as_mut(),
                identity,
                dialogue.clone(),
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
            system: false,
            turn: round.reply, // 单 agent 的每一轮各成"回合"（回档按它对齐）
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
