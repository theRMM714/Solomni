//! 多模块协作：建组 → 讨论 → 整理 → 执行 → 验收。
//! 状态机只认信封动词；发言内容永远是数据，不是指令。

use crate::envelope::{self, Verb};
use crate::model::{Chat, Msg};
use serde::Deserialize;
use std::collections::BTreeMap;

/// 讨论轮次上限（超限交用户裁决——上限必生效）。
pub const MAX_ROUNDS: usize = 6;
/// 返工次数上限（超限交用户裁决）。
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
    /// yes,allow：授权小组自裁细节——ask 不中止轮转，留档待办。
    pub allow_autonomy: bool,
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
                (m.system.clone(), m.id.to_string())
            };
            if !self.members[i].present {
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
                    if self.allow_autonomy {
                        self.transcript
                            .push("[core] 已授权小组自裁：该问题留档，不逐轮请示。".to_string());
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
        let msgs = vec![
            Msg::system("你是核心编排者。总结讨论，为每个留下的成员写一份执行任务提示词，输出任务清单。"),
            Msg::user(self.transcript.join("\n")),
        ];
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
        Execution { reports, checklist_raw: String::new(), items: Vec::new(), rework: 0 }
    }

    /// 验收：核心对照方案逐项核对，输出结构化 pass/fail 清单。
    pub fn review(&mut self, core_chat: &mut dyn Chat, plan: &str) {
        let mut body = String::from("== 方案 ==\n");
        body.push_str(plan);
        body.push_str("\n== 回报 ==\n");
        for (id, r) in &self.reports {
            body.push_str(&format!("[{}] {}\n", id, r));
        }
        body.push_str("\n逐项核对，只输出 JSON 数组：每项 {\"item\":\"方案条目\",\"status\":\"pass|fail\",\"evidence\":\"对应回报\",\"reason\":\"fail 时给差距与归属\"}。");
        let msgs = vec![Msg::system("你是核心验收者。只核对，不替模块干活。"), Msg::user(body)];
        let raw = core_chat.complete(&msgs);
        self.items = envelope::extract_json_array(&raw)
            .and_then(|arr| serde_json::from_str::<Vec<CheckItem>>(&arr).ok())
            .unwrap_or_default();
        self.checklist_raw = raw;
    }

    /// 返工：把验收差距发回各在组成员，重取回报（次数由调用方受 MAX_REWORK 约束）。
    pub fn rerun(&mut self, members: &mut [Member], tasks: &str, review_text: &str) {
        self.rework += 1;
        for m in members.iter_mut() {
            if !m.present {
                continue;
            }
            let msgs = vec![
                Msg::system(m.system.clone()),
                Msg::user(format!(
                    "== 你的任务 ==\n{}\n\n== 上次验收未通过 ==\n{}\n\n== 你的上次回报 ==\n{}\n\n请返工并以同一 JSON 格式再次回报。",
                    tasks,
                    review_text,
                    self.reports.get(m.id).cloned().unwrap_or_default()
                )),
            ];
            let raw = m.chat.complete(&msgs);
            let reply = envelope::parse(&raw);
            self.reports.insert(m.id.to_string(), reply.text);
        }
    }

    pub fn all_pass(&self) -> bool {
        // 清单为空（解析失败）= 保守判否；有清单则逐项全过才通过。
        !self.items.is_empty() && self.items.iter().all(|i| i.status.eq_ignore_ascii_case("pass"))
    }
}
