//! 单 agent 会话：上下文历史自有，通道来自注入的网关。
//! 发言席只有 agent：id = agent 实例名，既是说话人标签也是历史回放的依据。
//! 支持工具循环（联动 engine::converse_with）；转录行带会话内稳定 id（自 0 递增），
//! 并记录每行对应的历史长度，供回档精确回退。

use crate::capabilities::llm::api::{BoxedChat, Msg};
use crate::capabilities::session::domain::events::{LineView, SessionEvent, ToolCallView};
use crate::capabilities::session::domain::tools::MemberTools;

/// 把"保留前 keep 行"对齐到**回复边界**：keep 落在某次回复内部时，退到该回复的第一行之前。
///
/// 为什么必须对齐：一次回复的消息是「一条助手消息 + N 条结果」，截在中间会留下孤儿结果
/// （协议要求结果紧跟发起它的助手消息）。转录的截断与内存历史的截断**必须用同一个函数**，
/// 否则前端看到的事件流与模型上下文会不一致。
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
        sb: &crate::capabilities::workspace::api::Sandbox,
        modules: &[crate::capabilities::workspace::api::Module],
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
        prompt: &dyn crate::capabilities::prompt::api::Prompt,
        mode: crate::capabilities::llm::api::ToolMode,
    ) -> String {
        let env = env_block(prompt, self);
        crate::capabilities::workspace::api::agent_system(
            prompt,
            &self.agent,
            &self.modules,
            &env,
            mode,
        )
    }
}

/// **工作环境块**：提示词册 env 渲染（真实根目录 + 路径规矩）。**不含任何工具清单**——
/// 能用哪些工具由核心按这一回合的身份现渲染后随回合注入（见 collab 的 `MemberTools::tools_block`）。
///
/// 归属：它渲染的就是 `SessionParams`，所以随会话走（留在 `tools` 会让 `tools → session` 成环）。
pub fn env_block(
    prompt: &dyn crate::capabilities::prompt::api::Prompt,
    p: &SessionParams,
) -> String {
    use crate::capabilities::prompt::api::Segment;
    let texts = prompt.tools();
    let module_roots = if p.module_dirs.is_empty() {
        prompt.text(Segment::NoModuleDirs).to_string()
    } else {
        p.module_dirs
            .iter()
            .map(|(id, root)| {
                texts.render(
                    &texts.module_root_line,
                    &[
                        ("id", id.clone()),
                        ("root", crate::kernel::api::slash(root)),
                    ],
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    prompt.render(
        Segment::Env,
        &[
            ("work_name", p.work_name.clone()),
            ("agent", p.agent.clone()),
            ("work_root", crate::kernel::api::slash(&p.shared)),
            ("sandbox_root", crate::kernel::api::slash(&p.private)),
            ("module_roots", module_roots),
        ],
    )
}

/// 跑**一个回合**要的那几样（单 agent / 节点 / 讨论成员共用同一条轮循环，差别只在这里）：
/// 身份块（谁）、工具面（能调什么）、本回合提示（说什么）、表态约定、回合号（行按它对上）。
pub struct TurnRun<'a> {
    /// 本回合的身份块（驱动按当前提示词册现渲染；不进对话）。
    pub identity: &'a str,
    /// 本回合的工具面（角色表发放的 id + 是否给它自己模块的工具）；None = 会话自己的（执行席）。
    /// 讨论席按回合切面：同一个 agent 会话会用两种身份干活（说话 / 干活）。
    pub face: Option<(Vec<String>, bool)>,
    /// 本回合的提示（讨论席的开场/轮转词；执行席常为空——它的指令在派发行里）。
    pub turn: Vec<Msg>,
    /// 本回合认不认**协作表态**（讨论席认；执行席不认）。
    pub verbs: bool,
    /// 本回合的回合号（转录行按它对上）；None = 用该轮自己的回复号（单 agent 的每轮各成回合）。
    pub turn_id: Option<u64>,
}

/// 一个 agent 的会话：模块数不限（形态只在校验与界面标签上区分）。
pub struct AgentSession {
    /// agent 实例名（说话人标签；重建时也按它命名）。
    pub(crate) id: String,
    /// 会话参数（派生的前提）：不占对话的位置，每次调用现渲染。
    params: SessionParams,
    /// **只有对话**：user / assistant / tool（+ 核心注入的系统消息）。
    /// 身份与环境不在这里——它们由 params 现渲染（见 docs/tools/tools-and-roles.md 二）。
    pub(crate) dialogue: Vec<Msg>,
    pub(crate) chat: BoxedChat,
    note: Option<String>,
    /// 工具环境：内置文件工具按该 agent 的沙箱放行 + 该 agent 模块声明的外部工具。
    pub(crate) tools: Option<MemberTools>,
    /// @ 引用的说明文案（提示词册）；改写在入历史与转录之前做。**共享一份**（不再每会话深拷贝）。
    pub(crate) refs: std::sync::Arc<crate::capabilities::prompt::api::RefsPrompts>,
    /// 模型侧运行时文案（提示词册）；**共享一份**。
    pub(crate) tool_texts: std::sync::Arc<crate::capabilities::prompt::api::ToolTexts>,
    /// 下一条转录行的 id。
    pub(crate) next_line: u64,
    /// 每行 id 对应「该行完成时的历史长度」，回档按它截断历史。
    pub(crate) marks: Vec<usize>,
    /// 每行属于哪次模型回复（id 相同 = 同一次回复）。回档**按回复原子**截断靠它：
    /// 截在一次回复中间会留下"孤儿工具结果"，而协议要求结果紧跟发起它的那条助手消息。
    pub(crate) line_reply: Vec<u64>,
    /// 正在落行的回复 id（每轮开始时设置；line() 用它，免得每个调用点都传一遍）。
    pub(crate) cur_reply: u64,
    /// 正在落行的**回合 id**（讨论的回合标记用它；单 agent 回合为 0）。
    cur_turn: u64,
    /// 自动压缩的**字符预算**（≈ 模型窗口 × 设置百分比 × 4）；0 = 关。
    /// 到点就在这一轮开始前先压一次（见 docs/session/session-model.md 六）。
    pub(crate) compact_at: usize,
    /// 压缩点（转录行 id）：0 = 没压过。此前的内容已被摘要取代，对话里补不回来。
    pub(crate) compacted_upto: u64,
}

/// 摘要消息（发送视图里是 **user 角色**）：`compact` 与重建共用一份口径，实时与回放才逐条相同。
pub fn summary_message(summary: &str) -> Msg {
    Msg::user(format!("[此前内容摘要]\n{}", summary))
}

impl AgentSession {
    /// 本会话的**对话**（测试据此断言"用户说过的话，下一回合带上了"）。
    /// 身份与环境不在里面——它们由 params 现渲染（见 SessionParams::identity）。
    #[cfg(test)]
    pub fn dialogue(&self) -> &[crate::capabilities::llm::api::Msg] {
        &self.dialogue
    }

    /// 会话参数：驱动据此现渲染身份块。
    pub fn params(&self) -> &SessionParams {
        &self.params
    }

    /// @ 改写要用的真实根：由参数派生，不另存一份。
    pub(crate) fn roots(&self) -> crate::capabilities::prompt::api::RefRoots {
        crate::capabilities::prompt::api::RefRoots {
            work: self.params.shared.clone(),
            private: Some(self.params.private.clone()),
        }
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
        refs: std::sync::Arc<crate::capabilities::prompt::api::RefsPrompts>,
        tool_texts: std::sync::Arc<crate::capabilities::prompt::api::ToolTexts>,
    ) -> AgentSession {
        AgentSession {
            cur_turn: 0,
            compact_at: 0,
            compacted_upto: 0,
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
        compacted_upto: u64,
        chat: BoxedChat,
        note: Option<String>,
        tools: Option<MemberTools>,
        refs: std::sync::Arc<crate::capabilities::prompt::api::RefsPrompts>,
        tool_texts: std::sync::Arc<crate::capabilities::prompt::api::ToolTexts>,
    ) -> AgentSession {
        AgentSession {
            cur_turn: 0,
            compact_at: 0,
            compacted_upto,
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
    pub fn tool_mode(&self) -> crate::capabilities::llm::api::ToolMode {
        self.tools.as_ref().map(|t| t.mode).unwrap_or_default()
    }

    /// 改形态：**只改这一格**（登记处派生出来的参数），不重建会话。
    pub fn set_tool_mode(&mut self, mode: crate::capabilities::llm::api::ToolMode) {
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

    /// 注入一条**系统消息**（提醒/边界这类"不是用户说的、也不是模型说的"内容）：
    /// 上下文里是 system 角色，转录里是系统行。
    /// 它**不得在界面与记录里长得像用户发的**——身份是核心，界面按系统行样式（见 session-model.md 二）。
    pub fn note_system(&mut self, text: &str) -> Vec<SessionEvent> {
        self.dialogue.push(Msg::system(text.to_string()));
        let v = self.line(text.to_string(), "system", "", "", None, None);
        vec![SessionEvent::Transcript(vec![v])]
    }

    /// 注入一条**派发任务**（核心给这个 agent 派的活，见 session-model.md 四之二）。
    /// 界面：**系统行**——说话人是核心，不冒充用户；上下文：**user 角色**——派活是一次"回合"，
    /// 会话协议要求请求里至少有一条 user 消息（全是 system 的请求会被供应商整条拒收）。
    /// 两处口径随行落档（`system` + `task`），重建/回放才产得出**同一条**消息。
    pub fn note_task(&mut self, text: &str) -> Vec<SessionEvent> {
        self.dialogue.push(Msg::user(text.to_string()));
        // 派发行不属于任何模型回复：给它自己的行号当回复号（与重建规则一致）。
        self.cur_reply = self.next_line;
        let v = self.line(text.to_string(), "system", "", "", None, None);
        vec![SessionEvent::Transcript(vec![LineView { task: true, ..v }])]
    }

    /// 设自动压缩的字符预算（装配时按模型窗口 × 设置百分比算出来；0 = 关）。
    pub fn set_compact_budget(&mut self, chars: usize) {
        self.compact_at = chars;
    }

    /// 压缩点（转录行 id）：0 = 没压过。回档到它之前，被销毁的对话在内存里补不回来。
    pub fn compacted_upto(&self) -> u64 {
        self.compacted_upto
    }

    /// 当前下一条转录行的 id（压缩点用它：把此前的行全部移出发送视图）。
    pub fn next_line_id(&self) -> u64 {
        self.next_line
    }

    /// 上下文压缩：把 up_to 之前的行移出**发送视图**（转录不动、用户照样能看），用一份摘要代替。
    /// 见 docs/session/session-model.md 六：改的是"发给模型什么"，不是"留下什么"。
    pub fn compact(&mut self, up_to: u64, summary: &str) {
        // 按行找到历史里的截断点（marks 就是"行 → 该行完成时的历史长度"）。
        let keep = self
            .marks
            .iter()
            .enumerate()
            .filter(|(i, _)| (*i as u64) < up_to)
            .count();
        // 被总结掉的那一段要**移出**发送视图——不是保留前缀（保留前缀会把新内容丢掉）。
        let cut = if keep == 0 { 0 } else { self.marks[keep - 1] };
        self.dialogue.drain(0..cut);
        // 摘要放在**对话最前面**（身份与环境由参数现渲染，不占对话的位置）：
        // 此后模型只看到"身份 + 摘要 + 之后的内容"。
        self.dialogue.insert(0, summary_message(summary));
        // 移出 cut 条、又插入一条：marks 整体前移 cut、再后移一格。
        for m in self.marks.iter_mut() {
            *m = m.saturating_sub(cut) + 1;
        }
        // 记下压缩点：回档到它之前，被销毁的对话在内存里补不回来（编排改走重建）。
        self.compacted_upto = up_to;
    }

    /// 给接下来的行打上回合 id（节点执行也用整场工作的同一套计数：回档才对得上）。
    pub fn set_turn(&mut self, turn_id: u64) {
        self.cur_turn = turn_id;
    }

    /// 生成一条转录行，并记下它完成时的历史长度（回档按 marks 逐行精确回退）与它属于哪次回复。
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
            tool,
            degraded: false,
            system: kind == "system",
            task: false,
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
}

/// 流式外送规则：信封之前照常外送，一旦累积文本里出现 "{" 就不再外送后续片段
/// （模型可能在同一轮里先写正文再发 tool 信封——信封绝不能当正文流上屏）。
/// 返回（本片可外送的部分, 新的累积文本）。纯函数，便于单测。
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
/// 工作名的缺省与唯一化：`base` 去空白；为空用 `fallback`；重名追加 `-2` / `-3`（进程内唯一即可）。
/// **纯策略**：`exists` 由调用方给——会话存在性归存储，本函数不关心它从哪来。
/// 归属：命名是**会话自己的策略**（它拥有会话身份），不是前端的便利函数。
pub fn unique_work_name(base: &str, fallback: &str, exists: impl Fn(&str) -> bool) -> String {
    let seed = if base.trim().is_empty() {
        fallback
    } else {
        base.trim()
    };
    if !exists(seed) {
        return seed.to_string();
    }
    let mut n = 2;
    loop {
        let cand = format!("{}-{}", seed, n);
        if !exists(&cand) {
            return cand;
        }
        n += 1;
    }
}
