//! 目的：进程级 Ctrl+C：第一次只记下「用户按了」，等消费方来取；没人取还按第二次就放行默认终止。
//! 管：安装（进程级只装一次）、取走标志（`take`）、丢弃陈旧标志（`clear`）、按平台选机制。
//! 不管：谁消费标志（`cli/gate.rs` 的工作窗口每拍取一次）、输入框里 Ctrl+C 怎么翻（`cli/term.rs` 的按键翻译）。
//! 联动：`cli.rs` 起转录中心时装；`web.rs` 起转录中心服务器时也装（同一个标志、同一套语义）。

use std::sync::atomic::{AtomicBool, Ordering};

/// 目的：用户按过、还没人取走的 Ctrl+C。
static PRESSED: AtomicBool = AtomicBool::new(false);

/// 目的：取走标志：拿到 `true` 表示「用户要停下手上的活」。
/// 约束：取即清——一次按键只算一次停止；同一轮里没人再取走才算「第二下」。
pub(crate) fn take() -> bool {
    consume(&PRESSED)
}

/// 目的：丢掉陈旧标志（进输入框之前调：工作期留下的那一下不是冲着这条提示符来的）。
pub(crate) fn clear() {
    PRESSED.store(false, Ordering::SeqCst);
}

/// 目的：装上处理器：Ctrl+C / Ctrl+Break 的第一次只置标志（进程继续），第二次按默认处理。
/// 约束：进程级一次——重复调用没有副作用；装不上时如实返回原因（调用方自己决定说不说）。
pub(crate) fn install() -> Result<(), String> {
    imp::install()
}

/// 目的：一次按下落到标志上。
/// 返回：这次要不要按默认终止——`true` 表示标志本来就置着、没人取走，即用户已经按过一次了。
fn note_press(flag: &AtomicBool) -> bool {
    flag.swap(true, Ordering::SeqCst)
}

/// 目的：把标志取走（读到 `true` 表示按过），取即清。
fn consume(flag: &AtomicBool) -> bool {
    flag.swap(false, Ordering::SeqCst)
}

#[cfg(windows)]
mod imp {
    use super::{PRESSED, note_press};
    use std::sync::{Once, OnceLock};
    use windows_sys::Win32::System::Console::{
        CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler,
    };

    static ONCE: Once = Once::new();
    static ERR: OnceLock<String> = OnceLock::new();

    /// 目的：控制台事件的处理：只接 Ctrl+C / Ctrl+Break。
    /// 返回：1 = 已处理（进程继续，标志等消费方来取）；0 = 交给后续处理器与默认处理。
    /// 约束：标志没人取走时再按一次即返回 0——那是用户明确的「不玩了」，也给关窗 / 注销要跑的清理让路。
    unsafe extern "system" fn handler(ctrl: u32) -> i32 {
        if ctrl != CTRL_C_EVENT && ctrl != CTRL_BREAK_EVENT {
            return 0;
        }
        if note_press(&PRESSED) {
            return 0;
        }
        1
    }

    pub(super) fn install() -> Result<(), String> {
        ONCE.call_once(|| {
            // SAFETY: 只登记本项目自己的处理器（`handler` 是 `extern "system"`，只碰一个原子量）。
            if unsafe { SetConsoleCtrlHandler(Some(handler), 1) } == 0 {
                let _ = ERR.set("装 ConsoleCtrlHandler 失败".to_string());
            }
        });
        ERR.get().cloned().map_or(Ok(()), Err)
    }
}

#[cfg(unix)]
mod imp {
    use super::{PRESSED, note_press};
    use std::sync::{Once, OnceLock};

    static ONCE: Once = Once::new();
    static ERR: OnceLock<String> = OnceLock::new();

    /// 目的：SIGINT 的处理：第一次只置标志（等消费方来取），没人取还按第二次就真的退出。
    /// 约束：信号处理里只能用异步信号安全的操作——这里只有一个原子量的交换与 `_exit`。
    extern "C" fn handler(_sig: libc::c_int) {
        if note_press(&PRESSED) {
            // SAFETY: `_exit` 是异步信号安全的；退出码 130 = 128 + SIGINT，与默认终止的观测一致。
            unsafe { libc::_exit(130) };
        }
    }

    pub(super) fn install() -> Result<(), String> {
        ONCE.call_once(|| {
            // 函数项先转指针再转整数：直接转整数会被 clippy 判为 function_casts_as_integer。
            let handler_ptr = handler as *const () as libc::sighandler_t;
            // SAFETY: 只把本项目自己的 `extern "C"` 函数装到 SIGINT 上。
            let prev = unsafe { libc::signal(libc::SIGINT, handler_ptr) };
            if prev == libc::SIG_ERR {
                let _ = ERR.set("装 SIGINT 处理器失败".to_string());
            }
        });
        ERR.get().cloned().map_or(Ok(()), Err)
    }
}

#[cfg(not(any(windows, unix)))]
mod imp {
    pub(super) fn install() -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 取走即清：一次按键只算一次停止，取走之后不该再算「第二下」。
    #[test]
    fn a_press_is_consumed_exactly_once() {
        let flag = AtomicBool::new(false);
        assert!(!consume(&flag), "没按过就取不到");
        assert!(!note_press(&flag), "第一次只记下，不放行终止");
        assert!(consume(&flag), "按过就取到");
        assert!(!consume(&flag), "取走之后不再算第二次");
    }

    /// 没人取走时再按一次 = 用户要退出：这一下按默认处理（进程终止）。
    #[test]
    fn the_second_press_without_a_consumer_gives_up() {
        let flag = AtomicBool::new(false);
        assert!(!note_press(&flag), "第一次记下");
        assert!(note_press(&flag), "标志还在 = 没人取走 = 放行默认终止");
        consume(&flag);
        assert!(!note_press(&flag), "取走之后再按又从第一下算起");
    }

    /// 清掉陈旧标志之后，下一次按键仍然是「第一下」（工作期留下的那下不算）。
    #[test]
    fn clearing_resets_to_the_first_press() {
        let flag = AtomicBool::new(false);
        assert!(!note_press(&flag));
        flag.store(false, Ordering::SeqCst);
        assert!(!note_press(&flag), "清掉之后又回到第一下");
        assert!(consume(&flag));
    }
}
