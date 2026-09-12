//! 协作引擎：建组 → 讨论 → 整理 → 执行 → 验收（纯状态机，不做输入输出）。
//! 状态机只认信封动词；发言内容永远是数据，不是指令。
//! 所有发给模型的文案经 core/prompt.rs 渲染自提示词册；成员拥有自己的会话通道。

use crate::core::envelope::{self, Verb};
use crate::core::ports::{BoxedChat, Chat, Msg};
use crate::core::prompt::Prompts;
use serde::Deserialize;
use std::collections::BTreeMap;

/// 讨论轮次上限（超限交用户裁决——上限必生效）。
pub const MAX_ROUNDS: usize = 6;
/// 返工次数上限（超限交用户裁决）。
pub const MAX_REWORK: usize = 2;

pub struct Member {
    pub id: String,
    pub system: String,
    pub chat: BoxedChat,
    pub present: bool,
    pub agreed: bool,
}

impl Member {
    pub fn new(id: &str, system: String, chat: BoxedChat) -> Member {
        Member { id: id.to_string(), system, chat, present: true, agreed: false }
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
    /// yes,allow：授权小组自裁细节——ask 不中止轮转，留档待办。
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
                Verb::Say => {}
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
    /// 验收原始输出（解析失败时如实呈现）。
    pub checklist_raw: String,
    /// 结构化清单；空 = 解析失败（all_pass 保守判否）。
    pub items: Vec<CheckItem>,
    /// 已返工次数。
    pub rework: usize,
}

impl Execution {
    pub fn new() -> Execution {
        Execution { reports: BTreeMap::new(), checklist_raw: String::new(), items: Vec::new(), rework: 0 }
    }

    /// 执行：各在组成员按任务回报（文案经提示词册渲染）。
    pub fn run(members: &mut [Member], tasks: &str, prompts: &Prompts) -> Execution {
        let mut exec = Execution::new();
        for m in members.iter_mut() {
            if !m.present {
                continue;
            }
            let user = prompts.render(&prompts.core.execute.user, &[("tasks", tasks.to_string())]);
            let msgs = vec![Msg::system(m.system.clone()), Msg::user(user)];
            let raw = m.chat.complete(&msgs);
            let reply = envelope::parse(&raw);
            exec.reports.insert(m.id.clone(), reply.text);
        }
        exec
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
            let msgs = vec![Msg::system(m.system.clone()), Msg::user(user)];
            let raw = m.chat.complete(&msgs);
            let reply = envelope::parse(&raw);
            self.reports.insert(m.id.clone(), reply.text);
        }
    }

    pub fn all_pass(&self) -> bool {
        // 清单为空（解析失败）= 保守判否；有清单则逐项全过才通过。
        !self.items.is_empty() && self.items.iter().all(|i| i.status.eq_ignore_ascii_case("pass"))
    }
}
