//! 目的：**工具层提问端口的会话侧实现**（`kernel::ports::AskUser`）——推一条问题并**阻塞**等一个选项 id。
//! 管：进队与等回答、构不出可用选项时的收场（停会话 + 落警告）、停止 / 关闭时按"拒绝"解开。
//! 不管：问什么、有几个选项（发起方给）、回答怎么落进业务（处置归发起方）、界面怎么画（呈现层）。
//! 联动：走的是**同一条通道**（同一条队、同一张卡、同一条回答命令），契约见 docs/session/session-model.md 的
//!   「请用户裁决」；队列本体在 `capabilities/session/domain/decisions.rs`，本实现由核心在生成线程上现造。

use crate::capabilities::conductor::api::{ConductorHandle, SessionOps};
use crate::capabilities::session::api::{
    AnswerSlot, DecisionDoor, Pending, SessionEvent, SlotWake,
};
use crate::kernel::api::{Ask, AskOutcome};
use crate::kernel::ports::AskUser;
use std::sync::{Arc, Mutex};

/// 目的：工具执行层提问端口的实现：推一条卡给用户、**阻塞**等一个选项 id（不点不继续）。
/// 约束：构不出可用选项（空选项集）时**不发起裁决**，改为停掉这个会话 + 落一条警告（契约禁止置灰）；
///   停过一次之后不再推卡——会话已停下，再推一张没人能做主的卡只会让人困惑。
pub struct SessionAsk {
    /// 这个会话的裁决队（与核心各关卡、工具级确认**共用同一条**）。
    door: Arc<DecisionDoor>,
    /// 这个会话的 id（停会话与落警告都指它）。
    sid: String,
    /// 卡片外送与落盘（与这一回合其余事件同一条路）。
    emit: Box<dyn Fn(SessionEvent) + Send + Sync>,
    /// 停会话走核心手柄；`Mutex` 是因为 `mpsc::Sender` 不是 `Sync`（与 `ProxyBridge` 同一理由）。
    handle: Mutex<ConductorHandle>,
    /// 这一趟已经因"做不下去"停过会话（`halt` 置位）。
    halted: std::sync::atomic::AtomicBool,
}

impl SessionAsk {
    /// 目的：装配一个会话的提问端口（核心在生成线程上按该会话的裁决队与事件出口现造）。
    /// 参数：`emit` = 卡片的外送 + 落盘；`handle` = 停会话用的核心手柄。
    pub fn new(
        door: Arc<DecisionDoor>,
        sid: &str,
        emit: Box<dyn Fn(SessionEvent) + Send + Sync>,
        handle: ConductorHandle,
    ) -> SessionAsk {
        SessionAsk {
            door,
            sid: sid.to_string(),
            emit,
            handle: Mutex::new(handle),
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
            // 契约禁止置灰：没有一条真能执行的选项就不发起裁决——停会话 + 落警告。
            self.halt(&format!("{}：{}（{}）", ask.title, ask.body, ask.detail));
            return AskOutcome::NoOptions;
        }
        // 声明的默认项必须是**本卡选项集里的一条**：不属于就如实记一句、按"没人答"处置，不静默改写。
        let default = match ask.on_unanswered.as_deref() {
            Some(id) if ask.has_option(id) => Some(id.to_string()),
            Some(id) => {
                (self.emit)(SessionEvent::Notice(format!(
                    "[警告] 这一问声明的默认项不在它的选项集里（{}）：这条声明不算数，没人答时按不执行收场。",
                    id
                )));
                None
            }
            None => None,
        };
        // **先登记再推卡**：否则用户手快会答在一张还没进队的卡上，答案丢掉、发起方干等。
        let slot = AnswerSlot::new();
        let (_, events) = self.door.push(
            None,
            Pending::ToolAsk(Box::new(ask.clone())),
            "",
            Some(Arc::clone(&slot)),
        );
        for e in events {
            (self.emit)(e);
        }
        // 不设超时：回答、用户按停止（整队作废 = 拒绝）、会话被删 / 重建都会解开这一格。
        match slot.wait() {
            SlotWake::Answer(id) => AskOutcome::Chosen(id),
            // 停止是用户的动作（= 拒绝）：**不套用默认项**，否则"按停止"会被吞成放行。
            SlotWake::Stopped => AskOutcome::Stopped,
            SlotWake::Gone => match default {
                Some(id) => {
                    (self.emit)(SessionEvent::Notice(format!(
                        "[裁决] 这一问没人能答：按声明的默认项（{}）收场——这一下不是用户答的。",
                        id
                    )));
                    AskOutcome::Defaulted(id)
                }
                None => {
                    (self.emit)(SessionEvent::Notice(
                        "[裁决] 这一问没人能答（这一趟没有可回答的前端），也没有声明默认项：这一趟不执行。"
                            .to_string(),
                    ));
                    AskOutcome::NoAnswer
                }
            },
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
        let handle = self.handle.lock().unwrap_or_else(|e| e.into_inner());
        let _ = handle.stop(&self.sid);
    }
}
