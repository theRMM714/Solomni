//! 目的：生成中作业的工具放行表——"要问用户"的调用在这里登记并等待。
//! 管：登记一格、等核心写回"是/否/全部放行"，停止时把等待解成取消。
//! 不管：谁该问、问什么（调用方判）；界面呈现（呈现层）；排队（它不进命令队列）。
//! 联动：与 `src/kernel/domain/jobs.rs` 同级；由核心持有并驱动（`src/capabilities/conductor/api/`）。

use crate::kernel::api::SessionId;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

/// 用户对一次工具确认的回答。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approval {
    /// 放行这一次。
    Allow,
    /// 拒绝这一次（工具不执行）。
    Deny,
    /// 放行这一次，且**本轮（这次生成）不再问**——只持续到 AI 停下输出，不改落盘设置。
    Full,
}

/// 当前在等的确认请求（快照用）：刷新后界面据此照样画得出那张"是 / 否 / 本轮不再问"的卡。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequest {
    pub module: Option<String>,
    pub tool: String,
    pub args: String,
}

/// 一次工具放行的等待格：工作线程等 `answer`，核心线程写它并唤醒。
#[derive(Default)]
pub struct ApprovalSlot {
    answer: Mutex<Option<Approval>>,
    cv: Condvar,
    cancelled: AtomicBool,
}

impl ApprovalSlot {
    /// 还在等吗（没回答、也没取消）？快照据此决定要不要把卡画出来。
    fn waiting(&self) -> bool {
        !self.cancelled.load(Ordering::Relaxed)
            && self
                .answer
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_none()
    }

    /// 工作线程：等用户的回答；被取消（停止）返回 `None`。
    pub fn wait(&self) -> Option<Approval> {
        let mut g = self.answer.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if self.cancelled.load(Ordering::Relaxed) {
                return None;
            }
            if let Some(v) = *g {
                return Some(v);
            }
            g = self.cv.wait(g).unwrap_or_else(|e| e.into_inner());
        }
    }
}

/// 工具放行表：按会话登记一格。核心登记，呈现层只能说"放行 / 拒绝哪个会话"。
#[derive(Default)]
pub struct ApprovalRegistry {
    slots: Mutex<HashMap<SessionId, (Arc<ApprovalSlot>, ApprovalRequest)>>,
}

impl ApprovalRegistry {
    pub fn new() -> Arc<ApprovalRegistry> {
        Arc::new(ApprovalRegistry::default())
    }

    /// 登记一格并给出它（核心内部用）。同一会话同时只有一次生成：重登记即覆盖上一格。
    /// `request` 是"在等什么"，供快照在刷新后重建卡片。
    pub(crate) fn register(&self, sid: &str, request: ApprovalRequest) -> Arc<ApprovalSlot> {
        let slot = Arc::new(ApprovalSlot::default());
        self.slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(sid.to_string(), (Arc::clone(&slot), request));
        slot
    }

    /// 这个会话此刻在等的确认请求（没有 / 已回答 / 已取消 = `None`）。
    pub fn pending(&self, sid: &str) -> Option<ApprovalRequest> {
        let slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        let (slot, req) = slots.get(sid)?;
        slot.waiting().then(|| req.clone())
    }

    pub(crate) fn unregister(&self, sid: &str) {
        self.slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(sid);
    }

    /// 用户回答（核心线程）：写入答案并唤醒工作线程；没有在等的格 = `false`。
    pub fn resolve(&self, sid: &str, answer: Approval) -> bool {
        let slot = self
            .slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(sid)
            .cloned();
        match slot {
            Some((s, _)) => {
                let mut g = s.answer.lock().unwrap_or_else(|e| e.into_inner());
                *g = Some(answer);
                s.cv.notify_all();
                true
            }
            None => false,
        }
    }

    /// 取消一个会话在等的确认（停止时用）：工作线程的 `wait` 返回 `None`。
    pub fn cancel(&self, sid: &str) {
        let slot = self
            .slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(sid)
            .cloned();
        if let Some((s, _)) = slot {
            s.cancelled.store(true, Ordering::Relaxed);
            s.cv.notify_all();
        }
    }
}

/// 一次生成要用的放行上下文：注册表 + 本会话 id（由核心线程装配进 `Live`）。
#[derive(Clone)]
pub struct ApprovalCtx {
    pub registry: Arc<ApprovalRegistry>,
    pub sid: SessionId,
}

impl ApprovalCtx {
    pub fn new(registry: Arc<ApprovalRegistry>, sid: &str) -> ApprovalCtx {
        ApprovalCtx {
            registry,
            sid: sid.to_string(),
        }
    }
}
