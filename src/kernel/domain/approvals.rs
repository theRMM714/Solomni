//! 生成中作业的**工具放行表**：机制，不认识任何业务概念。
//! 工具循环在工作线程上遇到"要问用户"的调用时登记一格并等待；核心线程收到用户的"是/否"后唤醒它。
//! 与 `JobRegistry` 同级：不进命令队列，所以生成期间照样立刻生效；停止会把它解成取消。

use crate::kernel::api::SessionId;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

/// 一次工具放行的等待格：工作线程等 `answer`，核心线程写它并唤醒。
#[derive(Default)]
pub struct ApprovalSlot {
    answer: Mutex<Option<bool>>,
    cv: Condvar,
    cancelled: AtomicBool,
}

impl ApprovalSlot {
    /// 工作线程：等用户的是/否；被取消（停止）返回 `None`。
    pub fn wait(&self) -> Option<bool> {
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
    slots: Mutex<HashMap<SessionId, Arc<ApprovalSlot>>>,
}

impl ApprovalRegistry {
    pub fn new() -> Arc<ApprovalRegistry> {
        Arc::new(ApprovalRegistry::default())
    }

    /// 登记一格并给出它（核心内部用）。同一会话同时只有一次生成：重登记即覆盖上一格。
    pub(crate) fn register(&self, sid: &str) -> Arc<ApprovalSlot> {
        let slot = Arc::new(ApprovalSlot::default());
        self.slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(sid.to_string(), Arc::clone(&slot));
        slot
    }

    pub(crate) fn unregister(&self, sid: &str) {
        self.slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(sid);
    }

    /// 用户回答（核心线程）：写入答案并唤醒工作线程；没有在等的格 = `false`。
    pub fn resolve(&self, sid: &str, ok: bool) -> bool {
        let slot = self
            .slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(sid)
            .cloned();
        match slot {
            Some(s) => {
                let mut g = s.answer.lock().unwrap_or_else(|e| e.into_inner());
                *g = Some(ok);
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
        if let Some(s) = slot {
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
