//! 目的：生成中作业的取消表——让「停止」在生成期间立刻生效。
//! 管：登记生成中的作业、按作业名取消。
//! 不管：命令队列与核心状态（正因如此它才能立刻生效）；谁算一个作业（调用方定）。
//! 联动：由核心持有并驱动（`src/capabilities/conductor/api/handle.rs`）；它与会话的裁决队各管一半——这里管取消生成，那里管等用户。

use crate::kernel::api::SessionId;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// 目的：生成中作业的取消表。
/// 约束：核心登记，呈现层只能说「停哪个会话」。
#[derive(Default)]
pub struct JobRegistry {
    running: Mutex<HashMap<SessionId, Arc<AtomicBool>>>,
}

impl JobRegistry {
    pub fn new() -> Arc<JobRegistry> {
        Arc::new(JobRegistry::default())
    }

    /// 目的：登记一个生成中作业并给出它的取消标志（核心内部用）。
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

    /// 目的：请求停止该会话正在跑的生成。
    /// 返回：是否确实有一个在跑。
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

    /// 目的：该会话是否正在生成。
    /// 约束：配置界面据此拒绝改到一半的语义。
    pub fn is_running(&self, sid: &str) -> bool {
        self.running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(sid)
    }
}
