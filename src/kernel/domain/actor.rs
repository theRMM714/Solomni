//! 目的：命令队列——把一份核心状态搬到它自己的线程上，命令按到达顺序逐条执行。
//! 管：线程的起与退、命令的「取 / 交 / 回包」、一条命令 panic 不带走整个核心。
//! 不管：核心状态是什么（由调用方给）、停止与取消（`JobRegistry` 承担、不进队列）、命令的业务语义。
//! 联动：由 `src/kernel/api.rs` 重导出；核心手柄（`src/units/business/runtime/api/handle.rs`）持有一个 `Actor<Core>`。

use crate::kernel::ports::Log;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::mpsc::{self, Sender};

/// 目的：一条命令：在核心自己的线程上执行（因此核心状态不需要任何锁）。
pub type Job<S> = Box<dyn FnOnce(&mut S) + Send>;

/// 目的：命令队列与它独占的那条线程。
/// 约束：克隆廉价（内部只是通道）；状态只被工作线程碰，主线程一律经 `call` 提交命令。
pub struct Actor<S> {
    tx: Sender<Job<S>>,
}

impl<S> Clone for Actor<S> {
    fn clone(&self) -> Actor<S> {
        Actor {
            tx: self.tx.clone(),
        }
    }
}

impl<S: Send + 'static> Actor<S> {
    /// 目的：把状态搬到它的线程上：此后所有状态只被这一个线程碰。全部句柄丢弃后线程自然结束（通道断开即退出）。
    /// 参数：`thread` = 线程名；`at` = 日志埋点位置；`log` = 线程退出与 panic 的落点；`state` = 核心状态。
    /// 错误：起不出线程时如实拒绝（错误里带系统原因）。
    /// 约束：工作线程**绝不能**捕获句柄（那会持有一个发送端，通道永不闭合、线程永不退出）。
    pub fn spawn(
        thread: &str,
        at: &str,
        log: Arc<dyn Log + Send + Sync>,
        state: S,
    ) -> Result<Actor<S>, String> {
        let (tx, rx) = mpsc::channel::<Job<S>>();
        let at = at.to_string();
        std::thread::Builder::new()
            .name(thread.to_string())
            .spawn(move || {
                let mut state = state;
                while let Ok(job) = rx.recv() {
                    // 一条命令 panic 不该带走整个核心：接住并继续（回包通道随闭包销毁，调用方会看到「无回应」）。
                    if catch_unwind(AssertUnwindSafe(|| job(&mut state))).is_err() {
                        log.error(
                            &at,
                            "核心命令 panic：已接住，核心继续服务（请查上面的 panic 现场）",
                        );
                    }
                }
                log.info(&at, "核心线程退出：全部手柄已释放");
            })
            .map_err(|e| format!("启动核心线程失败：{}", e))?;
        Ok(Actor { tx })
    }

    /// 目的：发一条命令并等回复：调用方因此仍是「调用即拿结果」，不需要改成异步。
    /// 参数：`f` = 在核心线程上执行的那一步。
    /// 错误：通道已断（核心已停）或回包丢失（命令执行中 panic）。
    pub fn call<T, F>(&self, f: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(&mut S) -> Result<T, String> + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let job: Job<S> = Box::new(move |state: &mut S| {
            let _ = tx.send(f(state));
        });
        self.tx
            .send(job)
            .map_err(|_| "核心已停止：无法发送命令".to_string())?;
        rx.recv()
            .map_err(|_| "核心无回应：命令执行中发生 panic，或核心线程已停止".to_string())?
    }
}
