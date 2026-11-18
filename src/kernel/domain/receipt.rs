//! 目的：进程执行回执的纯装配——本机守门进程与 guest 命令通道共用同一份 shell 回执形状。
//! 管：stdout 与 stderr 分区（`[stderr]` 头）、shell 状态行（退出码 / 超时 / 用户停止）。
//! 不管：超时 / 取消的判定（由执行层给）、文案本身（来自 ProcessTexts）、cwd 与耗时（由工具层追加）、
//!   输出截断（唯一预算在 `capabilities::tools::domain::budget`，送回模型前统一收口）。
//! 联动：kernel 的 ProcTools 与 vm 的 QemuHandle 调用；词表见 ProcessTexts。

use crate::kernel::ports::ProcessTexts;

/// 目的：一次 shell 执行的收尾状态——正常退出（含拿不到码）/ 到点被杀 / 用户停止。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellEnd {
    /// 目的：真实退出码；None = 被信号结束、拿不到码。
    Exit(Option<i32>),
    /// 目的：到点被连根杀树。
    TimedOut { secs: u64 },
    /// 目的：用户停止（host 取消旗）。
    Cancelled,
}

/// 目的：把一次 shell 执行的输出与收尾状态拼成回执正文（**不含** cwd 与耗时——那由工具层追加）。
/// 参数：`texts` = 收尾文案；`out` / `err` = 两路输出；`end` = 收尾状态。
/// 约束：这里**不截断**——超限只由工具预算一处处理（见 `capabilities::tools::domain::budget`），
///   否则预算会再切一刀，标记自相矛盾且状态行可能被劈开。空输出不再留开头的空行。
pub fn shell_receipt(texts: &dyn ProcessTexts, out: &str, err: &str, end: ShellEnd) -> String {
    let mut output = out.to_string();
    if !err.is_empty() {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&texts.stderr_header());
        output.push('\n');
        output.push_str(err);
    }
    let status = match end {
        ShellEnd::Exit(code) => texts.shell_exit(code),
        ShellEnd::TimedOut { secs } => texts.shell_timeout(secs),
        ShellEnd::Cancelled => texts.shell_cancelled(),
    };
    if !output.is_empty() {
        output.push('\n');
    }
    output.push_str(&status);
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::detail::process::CodeProcessTexts;

    fn texts() -> CodeProcessTexts {
        CodeProcessTexts
    }

    #[test]
    fn normal_receipt_puts_the_exit_line_after_the_stderr_section() {
        let out = shell_receipt(&texts(), "hello\n", "warn\n", ShellEnd::Exit(Some(3)));
        assert_eq!(out, "hello\n\n[stderr]\nwarn\n\n[shell] 退出码 3");
        assert!(!out.contains("[shell] cwd"), "cwd 由工具层追加，不在装配里");
    }

    #[test]
    fn timeout_and_cancel_have_no_exit_line() {
        let timed = shell_receipt(&texts(), "", "", ShellEnd::TimedOut { secs: 2 });
        assert_eq!(
            timed,
            "[shell] 超时（2 秒）：工具进程与它拉起的整棵进程树已被终止"
        );
        let stopped = shell_receipt(&texts(), "", "", ShellEnd::Cancelled);
        assert_eq!(
            stopped,
            "[shell] 用户停止：工具进程与它拉起的整棵进程树已被终止"
        );
        let signal = shell_receipt(&texts(), "", "", ShellEnd::Exit(None));
        assert_eq!(signal, "[shell] 退出码未知（被信号结束）");
    }

    /// 目的：装配层原样交出正文——截断只有工具预算一处，否则标记会叠加、状态行可能被劈开。
    #[test]
    fn long_output_is_handed_over_whole_with_the_status_line_last() {
        let long = "x".repeat(40_000);
        let out = shell_receipt(&texts(), &long, "", ShellEnd::Exit(Some(0)));
        assert_eq!(out.chars().count(), 40_000 + 1 + "[shell] 退出码 0".chars().count());
        assert!(out.ends_with("[shell] 退出码 0"), "状态行必须最后");
        assert!(!out.contains("[截断]"), "装配层不做预算");
    }
}
