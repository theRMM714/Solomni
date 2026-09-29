//! 生成中作业的取消表：**机制**，不认识任何业务概念。
//! 「停止」不排队、不碰核心状态，所以生成期间也能立刻生效——这是它存在的全部理由。

use crate::kernel::api::SessionId;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// 生成中作业的取消表。核心登记，呈现层只能说「停哪个会话」。
#[derive(Default)]
pub struct JobRegistry {
    running: Mutex<HashMap<SessionId, Arc<AtomicBool>>>,
}

impl JobRegistry {
    pub fn new() -> Arc<JobRegistry> {
        Arc::new(JobRegistry::default())
    }

    /// 登记一个生成中作业并给出它的取消标志（核心内部用）。
    pub(crate) fn register(&self, sid: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(sid.to_string(), Arc::clone(&flag));
        flag
    }

    pub(crate) fn unregister(&self, sid: &str) {
        self.running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(sid);
    }

    /// 请求停止该会话正在跑的生成；返回是否确实有一个在跑。
    pub fn stop(&self, sid: &str) -> bool {
        match self
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(sid)
            .cloned()
        {
            Some(flag) => {
                flag.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    /// 该会话是否正在生成（配置界面据此拒绝改到一半的语义）。
    pub fn is_running(&self, sid: &str) -> bool {
        self.running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(sid)
    }
}
