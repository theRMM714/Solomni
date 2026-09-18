//! 协作引擎：建组 → 讨论 → 整理 → 执行 → 验收（纯状态机，不做输入输出）。
//! 状态机只认信封动词；发言内容永远是数据，不是指令。
//! 所有发给模型的文案经 core/prompt.rs 渲染自提示词册；成员拥有自己的会话通道。
//! 工具循环（联动 envelope::Verb::Tool 与 ports::ToolRunner）：策略（放行表）在核心，机制在适配层。

use crate::core::envelope::{self, ToolInvoke, Verb};
use crate::core::events::ToolCallView;
use crate::core::ports::{BoxedChat, Chat, Chunk, Msg, ToolOutcome, ToolRunner};
use crate::core::prompt::Prompts;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

/// 讨论轮次上限（超限交用户裁决——上限必生效）。
pub const MAX_ROUNDS: usize = 6;
/// 返工次数上限（超限交用户裁决）。
pub const MAX_REWORK: usize = 2;
/// 单次问询内的工具调用上限（超限强制收尾——上限必生效）。
pub const MAX_TOOL_CALLS: usize = 8;

/// 一个模块的外部工具环境：模块目录（外部工具进程的 cwd）+ 它声明的工具表。
/// cwd 必须落在声明它的模块里（命令形如 python tools/x.py，是相对模块根写的）。
#[derive(Debug, Clone)]
pub struct ModuleTools {
    pub root: PathBuf,
    /// 工具名 → 启动命令（模块作者在 module.yaml 的 tools.<名字>.command 里声明）。
    pub commands: BTreeMap<String, String>,
    /// 工具名 → 参数契约（只含**声明了** params 的工具；没声明的工具不校验、不进提示词）。
    pub books: BTreeMap<String, crate::core::schema::ToolSchema>,
}

/// 放行表：模块 id → 该模块的（目录, 工具表）。
/// **包含没有声明任何工具的模块**（命令表为空）——这样报错能区分「没有这个模块」与「这个模块没有这个工具」。
/// 跨模块同名工具不再冲突：模块内名字唯一由 map 保证，跨模块由信封里的 module 消歧。
pub fn tool_table(modules: &[crate::core::module::Module]) -> BTreeMap<String, ModuleTools> {
    modules
        .iter()
        .map(|m| {
            (
                m.manifest.id.clone(),
                ModuleTools {
                    root: m.root.clone(),
                    commands: m
                        .manifest
                        .tools
                        .iter()
                        .map(|(name, decl)| (name.clone(), decl.command.clone()))
                        .collect(),
                    books: m
                        .manifest
                        .tools
                        .iter()
                        .filter_map(|(name, decl)| decl.schema().map(|s| (name.clone(), s)))
                        .collect(),
                },
            )
        })
        .collect()
}

/// 成员的工具执行环境：来自 module.yaml（按模块分组的放行表）+ 注入的执行端口 + 本成员的沙箱。
pub struct MemberTools {
    /// 模块 id → 该模块的（目录, 工具表）；内置 read/write 不走这里。
    pub modules: BTreeMap<String, ModuleTools>,
    /// 本次会话的观察账本（哪些文件完整读过 / 由核心写过）：改动前的证据（见 systool::Observations）。
    pub observations: crate::core::systool::Observations,
    /// 信封修复端口：手写信封不合法时先问它能不能按无歧义的写法修好（默认只转义裸控制字符）。
    pub repair: Arc<dyn crate::core::ports::EnvelopeRepair + Send + Sync>,
    /// 运行日志：模型输出被长度截断这类"看不见的事实"要落盘，供事后确定问题。
    pub log: Arc<dyn crate::core::ports::Log + Send + Sync>,
    pub runner: Arc<dyn ToolRunner + Send + Sync>,
    /// 本成员的沙箱：内置文件工具的寻址与越界依据（权限收口在 core）。
    pub sandbox: crate::core::workspace::Sandbox,
    /// 内置文件工具的读写端口。
    pub io: Arc<dyn crate::core::ports::SysIo + Send + Sync>,
    /// 模块 id → 它缺的运行包能力（本档位下该模块的工具不执行；空表 = 都能执行）。
    pub unavailable: BTreeMap<String, Vec<String>>,
    /// 本成员工具进程的围栏（可达范围 + 断网）：策略在 core 派生，机制在 ToolRunner 适配层安装。
    pub fence: crate::core::fence::FenceSpec,
}

/// 可用的外部工具清单：逐条列成「模块.工具」，末尾补上内置工具。
/// 三种失败（模块不存在 / 模块没这个工具 / 缺 module 且模块不唯一）都用它把可选范围说回去。
fn available_tools(ctx: &MemberTools) -> String {
    let mut list: Vec<String> = Vec::new();
    for (id, mt) in &ctx.modules {
        for name in mt.commands.keys() {
            list.push(format!("{}.{}", id, name));
        }
    }
    list.extend(crate::core::systool::names());
    list.join(&ctx.sandbox.texts.tool_list_separator)
}

fn deny(ctx: &MemberTools, why: String) -> ToolOutcome {
    let texts = &ctx.sandbox.texts;
    ToolOutcome {
        ok: false,
        output: texts.render(&texts.available_wrapper, &[("why", why), ("tools", available_tools(ctx))]),
    }
}

/// 外部工具派发：先定模块（信封里的 module；省略时只有唯一模块才兜底），再查该模块的工具表。
/// 返回（实际使用的模块 id, 执行结果）。不猜：多模块 agent 下省略 module 直接如实报错。
fn dispatch_external(ctx: &MemberTools, inv: &ToolInvoke) -> (String, ToolOutcome) {
    let module = match inv.module.as_deref() {
        Some(m) => m.to_string(),
        None if ctx.modules.len() == 1 => ctx.modules.keys().next().cloned().unwrap_or_default(),
        None => {
            let why = ctx
                .sandbox
                .texts
                .render(&ctx.sandbox.texts.no_module_field, &[("tool", inv.name.clone())]);
            return (String::new(), deny(ctx, why));
        }
    };
    let Some(mt) = ctx.modules.get(&module) else {
        let why = ctx.sandbox.texts.render(&ctx.sandbox.texts.unknown_module, &[("module", module.clone())]);
        return (module.clone(), deny(ctx, why));
    };
    // 运行包未就绪（本档位下该模块的工具不执行）：如实报缺哪个能力，让模型换工具或告诉用户。
    if let Some(caps) = ctx.unavailable.get(&module) {
        let why = ctx.sandbox.texts.render(
            &ctx.sandbox.texts.module_unavailable,
            &[
                ("module", module.clone()),
                ("capability", caps.join(&ctx.sandbox.texts.tool_list_separator)),
            ],
        );
        return (module.clone(), deny(ctx, why));
    }
    match mt.commands.get(&inv.name) {
        // 工具进程的工作目录 = 它所属模块的根目录；围栏按该模块的根收口。
        Some(command) => {
            // 模块声明了参数契约就按它校验（参数错了不必启动进程）：没声明就照旧把 args 原样交给工具。
            if let Some(book) = mt.books.get(&inv.name) {
                let full = format!("{}.{}", module, inv.name);
                match serde_json::from_str::<serde_json::Value>(&inv.args_json) {
                    Ok(args) => {
                        if let Err(fault) = book.check(&args) {
                            let why = crate::core::systool::arg_fault_text(&ctx.sandbox.texts, &full, book, &fault);
                            return (module.clone(), deny(ctx, why));
                        }
                    }
                    Err(e) => {
                        let why = ctx
                            .sandbox
                            .texts
                            .render(&ctx.sandbox.texts.bad_args_json, &[("error", e.to_string())]);
                        return (module.clone(), deny(ctx, why));
                    }
                }
            }
            (module, ctx.runner.run(&ctx.fence.at(&mt.root), command, &inv.args_json))
        }
        None => {
            let why = ctx.sandbox.texts.render(
                &ctx.sandbox.texts.module_lacks_tool,
                &[("module", module.clone()), ("tool", inv.name.clone())],
            );
            (module.clone(), deny(ctx, why))
        }
    }
}

pub struct Member {
    pub id: String,
    pub system: String,
    pub chat: BoxedChat,
    pub present: bool,
    pub agreed: bool,
    /// 工具环境；None = 本模块未声明工具（tool 信封按原文收录）。
    pub tools: Option<MemberTools>,
}

impl Member {
    pub fn new(id: &str, system: String, chat: BoxedChat) -> Member {
        Member { id: id.to_string(), system, chat, present: true, agreed: false, tools: None }
    }
}

pub enum TurnOut {
    /// 一轮正常走完，转达给用户过目。
    Round,
    /// 有模块请教用户：轮转中止，等用户回答。
    AskUser { member: String, question: String },
    /// 留在组的成员全部同意 → 讨论终止。
    Done,
}

/// 讨论转录的一行：文本 + 该行是否"信封缺失、按发言原文收录"。
/// 降级是结构化信号（呈现层与后续判断都读它），文本里那句说明只给人和模型看。
#[derive(Debug, Clone)]
pub struct DiscLine {
    pub text: String,
    pub degraded: bool,
}

pub struct Discussion {
    pub members: Vec<Member>,
    pub transcript: Vec<DiscLine>,
    pub round: usize,
    /// 用户对 ask 的回答在此队列：先入先转达。
    pub pending_user_answers: Vec<String>,
    pub closed: bool,
    /// yes,allow：授权小组自裁——ask 不中止轮转，留档待办。
    pub allow_autonomy: bool,
    /// 提示词册（讨论文案来源）。
    prompts: Prompts,
}

impl Discussion {
    pub fn new(members: Vec<Member>, allow_autonomy: bool, prompts: Prompts) -> Discussion {
        Discussion { members, transcript: Vec::new(), round: 0, pending_user_answers: Vec::new(), closed: false, allow_autonomy, prompts }
    }

    /// 首轮：聊天约定 + 用户需求（文案经提示词册渲染）。
    pub fn open(&mut self, task: &str) {
        let opener = self.prompts.render(
            &self.prompts.core.discuss.opener,
            &[("protocol", self.prompts.core.chat_protocol.clone()), ("task", task.to_string())],
        );
        for i in 0..self.members.len() {
            let (system, id) = {
                let m = &self.members[i];
                (m.system.clone(), m.id.clone())
            };
            let msgs = vec![Msg::system(system), Msg::user(opener.clone())];
            let done = self.members[i].chat.complete(&msgs, false, &mut |_| true);
            let reply = envelope::parse(&done.raw);
            self.absorb(&id, reply.verb, reply.text, reply.degraded, done.truncated());
        }
        self.round = 1;
    }

    /// 推进一轮：把当前转录并入上下文，依次转达给每个在组且未同意的成员。
    /// 讨论阶段不接工具循环：工具属执行机制，讨论只出主意（最小边界）。
    pub fn step(&mut self) -> TurnOut {
        if self.closed {
            return TurnOut::Done;
        }
        // 轮次边界：本轮的发言都在这条之后（回放时据此重算「本轮谁已同意」）。
        self.transcript.push(DiscLine { text: format!("[轮次 {}]", self.round + 1), degraded: false });
        // 用户回答优先转达。
        if let Some(ans) = self.pending_user_answers.first().cloned() {
            self.pending_user_answers.remove(0);
            self.transcript.push(DiscLine { text: format!("[用户] {}", ans), degraded: false });
        }
        // 同意是针对方案的：转录变化后以本轮最新表态为准。
        for m in self.members.iter_mut() {
            if m.present {
                m.agreed = false;
            }
        }
        let snapshot = self.transcript.clone();
        for i in 0..self.members.len() {
            let (system, id) = {
                let m = &self.members[i];
                (m.system.clone(), m.id.clone())
            };
            if !self.members[i].present {
                continue;
            }
            let step_prompt = self.prompts.render(
                &self.prompts.core.discuss.step,
                &[("transcript", snapshot.iter().map(|l| l.text.clone()).collect::<Vec<_>>().join("\n"))],
            );
            let msgs = vec![Msg::system(system), Msg::user(step_prompt)];
            let done = self.members[i].chat.complete(&msgs, false, &mut |_| true);
            let reply = envelope::parse(&done.raw);
            let verb = reply.verb;
            let text = reply.text;
            let degraded = reply.degraded;
            self.absorb(&id, verb, text.clone(), degraded, done.truncated());
            let m = &mut self.members[i];
            match verb {
                Verb::Leave => m.present = false,
                Verb::Agree => m.agreed = true,
                Verb::Ask => {
                    if self.allow_autonomy {
                        let note = self.prompts.core.discuss.autonomy_note.clone();
                        self.transcript.push(DiscLine { text: note, degraded: false });
                        continue;
                    }
                    return TurnOut::AskUser { member: id, question: text };
                }
                Verb::Say | Verb::Tool => {}
            }
        }
        self.round += 1;
        if self.members.iter().filter(|m| m.present).all(|m| m.agreed) {
            self.closed = true;
            return TurnOut::Done;
        }
        if self.round > MAX_ROUNDS {
            self.closed = true;
            return TurnOut::Done;
        }
        TurnOut::Round
    }

    fn absorb(&mut self, id: &str, verb: Verb, text: String, degraded: bool, truncated: bool) {
        let tag = match verb {
            Verb::Say => "say",
            Verb::Ask => "ask",
            Verb::Leave => "leave",
            Verb::Agree => "agree",
            Verb::Tool => "tool",
        };
        let mut line = format!("[{}:{}] {}", id, tag, text);
        if degraded {
            line.push_str(&self.prompts.render(&self.prompts.core.tool_texts.discuss_degraded, &[]));
        }
        // 被长度截断：如实写在行尾（与"降级"同一套做法）——模型与用户都看得到
        if truncated {
            line.push_str(&self.prompts.render(&self.prompts.core.tool_texts.truncated_suffix, &[]));
        }
        self.transcript.push(DiscLine { text: line, degraded });
    }

    /// 全员同意后：核心整理——总结讨论，为每个留下的成员写执行任务提示词。
    pub fn synthesize(&self, core_chat: &mut dyn Chat) -> String {
        let user = self.prompts.render(
            &self.prompts.core.synthesize.user,
            &[("transcript", self.transcript.iter().map(|l| l.text.clone()).collect::<Vec<_>>().join("\n"))],
        );
        let msgs = vec![Msg::system(self.prompts.core.synthesize.system.clone()), Msg::user(user)];
        core_chat.complete(&msgs, false, &mut |_| true).raw
    }
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
}

/// 执行与验收：成员按任务干活并回报；核心对照回报产出结构化清单。
pub struct Execution {
    pub reports: BTreeMap<String, String>,
    /// 工具调用（成员 id → 该成员本轮的调用视图；会话据此发 tool 转录行）。
    pub traces: BTreeMap<String, Vec<ToolCallView>>,
    /// 验收原始输出（解析失败时如实呈现）。
    pub checklist_raw: String,
    /// 结构化清单；空 = 解析失败（all_pass 保守判否）。
    pub items: Vec<CheckItem>,
    /// 已返工次数。
    pub rework: usize,
}

impl Execution {
    pub fn new() -> Execution {
        Execution { reports: BTreeMap::new(), traces: BTreeMap::new(), checklist_raw: String::new(), items: Vec::new(), rework: 0 }
    }

    /// 执行：各在组成员按任务回报（文案经提示词册渲染）；声明了工具的成员走工具循环。
    pub fn run(members: &mut [Member], tasks: &str, prompts: &Prompts) -> Execution {
        let mut exec = Execution::new();
        exec.collect_reports(members, prompts.render(&prompts.core.execute.user, &[("tasks", tasks.to_string())]));
        exec
    }

    /// 返工：把验收差距发回各在组成员，重取回报（次数由调用方受 MAX_REWORK 约束）。
    pub fn rerun(&mut self, members: &mut [Member], tasks: &str, review_text: &str, prompts: &Prompts) {
        self.rework += 1;
        for m in members.iter_mut() {
            if !m.present {
                continue;
            }
            let user = prompts.render(
                &prompts.core.rerun.user,
                &[
                    ("tasks", tasks.to_string()),
                    ("review", review_text.to_string()),
                    ("report", self.reports.get(&m.id).cloned().unwrap_or_default()),
                ],
            );
            let (text, views) = self.collect_one(m, user);
            self.traces.entry(m.id.clone()).or_default().extend(views);
            self.reports.insert(m.id.clone(), text);
        }
    }

    /// 逐成员收集回报（工具循环在 converse 内）。
    fn collect_reports(&mut self, members: &mut [Member], user_prompt: String) {
        for m in members.iter_mut() {
            if !m.present {
                continue;
            }
            let (text, views) = self.collect_one(m, user_prompt.clone());
            self.traces.entry(m.id.clone()).or_default().extend(views);
            self.reports.insert(m.id.clone(), text);
        }
    }

    /// 单成员一次问询：拆字段借用（chat 可变 / tools 只读互不冲突），工具调用入册。
    fn collect_one(&mut self, m: &mut Member, user_prompt: String) -> (String, Vec<ToolCallView>) {
        let Member { id, system, chat, tools, .. } = m;
        converse(system, chat.as_mut(), tools.as_mut(), id, Msg::user(user_prompt))
    }

    /// 验收：核心对照方案逐项核对，输出结构化 pass/fail 清单。
    pub fn review(&mut self, core_chat: &mut dyn Chat, plan: &str, prompts: &Prompts) {
        let reports = self
            .reports
            .iter()
            .map(|(id, r)| format!("[{}] {}\n", id, r))
            .collect::<Vec<_>>()
            .join("");
        let user = prompts.render(
            &prompts.core.review.user,
            &[("plan", plan.to_string()), ("reports", reports)],
        );
        let msgs = vec![Msg::system(prompts.core.review.system.clone()), Msg::user(user)];
        let raw = core_chat.complete(&msgs, false, &mut |_| true).raw;
        self.items = envelope::extract_json_array(&raw)
            .and_then(|arr| serde_json::from_str::<Vec<CheckItem>>(&arr).ok())
            .unwrap_or_default();
        self.checklist_raw = raw;
    }

    pub fn all_pass(&self) -> bool {
        // 清单为空（解析失败）= 保守判否；有清单则逐项全过才通过。
        !self.items.is_empty() && self.items.iter().all(|i| i.status.eq_ignore_ascii_case("pass"))
    }
}

/// 一次工具调用的产出：调用视图 + 它压进历史的消息。
pub struct ToolRun {
    pub view: ToolCallView,
    /// 该工具行压进历史的消息（[工具结果] …）。
    pub msgs: Vec<Msg>,
}

/// 工具结果回注给模型的那条消息（文案来自册子：tool_result_wrapper）。
fn tool_result_msg(texts: &crate::core::prompt::ToolTexts, view: &ToolCallView) -> Msg {
    Msg::user(texts.render(&texts.tool_result_wrapper, &[("label", view.label()), ("output", view.output.clone())]))
}

/// 工具调用超限时告知模型的那条消息（文案来自册子：tool_cap）。
fn tool_cap_msg(texts: &crate::core::prompt::ToolTexts) -> Msg {
    Msg::user(texts.render(&texts.tool_cap, &[("n", MAX_TOOL_CALLS.to_string())]))
}

/// 一轮模型调用的产出（一轮 = 一条文本转录行；有工具时紧跟一条工具行）。
/// 原始输出不进这里：工具轮由 ToolCallView.raw 承载、文本轮进上下文的就是解析后的文本。
pub struct Round {
    /// 解析后的可见文本（信封缺失时即原文）；工具轮为空串（它说的就是那封信封）。
    pub text: String,
    /// 该轮思维链（没给就是空串）。
    pub reasoning: String,
    /// 该轮压进历史的消息：工具轮 = [assistant(raw)]，末轮 = [assistant(text)]。
    pub text_msgs: Vec<Msg>,
    pub tool: Option<ToolRun>,
    /// 供应商给的结束原因（原样；没给 = 空串）：核心据此分辨"写完停"还是"被长度截断"。
    pub finish: String,
}

impl Round {
    /// 这一轮的输出是不是被供应商按长度截断了。
    pub fn truncated(&self) -> bool {
        crate::core::ports::truncated(&self.finish)
    }
}

/// 成员一次问询（含工具循环，非流式）：返回（最终答复, 本轮全部工具调用视图）。
/// 执行阶段不复用历史，所以不返回消息；单 agent 会话走 converse_with 逐轮取消息。
pub(crate) fn converse(
    system: &str,
    chat: &mut dyn Chat,
    tools: Option<&mut MemberTools>,
    speaker: &str,
    first: Msg,
) -> (String, Vec<ToolCallView>) {
    let mut noop = |_c: Chunk| true;
    let mut views: Vec<ToolCallView> = Vec::new();
    let rounds = converse_with(
        chat,
        tools,
        vec![Msg::system(system.to_string()), first],
        false,
        speaker,
        &mut noop,
        &mut |v: &ToolCallView| views.push(v.clone()),
    );
    // 末轮恒为文本轮（工具轮之后必然再问一次；超限后按原文作答也走文本轮）。
    let text = rounds.last().map(|r| r.text.clone()).unwrap_or_default();
    (text, views)
}

/// 从既有消息列表续跑，**逐轮**返回产出；顺序即 round0 文本 → round0 工具 → round1 文本 → …
/// stream/on 透传给通道（呈现层在 on 里外送 Delta）；on 返回 false = 用户要求中止。
/// on_tool 在每个工具跑完后立刻回调（工具行与文本行因此天然有序）。
/// 终止保证：超限后告知一次并强制收尾；其后再来 tool 信封按原文作答，不再执行。
pub(crate) fn converse_with(
    chat: &mut dyn Chat,
    mut tools: Option<&mut MemberTools>,
    mut msgs: Vec<Msg>,
    stream: bool,
    speaker: &str,
    on: &mut dyn FnMut(Chunk) -> bool,
    on_tool: &mut dyn FnMut(&ToolCallView),
) -> Vec<Round> {
    // 观察账本随会话保存（回档时清空），这里不动它——它的语义是"这一段转录里的读取证据"。
    let mut rounds: Vec<Round> = Vec::new();
    let mut forced_final = false;
    loop {
        // 逐轮累积思维链（原文以通道返回值为准：非流式通道不回 Chunk）。
        let mut reasoning = String::new();
        let mut aborted = false;
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
            chat.complete(&msgs, stream, &mut sink)
        };
        // 结束原因如实带回：被长度截断要落日志——事后才判定得出"是截断还是模型自己写错"。
        let finish = done.finish.clone();
        let truncated = done.truncated();
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
            .tool
            .as_ref()
            .map(|t| crate::core::systool::is_freeform(&t.name))
            .unwrap_or(false);
        let mut repaired: Option<String> = None;
        if !forced_final && !aborted && !freeform_tool {
            if let Some(kind) = reply.tool.as_ref().and_then(|t| t.malformed.clone()) {
                if let Some(ctx) = tools.as_deref_mut() {
                    let out = ctx.repair.repair(&raw, &kind);
                    if let Some(text) = out.repaired.as_deref() {
                        let again = envelope::parse(text);
                        if again.tool.as_ref().map(|t| t.malformed.is_none()).unwrap_or(false) {
                            reply = again;
                            repaired = Some(out.what.join("；"));
                        }
                    }
                }
            }
        }
        match reply.tool.clone() {
            // 信封非法：**不执行任何工具**，但记一条失败的工具行把"信封不合法"回注给模型（下一轮自己改）。
            // 同样计入上限，所以模型反复输出非法信封最终会被强制收尾，不会死循环。
            Some(inv) if inv.malformed.is_some() && tools.is_some() && !forced_final => {
                let ctx = tools.as_deref_mut().expect("上臂已判存在");
                // 回执按判定出的类别给修法（未闭合 / 裸控制字符 / 语法错 / 字段不合法）。
                let mut why = ctx
                    .sandbox
                    .texts
                    .malformed_report(inv.malformed.as_ref().expect("上臂已判存在"));
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
                };
                on_tool(&view);
                let texts = &ctx.sandbox.texts;
                let raw_msg = Msg::assistant(raw.clone());
                let result_msg = tool_result_msg(texts, &view);
                msgs.push(raw_msg.clone());
                msgs.push(result_msg.clone());
                rounds.push(Round {
                    text: reply.text.clone(),
                    reasoning,
                    text_msgs: vec![raw_msg],
                    tool: Some(ToolRun { view, msgs: vec![result_msg] }),
                    finish: finish.clone(),
                });
                if rounds.len() >= MAX_TOOL_CALLS {
                    forced_final = true;
                    msgs.push(tool_cap_msg(texts));
                }
            }
            Some(inv) if tools.is_some() && !forced_final => {
                let ctx = tools.as_deref_mut().expect("上臂已判存在");
                // 内置工具（read/write/edit/search）优先且不属于任何模块；外部工具按模块定 cwd。
                // 自由格式工具（patch）的输入是**信封之后的那段正文**（不必转义）；其余工具是 JSON 参数。
                // 它的显示正文只认信封**之前**那段：补丁内容不该被当成 AI 发言渲染出来。
                let freeform = crate::core::systool::is_freeform(&inv.name);
                let args = if freeform { inv.body.clone() } else { inv.args_json.clone() };
                if freeform {
                    reply.text = inv.lead.clone();
                }
                let (module, outcome) = if crate::core::systool::is_builtin(&inv.name) {
                    (
                        String::new(),
                        crate::core::systool::execute(
                            &ctx.sandbox,
                            ctx.io.as_ref(),
                            &mut ctx.observations,
                            &inv.name,
                            &args,
                        ),
                    )
                } else {
                    dispatch_external(ctx, &inv)
                };
                // 修过信封就如实标注在回执最前面（模型与用户都能看到核心没有瞎猜）
                let outcome = match repaired.as_deref() {
                    Some(what) if !what.is_empty() => ToolOutcome {
                        ok: outcome.ok,
                        output: format!(
                            "{}\n{}",
                            ctx.sandbox
                                .texts
                                .render(&ctx.sandbox.texts.envelope_repaired, &[("what", what.to_string())]),
                            outcome.output
                        ),
                    },
                    _ => outcome,
                };
                let view = ToolCallView {
                    speaker: speaker.to_string(),
                    module,
                    name: inv.name.clone(),
                    ok: outcome.ok,
                    args: inv.args_json.clone(),
                    output: outcome.output.clone(),
                    raw: raw.clone(),
                };
                on_tool(&view);
                let texts = &ctx.sandbox.texts;
                let raw_msg = Msg::assistant(raw.clone());
                let result_msg = tool_result_msg(texts, &view);
                msgs.push(raw_msg.clone());
                msgs.push(result_msg.clone());
                // text = 信封之外的那段正文（可能为空；信封 JSON 已被 parse 剥掉，永不进 text）。
                // 这一轮的历史只有 assistant(raw)（raw 含正文+信封）：若它先出文本行，
                // 那条行自己不推历史，统一由紧随的工具行推进（见 session 与 rebuild 的分组规则）。
                rounds.push(Round {
                    text: reply.text.clone(),
                    reasoning,
                    text_msgs: vec![raw_msg],
                    tool: Some(ToolRun { view, msgs: vec![result_msg] }),
                    finish: finish.clone(),
                });
                if rounds.len() >= MAX_TOOL_CALLS {
                    forced_final = true;
                    msgs.push(tool_cap_msg(texts));
                }
            }
            // 无工具环境 / 已超限：按原文口径如实收录（信封已被剥掉，显示文本里不会有 JSON），循环终止。
            _ => {
                // 只有会出文本行（有正文或思维链）时才往历史里放这条 assistant，
                // 否则实时历史会比重建历史多一条空消息。
                let text = reply.text;
                let has_line = !text.trim().is_empty() || !reasoning.trim().is_empty();
                let text_msgs = if has_line { vec![Msg::assistant(text.trim().to_string())] } else { Vec::new() };
                rounds.push(Round { text, reasoning, text_msgs, tool: None, finish });
                return rounds;
            }
        }
    }
}
