//! 多模块协作：建组 → 讨论 → 整理 → 执行 → 验收。
//! 状态机只认信封动词；发言内容永远是数据，不是指令。

use crate::envelope::{self, Verb};
use crate::model::{Chat, Msg};
use std::collections::BTreeMap;

/// 讨论轮次上限（超限交用户裁决——上限必生效）。
pub const MAX_ROUNDS: usize = 6;
// 待接入：验收 fail 后的定向返工循环（联动 Execution::review）。
#[allow(dead_code)]
pub const MAX_REWORK: usize = 2;

pub struct Member<'a> {
    pub id: &'a str,
    pub system: String,
    pub chat: &'a mut dyn Chat,
    pub present: bool,
    pub agreed: bool,
}

pub enum TurnOut {
    /// 一轮正常走完，转达给用户过目。
    Round,
    /// 有模块请教用户：核心中止轮转，等用户回答。
    AskUser { member: String, question: String },
    /// 留在组的成员全部同意 → 讨论终止。
    Done,
}

pub struct Discussion<'a> {
    pub members: Vec<Member<'a>>,
    pub transcript: Vec<String>,
    pub round: usize,
    /// 用户对 ask 的回答在此队列：先入先转达。
    pub pending_user_answers: Vec<String>,
    pub closed: bool,
}

impl<'a> Discussion<'a> {
    /// 首轮提示词：聊天约定 + 各模块职责 + 用户需求。
    /// 聊天约定是文本不是信封的一部分，可自由演化。
    pub fn open(&mut self, task: &str, chat_protocol: &str) {
        let mut opener = String::from(chat_protocol);
        opener.push_str("\n\n== 用户需求 ==\n");
        opener.push_str(task);
        for i in 0..self.members.len() {
            let (system, id) = {
                let m = &self.members[i];
                (m.system.clone(), m.id.to_string())
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
        // 先清空旧同意票再重新投票（同意是针对方案的，转录变化后以最新表态为准）。
        for m in self.members.iter_mut() {
            if m.present {
                m.agreed = false;
            }
        }
        let snapshot = self.transcript.clone();
        let mut anyone_left = false;
        for i in 0..self.members.len() {
            let (system, id, present, agreed_before) = {
                let m = &self.members[i];
                (m.system.clone(), m.id.to_string(), m.present, m.agreed)
            };
            if !present {
                continue;
            }
            let msgs = vec![
                Msg::system(system),
                Msg::user(format!("== 讨论至今 ==\n{}\n\n请继续。", snapshot.join("\n"))),
            ];
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
                    return TurnOut::AskUser { member: id, question: text };
                }
                Verb::Say => {}
            }
            if m.agreed || !m.present {
                continue;
            }
            let _ = agreed_before;
            anyone_left = true;
        }
        let _ = anyone_left;
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

    /// 全员同意后：核心整理（本骨架版把整理也交给一个会话——真实版由核心提示词完成）。
    pub fn synthesize(&self, core_chat: &mut dyn Chat) -> String {
        let msgs = vec![
            Msg::system("你是核心编排者。总结讨论，为每个留下的成员写一份执行任务提示词，输出任务清单。"),
            Msg::user(self.transcript.join("\n")),
        ];
        core_chat.complete(&msgs)
    }
}

/// 执行与验收：成员按任务干活并回报；核心对照回报产出 pass/fail 清单。
pub struct Execution {
    pub reports: BTreeMap<String, String>,
    pub checklist: String,
    // 待接入：返工计数（联动 MAX_REWORK 上限裁决）。
    #[allow(dead_code)]
    pub rework: usize,
}

impl Execution {
    pub fn run(members: &mut [Member], tasks: &str) -> Execution {
        let mut reports = BTreeMap::new();
        for m in members.iter_mut() {
            if !m.present {
                continue;
            }
            let msgs = vec![
                Msg::system(m.system.clone()),
                Msg::user(format!(
                    "== 你的任务 ==\n{}\n\n完成后必须以 JSON 回报：{{\"summary\":\"做了什么\",\"changes\":\"动了什么\",\"open\":\"遗留问题，没有则空\"}}",
                    tasks
                )),
            ];
            let raw = m.chat.complete(&msgs);
            let reply = envelope::parse(&raw);
            reports.insert(m.id.to_string(), reply.text);
        }
        Execution { reports, checklist: String::new(), rework: 0 }
    }

    /// 验收：核心对照方案逐项核对（本骨架版由核心会话完成，输出清单文本）。
    pub fn review(&mut self, core_chat: &mut dyn Chat, plan: &str) {
        let mut body = String::from("== 方案 ==\n");
        body.push_str(plan);
        body.push_str("\n== 回报 ==\n");
        for (id, r) in &self.reports {
            body.push_str(&format!("[{}] {}\n", id, r));
        }
        body.push_str("\n逐项核对，输出 pass/fail 清单：每项 {item, status, evidence/reason}。");
        let msgs = vec![Msg::system("你是核心验收者。只核对，不替模块干活。"), Msg::user(body)];
        self.checklist = core_chat.complete(&msgs);
    }

    pub fn all_pass(&self) -> bool {
        // 骨架版判据：验收文本包含 "fail" 即视为有未通过项（真实版用结构化清单）。
        !self.checklist.contains("fail")
    }
}
