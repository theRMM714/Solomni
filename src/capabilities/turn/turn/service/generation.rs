//! 目的：生成的长步骤：各自在自己的工作线程上跑，核心队列只占「取 / 交」两步。
//! 管：每条循环自己的线程与收尾；脱离调用方的点火器也在这里。
//! 不管：命令分发、会话装配、呈现层（宿主）。线程名与错误文案是对外可见的运行时事实，不进不出。
//! 联动：`turn::api::TurnHost`（宿主的队列与状态入口）、`service/{turn,flow}`（推进规则）、docs/presentation/contracts.md

mod collab;
mod compact;
mod member;
mod single;

pub(crate) use collab::collab_generation;
pub(crate) use compact::compact;
pub(crate) use member::run_member_turn;
pub(crate) use single::{node_generation, single_generation};

use crate::capabilities::conversation::session::api::GateTicket;
use crate::capabilities::turn::turn::api::{CollabWork, Output, TurnHost};

/// 目的：起一轮**脱离调用方**的节点执行：核心注入任务 + 不等它跑完。 完成后由叫醒逻辑推进父会话——所以这里只是"点火"。
pub(crate) fn spawn_detached_node<H: TurnHost>(host: &H, sid: &str, objective: &str) {
    // 节点执行也用整场工作的同一套回合计数（回档同步靠两边同一套编号）。
    let _ = host.core({
        let sid = sid.to_string();
        move |turn| {
            turn.bump_turn_of_child(&sid);
            Ok(())
        }
    });
    let me = host.clone();
    let (sid, objective) = (sid.to_string(), objective.to_string());
    let _ = std::thread::Builder::new()
        .name("solomni-node".to_string())
        .spawn(move || {
            let _ = node_generation(&me, &sid, &objective);
        });
}

/// 目的：起一轮**脱离调用方**的"继续"（恢复被暂停的单 agent 会话用）：不等它跑完。 与"派活"的区别：不注入新任务，接着上一次的断点走。
pub(crate) fn spawn_detached_continue<H: TurnHost>(host: &H, sid: &str) {
    let me = host.clone();
    let sid = sid.to_string();
    let _ = std::thread::Builder::new()
        .name("solomni-resume".to_string())
        .spawn(move || {
            let _ = single_generation(&me, &sid, None, Output::Stream);
        });
}

/// 目的：起一次**脱离调用方**的代理回合（子会话停下后叫醒代理用）：不等它跑完。 代理那边是一轮普通成员会话生成；它自己决定继续观察、追问、返工还是收敛。
pub(crate) fn spawn_detached_proxy<H: TurnHost>(host: &H, sid: &str) {
    let me = host.clone();
    let sid = sid.to_string();
    let _ = std::thread::Builder::new()
        .name("solomni-proxy".to_string())
        .spawn(move || {
            let _ = single_generation(&me, &sid, None, Output::Stream);
        });
}

/// 目的：起一次**脱离调用方**的裁决回答（代理把转达的话落进协作子会话用）：不等它跑完。 与"叫醒父会话"的区别：这条带一份明确的回答（卡号 + 选项 id + 附言），不是从断点继续。
pub(crate) fn spawn_detached_collab_answer<H: TurnHost>(
    host: &H,
    sid: &str,
    card: &str,
    option: &str,
    note: &str,
) {
    let me = host.clone();
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
pub(crate) fn start_child_collab<H: TurnHost>(host: &H, sid: &str) {
    let me = host.clone();
    let sid = sid.to_string();
    let _ = std::thread::Builder::new()
        .name("solomni-child-collab".to_string())
        .spawn(move || {
            let look = sid.clone();
            let Ok(Some(queue)) =
                me.core_env(move |turn, registry, probe| turn.open_queue(&look, registry, probe))
            else {
                return;
            };
            let allow = crate::capabilities::conversation::collab::api::OPT_BEGIN_ALLOW;
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
pub(crate) fn spawn_detached_dispose<H: TurnHost>(host: &H, sid: &str, ticket: GateTicket) {
    let me = host.clone();
    let sid = sid.to_string();
    let _ = std::thread::Builder::new()
        .name("solomni-answer".to_string())
        .spawn(move || {
            let _ = collab_generation(&me, &sid, CollabWork::Dispose(ticket));
        });
}

/// 目的：起一次**脱离调用方**的协作推进（叫醒父会话用）：不等它跑完。
pub(crate) fn spawn_detached_collab<H: TurnHost>(host: &H, sid: &str) {
    let me = host.clone();
    let sid = sid.to_string();
    let _ = std::thread::Builder::new()
        .name("solomni-chain".to_string())
        .spawn(move || {
            let _ = collab_generation(&me, &sid, CollabWork::Resume);
        });
}
