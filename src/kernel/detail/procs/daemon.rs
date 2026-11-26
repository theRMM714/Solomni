//! 目的：长驻子进程的句柄——活没活、说了什么、怎么收场。
//! 管：`try_wait` / `wait_exit` / `stderr_tail` / `take_stdin` / `take_stdout` / `stop`，
//!   以及 Drop 兜底收掉（默认先礼后兵，不留孤儿）。
//! 不管：就绪判据、重连与重启策略（在单元）；进程怎么起（同目录 `spawn.rs`）。
//! 联动：`spawn.rs` 产出本类型；单元在自己的收尾路径上调 `stop`，丢掉句柄等价于收掉。

use super::{ProcessBook, terminate};
use std::collections::VecDeque;
use std::process::{Child, ChildStdin, ChildStdout, ExitStatus};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 目的：stderr 的最近几行——抽干线程往里写，调用方按需取尾部。
/// 约束：只留尾部 `cap` 行（容量有界，长跑的进程不会把内存吃掉）。
#[derive(Clone)]
pub(crate) struct Tail {
    lines: Arc<Mutex<VecDeque<String>>>,
    cap: usize,
}

impl Tail {
    /// 目的：建一条有界尾行；`cap` 为 0 表示只抽干、不留。
    pub(crate) fn new(cap: usize) -> Tail {
        Tail {
            lines: Arc::new(Mutex::new(VecDeque::new())),
            cap,
        }
    }

    /// 目的：收一行（超出容量时丢最旧的那行）。
    pub(crate) fn push(&self, line: String) {
        if self.cap == 0 {
            return;
        }
        if let Ok(mut q) = self.lines.lock() {
            q.push_back(line);
            while q.len() > self.cap {
                q.pop_front();
            }
        }
    }

    /// 目的：取最近 `lines` 行（不足就全给，顺序照发生顺序）。
    pub(crate) fn take(&self, lines: usize) -> Vec<String> {
        let Ok(q) = self.lines.lock() else {
            return Vec::new();
        };
        let skip = q.len().saturating_sub(lines);
        q.iter().skip(skip).cloned().collect()
    }
}

/// 目的：一个我们亲手起的长驻子进程——已在账上、父死子亡挂好、stderr 有人在抽。
/// 约束：**丢掉即收掉**（Drop 调 `stop`）：先礼后兵在 `terminate` 里，Unix 先 TERM 整组再 KILL，
///   Windows 连整棵树；不想收场就显式 `stop` 一次（已收过的不再动手）。
pub struct Daemon {
    child: Child,
    pid: u32,
    tail: Option<Tail>,
    book: Option<Arc<ProcessBook>>,
    /// 目的：收过场了（退出 / 被杀 / 被收尸）——`stop` 与 Drop 据此不再重复动手。
    finished: bool,
    /// 目的：退出码（收过场且拿到了才有）。
    status: Option<ExitStatus>,
}

impl Daemon {
    /// 目的：把刚起好的子进程包起来（只有 `spawn.rs` 该调）。
    pub(crate) fn new(child: Child, tail: Option<Tail>, book: Option<Arc<ProcessBook>>) -> Daemon {
        let pid = child.id();
        Daemon {
            child,
            pid,
            tail,
            book,
            finished: false,
            status: None,
        }
    }

    /// 目的：问一次退没退——`None` = 还在跑；已退出时顺手销账（幂等）。
    pub fn try_wait(&mut self) -> Option<ExitStatus> {
        if self.finished {
            return self.status;
        }
        match self.child.try_wait() {
            Ok(Some(status)) => {
                self.settle(Some(status));
                Some(status)
            }
            Ok(None) => None,
            // 问不到就当还在跑：判活失败不该被读成"它死了"而提前收场。
            Err(_) => None,
        }
    }

    /// 目的：有界地等退出（用于"已经请它自己优雅停机"的场合）。
    /// 返回：等到了给退出码；预算内没等到给 `None`（调用方接着 `stop`）。
    pub fn wait_exit(&mut self, budget: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + budget;
        loop {
            if let Some(status) = self.try_wait() {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// 目的：最近几行 stderr（启动失败时拼进错误，别丢掉进程自己说的话）。
    pub fn stderr_tail(&self, lines: usize) -> Vec<String> {
        match &self.tail {
            Some(tail) => tail.take(lines),
            None => Vec::new(),
        }
    }

    /// 目的：取走 stdin（按行协议的会话要用；取走后 `send` 由调用方自己持）。
    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin.take()
    }

    /// 目的：取走 stdout（就绪的第一行、按行协议的读端都在调用方）。
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }

    /// 目的：收掉这个进程（幂等）——先礼后兵、连整棵树，再收尸销账。
    /// 约束：已退出时不重复动手；收不掉（信号发不出去）也照样收尸销账，不留死条目。
    pub fn stop(&mut self) {
        if self.finished {
            return;
        }
        let _ = terminate(self.pid);
        let status = self.child.wait().ok();
        self.settle(status);
    }

    /// 目的：把"收过场了"落定：记退出码、销账（只做一次）。
    fn settle(&mut self, status: Option<ExitStatus>) {
        self.finished = true;
        self.status = status;
        if let Some(book) = &self.book {
            book.forget(self.pid);
        }
    }
}

impl Drop for Daemon {
    /// 目的：句柄被丢掉就收掉——调用方忘了收也不留孤儿（这是本机制唯一能兜住的地方）。
    fn drop(&mut self) {
        self.stop();
    }
}
