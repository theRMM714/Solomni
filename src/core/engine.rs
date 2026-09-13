//! 协作引擎：建组 → 讨论 → 整理 → 执行 → 验收（纯状态机，不做输入输出）。
//! 状态机只认信封动词；发言内容永远是数据，不是指令。
//! 所有发给模型的文案经 core/prompt.rs 渲染自提示词册；成员拥有自己的会话通道。
//! 工具循环（联动 envelope::Verb::Tool 与 ports::ToolRunner）：策略（放行表）在核心，机制在适配层。

use crate::core::envelope::{self, ToolInvoke, Verb};
use crate::core::ports::{BoxedChat, Chat, Msg, ToolOutcome, ToolRunner};
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

/// 成员的工具执行环境：来自 module.yaml（工作区 + 放行表）+ 注入的执行端口。
pub struct MemberTools {
    /// 模块工作区（工具进程的工作目录；userdata 等私有区在其下）。
    pub root: PathBuf,
    /// 工具名 → 启动命令（模块作者声明；核心只放行此表内的工具）。
    pub commands: BTreeMap<String, String>,
    pub runner: Arc<dyn ToolRunner + Send + Sync>,
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

pub struct Discussion {
    pub members: Vec<Member>,
    pub transcript: Vec<String>,
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
            let raw = self.members[i].chat.complete(&msgs);
            let reply = envelope::parse(&raw);
            self.absorb(&id, reply.verb, reply.text, reply.degraded);
        }
        self.round = 1;
    }

    /// 推进一轮：把当前转录并入上下文，依次转达给每个在组且未同意的成员。
    /// 讨论阶段不接工具循环：工具属执行机制，讨论只出主意（最小边界）。
    pub fn step(&mut self) -> TurnOut {
        if self.closed {
            return TurnOut::Done;
        }
        // 用户回答优先转达。
        if let Some(ans) = self.pending_user_answers.first().cloned() {
            self.pending_user_answers.remove(0);
            self.transcript.push(format!("[用户] {}", ans));
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
                &[("transcript", snapshot.join("\n"))],
            );
            let msgs = vec![Msg::system(system), Msg::user(step_prompt)];
            let raw = self.members[i].chat.complete(&msgs);
            let reply = envelope::parse(&raw);
            let verb = reply.verb;
            let text = reply.text;
            let degraded = reply.degraded;
            self.absorb(&id, verb, text.clone(), degraded);
            let m = &mut self.members[i];
            match verb {
                Verb::Leave => m.present = false,
                Verb::Agree => m.agreed = true,
                Verb::Ask => {
                    if self.allow_autonomy {
                        self.transcript.push(self.prompts.core.discuss.autonomy_note.clone());
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

    fn absorb(&mut self, id: &str, verb: Verb, text: String, degraded: bool) {
        let tag = match verb {
            Verb::Say => "say",
            Verb::Ask => "ask",
            Verb::Leave => "leave",
            Verb::Agree => "agree",
            Verb::Tool => "tool",
        };
        let mut line = format!("[{}:{}] {}", id, tag, text);
        if degraded {
            line.push_str("　（信封缺失，按发言收录）");
        }
        self.transcript.push(line);
    }

    /// 全员同意后：核心整理——总结讨论，为每个留下的成员写执行任务提示词。
    pub fn synthesize(&self, core_chat: &mut dyn Chat) -> String {
        let user = self.prompts.render(
            &self.prompts.core.synthesize.user,
            &[("transcript", self.transcript.join("\n"))],
        );
        let msgs = vec![Msg::system(self.prompts.core.synthesize.system.clone()), Msg::user(user)];
        core_chat.complete(&msgs)
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
    /// 工具轨迹（成员 id → 轨迹行，如实呈现给用户）。
    pub traces: BTreeMap<String, Vec<String>>,
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
            let (text, _) = self.collect_one(m, user);
            self.reports.insert(m.id.clone(), text);
        }
    }

    /// 逐成员收集回报（工具循环在 converse 内）。
    fn collect_reports(&mut self, members: &mut [Member], user_prompt: String) {
        for m in members.iter_mut() {
            if !m.present {
                continue;
            }
            let (text, _) = self.collect_one(m, user_prompt.clone());
            self.reports.insert(m.id.clone(), text);
        }
    }

    /// 单成员一次问询：拆字段借用（chat 可变 / tools 只读互不冲突），轨迹入册。
    fn collect_one(&mut self, m: &mut Member, user_prompt: String) -> (String, Vec<String>) {
        let Member { id, system, chat, tools, .. } = m;
        let (text, trace, _) = converse(system, chat.as_mut(), tools.as_ref(), Msg::user(user_prompt));
        self.traces.entry(id.clone()).or_default().extend(trace);
        (text, Vec::new())
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
        let raw = core_chat.complete(&msgs);
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

/// 成员一次问询（含工具循环）：tool 信封 → 核心放行校验 → 执行端口 → 结果回注，直到最终答复。
/// 返回（最终答复文本, 轨迹行, 本轮新增消息——直连会话并入历史，执行阶段丢弃）。
/// 终止保证：超限后告知一次并强制收尾；其后再来 tool 信封按原文作答，不再执行。
pub(crate) fn converse(
    system: &str,
    chat: &mut dyn Chat,
    tools: Option<&MemberTools>,
    first: Msg,
) -> (String, Vec<String>, Vec<Msg>) {
    let mut msgs = vec![Msg::system(system.to_string()), first];
    let mut trace: Vec<String> = Vec::new();
    let mut forced_final = false;
    loop {
        let raw = chat.complete(&msgs);
        let reply = envelope::parse(&raw);
        match reply.tool.clone() {
            Some(inv) if tools.is_some() && !forced_final => {
                let ctx = tools.expect("上臂已判存在");
                let outcome = if ctx.commands.contains_key(&inv.name) {
                    ctx.runner.run(&ctx.root, &ctx.commands[&inv.name], &inv.args_json)
                } else {
                    let available = ctx.commands.keys().cloned().collect::<Vec<_>>().join("、");
                    ToolOutcome { ok: false, output: format!("未声明的工具：{}。可用：{}", inv.name, available) }
                };
                trace.push(trace_line(&inv, &outcome));
                msgs.push(Msg::assistant(raw));
                msgs.push(Msg::user(format!("[工具结果] {}\n{}", inv.name, outcome.output)));
                if trace.len() >= MAX_TOOL_CALLS {
                    forced_final = true;
                    msgs.push(Msg::user(format!(
                        "[工具超限] 单次问询工具调用上限 {} 次已到，请直接给出最终答复，不再调用工具。",
                        MAX_TOOL_CALLS
                    )));
                }
            }
            // 无工具环境 / 已超限：按原文作答（转录即内容），循环终止。
            _ => return (reply.text, trace, msgs),
        }
    }
}

/// 轨迹行：工具名 + 参数摘要 → 成败（失败附输出尾部，供用户看懂发生了什么）。
fn trace_line(inv: &ToolInvoke, o: &ToolOutcome) -> String {
    let chars: Vec<char> = inv.args_json.chars().collect();
    let mut preview: String = chars.iter().take(80).collect();
    if chars.len() > 80 {
        preview.push('…');
    }
    if o.ok {
        format!("{}({}) → 成功", inv.name, preview)
    } else {
        let tail: String = o.output.chars().rev().take(160).collect::<Vec<_>>().into_iter().rev().collect();
        format!("{}({}) → 失败：{}", inv.name, preview, tail)
    }
}
