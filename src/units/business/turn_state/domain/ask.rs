//! 目的：**裁决通道在回合侧的两个面**——工具层的阻塞面（`kernel::ports::AskUser`）与流程段的挂起面（`session_model` 的 `AskDesk`）。
//! 管：把一问推进同一条队、等回答、构不出可用选项或做不下去时的收场（发起方决定停不停会话）。
//! 不管：问什么、有几个选项（发起方给）、回答怎么落进业务（处置归发起方）、界面怎么画（呈现层）。
//! 联动：走的是**同一条通道**（同一条队、同一张卡、同一条回答命令），契约见 docs/session/session-model.md 的
//!   「请用户裁决」；队列本体在 `src/units/business/turn_state/domain/decisions.rs`。

use crate::kernel::api::types::Ask;
use crate::kernel::api::types::AskOutcome;
use crate::kernel::ports::AskUser;
use crate::units::business::session_model::api::AskDesk;
use crate::units::business::transcript::api::SessionEvent;
use crate::units::business::turn_state::domain::decisions::{Gates, SlotWake};
use std::sync::Arc;

/// 目的：工具执行层提问端口的实现：推一条卡给用户、**阻塞**等一个选项 id（不点不继续）。
/// 约束：构不出可用选项（空选项集）时只回 `NoOptions`——**停不停会话由发起方决定**；
///   停过一次之后不再推卡（会话已停下，再推一张没人能做主的卡只会让人困惑）。
pub struct SessionAsk {
    /// 这个会话的裁决通道（与流程段各关卡**共用同一条**）。
    door: Arc<dyn Gates>,
    /// 这个会话的 id（停会话与落警告都指它）。
    sid: String,
    /// 卡片外送与落盘（与这一回合其余事件同一条路）。
    emit: Box<dyn Fn(SessionEvent) + Send + Sync>,
    /// 停会话：宿主的停止入口（命令队列手柄的回调），本层不认识宿主的类型。
    stop: Box<dyn Fn(&str) + Send + Sync>,
    /// 这一趟已经因"做不下去"停过会话（`halt` 置位）。
    halted: std::sync::atomic::AtomicBool,
}

impl SessionAsk {
    /// 目的：装配一个会话的提问端口（宿主在生成线程上按该会话的裁决队与事件出口现造）。
    /// 参数：`emit` = 卡片的外送 + 落盘；`stop` = 停会话（归宿主的停止入口）。
    pub fn new(
        door: Arc<dyn Gates>,
        sid: &str,
        emit: Box<dyn Fn(SessionEvent) + Send + Sync>,
        stop: Box<dyn Fn(&str) + Send + Sync>,
    ) -> SessionAsk {
        SessionAsk {
            door,
            sid: sid.to_string(),
            emit,
            stop,
            halted: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// 目的：这一趟是不是已经停过会话（停了就不再发起新的裁决）。
    // 这套机制只在 Windows 的容器围栏里用（unix 没有 prepare_fence 这一步）：unix 侧无使用点，如实放行死代码。
    #[allow(dead_code)]
    fn stopped(&self) -> bool {
        self.halted.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl AskUser for SessionAsk {
    fn ask(&self, ask: &Ask) -> AskOutcome {
        if self.stopped() {
            // 这一趟已经因"做不下去"停过会话：不再推卡，如实按"没人答"收场。
            return AskOutcome::NoAnswer;
        }
        if ask.options.is_empty() {
            // 契约禁止置灰：没有一条真能执行的选项就不发起裁决——只回事实，停不停由发起方决定。
            (self.emit)(SessionEvent::Notice(format!(
                "[警告] 这一问没有一条能执行的选项：{}；这一环没有问出去。",
                ask.title
            )));
            return AskOutcome::NoOptions;
        }
        // 端口负责"先登记再推卡、外送卡片、不设超时地等"：回答、用户按停止（整队作废 = 拒绝）、
        // 会话被删 / 重建都会解开这一格。没有默认项：没人答就是失败。
        let wake = self.door.wait(ask, &mut |e| (self.emit)(e));
        match wake {
            SlotWake::Answer(id) => AskOutcome::Chosen(id),
            // 停止是用户的动作（= 拒绝）：**不套用任何收场声明**，否则"按停止"会被吞成放行。
            SlotWake::Stopped => AskOutcome::Stopped,
            SlotWake::Gone => {
                (self.emit)(SessionEvent::Notice(
                    "[裁决] 这一问没人能答（这一趟没有可回答的前端）：这一趟不执行。".to_string(),
                ));
                AskOutcome::NoAnswer
            }
        }
    }

    fn halt(&self, why: &str) {
        // 先落警告（它要留在会话里、用户看得见），再停会话（停止把队里其余挂起一律解成拒绝）。
        (self.emit)(SessionEvent::Notice(format!(
            "[警告] 工具层这一环做不下去：{}；这个会话已停下——先补上这一环再点「继续」。",
            why
        )));
        self.halted
            .store(true, std::sync::atomic::Ordering::Relaxed);
        (self.stop)(&self.sid);
    }
}

/// 目的：裁决通道**发起面**的实现：把同一份队列收窄成"挂起 + 有没有人在等"。
/// 约束：它只是视图——队列本体、卡号、回答的校验都在门里，这里不另存任何状态。
pub struct DeskFace {
    door: Arc<dyn Gates>,
}

impl AskDesk for DeskFace {
    fn later(&self, card: Option<&str>, ask: &Ask) -> (String, Vec<SessionEvent>) {
        self.door.later(card, ask)
    }

    fn pending(&self) -> bool {
        !self.door.is_empty()
    }

    fn head(&self) -> Option<(String, Ask)> {
        self.door.head()
    }

    fn drop_head(&self) {
        self.door.drop_head()
    }
}

/// 目的：把一条队列收窄成发起面（流程段挂卡用）。
pub fn desk_face(door: Arc<dyn Gates>) -> Arc<dyn AskDesk> {
    Arc::new(DeskFace { door })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::business::turn_state::domain::decisions::DecisionDoor;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Mutex, MutexGuard};

    /// 目的：这一趟的外送事件（断言"有没有推卡 / 有没有留警告"）。
    type Seen = Arc<Mutex<Vec<SessionEvent>>>;

    /// 目的：造一个接了空队与可观测停止口的工具层提问端口。
    fn port(seen: &Seen, stopped: &Arc<AtomicBool>) -> (SessionAsk, Arc<dyn Gates>) {
        let door: Arc<dyn Gates> = Arc::new(DecisionDoor::default());
        let ask = SessionAsk::new(
            Arc::clone(&door),
            "s1",
            Box::new({
                let seen = Arc::clone(seen);
                move |e| {
                    let mut g = seen.lock().unwrap_or_else(|x| x.into_inner());
                    g.push(e);
                }
            }),
            Box::new({
                let stopped = Arc::clone(stopped);
                move |_sid| stopped.store(true, Ordering::Relaxed)
            }),
        );
        (ask, door)
    }

    fn taken(seen: &Seen) -> MutexGuard<'_, Vec<SessionEvent>> {
        seen.lock().unwrap_or_else(|x| x.into_inner())
    }

    fn ask(options: Vec<(&str, &str)>) -> Ask {
        Ask {
            role: "tools".to_string(),
            name: "围栏".to_string(),
            title: "要不要破例".to_string(),
            body: String::new(),
            detail: String::new(),
            options: options
                .into_iter()
                .map(|(id, label)| (id.to_string(), label.to_string()))
                .collect(),
            kind: "fence".to_string(),
            payload: serde_json::Value::Null,
            notes: Vec::new(),
        }
    }

    /// 目的：没有一条能执行的选项就**不问**：只回 NoOptions，不推卡、不停会话（停不停由发起方定）。
    #[test]
    fn an_empty_option_set_reads_as_no_options_and_leaves_the_session_running() {
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let stopped = Arc::new(AtomicBool::new(false));
        let (port, door) = port(&seen, &stopped);
        assert_eq!(port.ask(&ask(Vec::new())), AskOutcome::NoOptions);
        assert!(!stopped.load(Ordering::Relaxed), "发起方没让停就不许停");
        assert!(door.is_empty(), "没有选项就一张卡都不许推");
        let g = taken(&seen);
        assert!(
            g.iter().any(
                |e| matches!(e, SessionEvent::Notice(t) if t.contains("没有一条能执行的选项"))
            ),
            "要留一条警告说明这一问没问出去"
        );
    }

    /// 目的：没人答（这一趟没有可回答的前端）如实按 NoAnswer 收场，并留一条说明。
    #[test]
    fn a_question_nobody_can_answer_reads_as_no_answer() {
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let stopped = Arc::new(AtomicBool::new(false));
        let (port, door) = port(&seen, &stopped);
        let port = Arc::new(port);
        let waiting = {
            let port = Arc::clone(&port);
            std::thread::spawn(move || port.ask(&ask(vec![("go", "跑一次")])))
        };
        let mut spins = 0;
        while door.is_empty() && spins < 500 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            spins += 1;
        }
        assert!(!door.is_empty(), "先登记再等");
        // 重建 = 这一趟没有回答者（不是用户停止）。
        door.rebuild(0);
        let outcome = waiting.join().expect("等待线程");
        assert_eq!(outcome, AskOutcome::NoAnswer);
        assert!(!stopped.load(Ordering::Relaxed), "没人答不等于停会话");
        let g = taken(&seen);
        assert!(
            g.iter()
                .any(|e| matches!(e, SessionEvent::Notice(t) if t.contains("没人能答"))),
            "要留一条说明，别让人以为跑过了"
        );
    }
}
