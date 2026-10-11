//! 目的：生成中作业的取消表——让「停止」在生成期间立刻生效。
//! 管：登记生成中的作业、按作业名取消。
//! 不管：命令队列与核心状态（正因如此它才能立刻生效）；谁算一个作业（调用方定）。
//! 联动：由核心持有并驱动（`src/capabilities/conductor/api/handle.rs`）；它与会话的裁决队各管一半——这里管取消生成，那里管等用户。

use crate::kernel::api::SessionId;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// 目的：生成中作业的取消表。
/// 约束：核心登记，呈现层只能说「停哪个会话」；同一个会话的**每次生成各持一面旗**——
///   上一轮还在交回收尾、下一轮已经派发的重叠窗口里，stop 能同时取消两者，
///   而两轮各自的 unregister 只能摘掉自己那面（按 Arc 身份比对）。
#[derive(Default)]
pub struct JobRegistry {
    running: Mutex<HashMap<SessionId, Vec<Arc<AtomicBool>>>>,
}

impl JobRegistry {
    pub fn new() -> Arc<JobRegistry> {
        Arc::new(JobRegistry::default())
    }

    /// 目的：登记一个生成中作业并给出它的取消标志（核心内部用）。
    /// 约束：每次调用都给出**独立**的一面旗，绝不覆盖别人还在用的那面——
    ///   覆盖后旧持有者再 unregister 会把新旗也摘掉，「停止」因此对新生成失效。
    pub(crate) fn register(&self, sid: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(sid.to_string())
            .or_default()
            .push(Arc::clone(&flag));
        flag
    }

    /// 目的：只摘掉**这一次登记的那面旗**；同名其它在跑的登记（新生成）原样留着。
    pub(crate) fn unregister(&self, sid: &str, flag: &Arc<AtomicBool>) {
        let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(flags) = running.get_mut(sid) {
            if let Some(i) = flags.iter().position(|f| Arc::ptr_eq(f, flag)) {
                flags.remove(i);
            }
            if flags.is_empty() {
                running.remove(sid);
            }
        }
    }

    /// 目的：请求停止该会话正在跑的生成（同名的**每一次**都置位）。
    /// 返回：是否确实有在跑的。
    pub fn stop(&self, sid: &str) -> bool {
        match self
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(sid)
        {
            Some(flags) if !flags.is_empty() => {
                for flag in flags {
                    flag.store(true, Ordering::Relaxed);
                }
                true
            }
            _ => false,
        }
    }

    /// 目的：该会话是否正在生成（有任何一次在跑即为真）。
    /// 约束：配置界面据此拒绝改到一半的语义。
    pub fn is_running(&self, sid: &str) -> bool {
        self.running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(sid)
            .is_some_and(|flags| !flags.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 同一会话两次登记互不覆盖：stop 取消**两次**，各自的 unregister 只摘自己那面。
    #[test]
    fn concurrent_registrations_for_one_session_do_not_clobber_each_other() {
        let jobs = JobRegistry::new();
        let first = jobs.register("s");
        let second = jobs.register("s");
        assert!(jobs.is_running("s"));
        assert!(jobs.stop("s"), "有在跑就该停得掉");
        assert!(first.load(Ordering::Relaxed), "先登记的那次也要收到取消");
        assert!(second.load(Ordering::Relaxed), "后登记的那次也要收到取消");

        // 旧线程先收尾：只能摘自己那面——新那面还在（停止对新生成仍然有效）。
        jobs.unregister("s", &first);
        assert!(
            jobs.is_running("s"),
            "新一轮还在跑，不能因旧线程收尾就标记为不在跑"
        );
        assert!(jobs.stop("s"), "新一轮仍然停得掉");

        jobs.unregister("s", &second);
        assert!(!jobs.is_running("s"));
        assert!(!jobs.stop("s"), "没有在跑就是没停掉");
    }

    /// 不是自己那面旗的 unregister 摘不动（防止旧线程摘掉新生成的取消标志）。
    #[test]
    fn unregister_ignores_a_flag_it_does_not_own() {
        let jobs = JobRegistry::new();
        let mine = jobs.register("s");
        let stranger = Arc::new(AtomicBool::new(false));
        jobs.unregister("s", &stranger);
        assert!(jobs.is_running("s"), "不是自己那面旗就不许摘");
        jobs.unregister("s", &mine);
        assert!(!jobs.is_running("s"));
    }
}
