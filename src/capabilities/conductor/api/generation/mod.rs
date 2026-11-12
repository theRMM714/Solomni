//! 目的：生成的长步骤：各自在自己的工作线程上跑，核心队列只占「取 / 交」两步。
//! 管：每条循环自己的线程、事件外送与收尾；脱离调用方的点火器也在这里。
//! 不管：命令分发、会话装配、呈现层。
//! 联动：`api/handle.rs`（线程边界）、`service/`（核心状态）、`docs/presentation/contracts.md`

use super::*;

mod collab;
mod compact;
mod member;
mod single;

impl ConductorHandle {
    /// 目的：起一轮**脱离调用方**的节点执行：核心注入任务 + 不等它跑完。 完成后由叫醒逻辑推进父会话——所以这里只是"点火"。
    pub(crate) fn spawn_detached_node(&self, sid: &str, objective: &str) {
        // 节点执行也用整场工作的同一套回合计数（回档同步靠两边同一套编号）。
        let _ = self.call({
            let sid = sid.to_string();
            move |core| {
                core.bump_turn_of_child(&sid);
                Ok(())
            }
        });
        let me = self.clone();
        let (sid, objective) = (sid.to_string(), objective.to_string());
        let _ = std::thread::Builder::new()
            .name("solomni-node".to_string())
            .spawn(move || {
                let _ = me.node_generation(&sid, &objective);
            });
    }

    /// 目的：起一轮**脱离调用方**的"继续"（恢复被暂停的单 agent 会话用）：不等它跑完。 与"派活"的区别：不注入新任务，接着上一次的断点走。
    pub(crate) fn spawn_detached_continue(&self, sid: &str) {
        let me = self.clone();
        let sid = sid.to_string();
        let _ = std::thread::Builder::new()
            .name("solomni-resume".to_string())
            .spawn(move || {
                let _ = me.single_generation(&sid, None, Output::Stream);
            });
    }

    /// 目的：起一次**脱离调用方**的代理回合（子会话停下后叫醒代理用）：不等它跑完。 代理那边是一轮普通成员会话生成；它自己决定继续观察、追问、返工还是收敛。
    pub(crate) fn spawn_detached_proxy(&self, sid: &str) {
        let me = self.clone();
        let sid = sid.to_string();
        let _ = std::thread::Builder::new()
            .name("solomni-proxy".to_string())
            .spawn(move || {
                let _ = me.single_generation(&sid, None, Output::Stream);
            });
    }
    /// 目的：起一次**脱离调用方**的裁决回答（代理把转达的话落进协作子会话用）：不等它跑完。 与"叫醒父会话"的区别：这条带一份明确的回答（卡号 + 选项 id + 附言），不是从断点继续。
    pub(crate) fn spawn_detached_collab_answer(
        &self,
        sid: &str,
        card: &str,
        option: &str,
        note: &str,
    ) {
        let me = self.clone();
        let (sid, card, option, note) = (
            sid.to_string(),
            card.to_string(),
            option.to_string(),
            note.to_string(),
        );
        let _ = std::thread::Builder::new()
            .name("solomni-proxy-collab".to_string())
            .spawn(move || {
                let _ = me.answer_card(&sid, &card, &option, &note);
            });
    }
    /// 目的：子会话建好就开工——核心自己建的协作子会话按它此刻那张卡作答（有 begin_allow 就授权自裁）。 为什么按选项 id 而不是另走一条"开始"路径：同一个门只能有一条回答口（见 session-model.md）。
    pub(crate) fn start_child_collab(&self, sid: &str) {
        let me = self.clone();
        let sid = sid.to_string();
        let _ = std::thread::Builder::new()
            .name("solomni-child-collab".to_string())
            .spawn(move || {
                let look = sid.clone();
                let Ok(Some(queue)) = me.call(move |core| core.open_queue(&look)) else {
                    return;
                };
                let allow = crate::capabilities::session::api::OPT_BEGIN_ALLOW;
                let option = if queue.card.options.iter().any(|o| o.id == allow) {
                    allow.to_string()
                } else {
                    queue
                        .card
                        .options
                        .first()
                        .map(|o| o.id.clone())
                        .unwrap_or_default()
                };
                if option.is_empty() {
                    return;
                }
                let _ = me.answer_card(&sid, &queue.card.id, &option, "");
            });
    }

    /// 目的：起一次**脱离调用方**的回答处置（回答已经落定）：不等它跑完，用户点完立刻拿到回执。
    pub(crate) fn spawn_detached_dispose(
        &self,
        sid: &str,
        ticket: crate::capabilities::session::api::GateTicket,
    ) {
        let me = self.clone();
        let sid = sid.to_string();
        let _ = std::thread::Builder::new()
            .name("solomni-answer".to_string())
            .spawn(move || {
                let _ = me.collab_generation(&sid, CollabWork::Dispose(ticket));
            });
    }

    /// 目的：起一次**脱离调用方**的协作推进（叫醒父会话用）：不等它跑完。
    pub(crate) fn spawn_detached_collab(&self, sid: &str) {
        let me = self.clone();
        let sid = sid.to_string();
        let _ = std::thread::Builder::new()
            .name("solomni-chain".to_string())
            .spawn(move || {
                let _ = me.collab_generation(&sid, CollabWork::Resume);
            });
    }
}
