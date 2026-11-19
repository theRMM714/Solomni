//! 目的：进程执行回执的纯装配——本机守门进程与 guest 命令通道共用同一份 shell 回执形状。
//! 管：stdout 与 stderr 分区（`[stderr]` 头）、收尾状态转成结构化说明（退出码 / 超时 / 用户停止）。
//! 不管：超时 / 取消的判定（由执行层给）；文案（在 `locales/<lang>.ftl`，由 `detail::i18n` 渲染）；
//!   cwd 与耗时（由工具层追加）；输出截断（唯一预算在 `capabilities::tools::domain::budget`）。
//! 联动：kernel 的 ProcTools 与 vm 的 QemuHandle 调用；渲染见 `detail::i18n::render_receipt`。

use crate::kernel::contracts::note::{Note, Severity, codes};

/// 目的：stderr 分区头——ASCII 常量（模型与人都读得懂，且不随界面语言变）。
pub const STDERR_HEADER: &str = "[stderr]";

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

/// 目的：一次 shell 执行的回执——原始输出（`payload`）与结构化说明（`notes`）分开。
/// 约束：`payload` 只含进程产出的字节；任何需要按语言渲染的句子都进 `notes`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellReceipt {
    pub payload: String,
    pub notes: Vec<Note>,
}

/// 目的：把 stdout 与 stderr 拼成回执正文（**不含**状态说明与 cwd——那两类都由说明或工具层给）。
/// 约束：这里**不截断**——超限只由工具预算一处处理（见 `capabilities::tools::domain::budget`），
///   否则预算会再切一刀，标记自相矛盾且分段可能被劈开。空输出不再留开头的空行。
pub fn shell_payload(out: &str, err: &str) -> String {
    let mut payload = out.to_string();
    if err.is_empty() {
        return payload;
    }
    if !payload.is_empty() {
        payload.push('\n');
    }
    payload.push_str(STDERR_HEADER);
    payload.push('\n');
    payload.push_str(err);
    payload
}

/// 目的：收尾状态 → 一条说明——退出 0 是 info，非零是 warning，超时 / 信号是 error，用户停止是 warning。
/// 约束：非零退出只说明"命令没正常完成"，**不**声称它被围栏拒绝（越界由围栏自己以退出码拒绝）。
pub fn shell_end_note(end: ShellEnd) -> Note {
    match end {
        ShellEnd::Exit(Some(0)) => Note::info(codes::SHELL_EXIT).arg("code", 0),
        ShellEnd::Exit(Some(code)) => {
            Note::new(codes::SHELL_EXIT, Severity::Warning).arg("code", code)
        }
        ShellEnd::Exit(None) => Note::error(codes::SHELL_SIGNAL),
        ShellEnd::TimedOut { secs } => Note::error(codes::SHELL_TIMEOUT).arg("secs", secs),
        ShellEnd::Cancelled => Note::new(codes::SHELL_CANCELLED, Severity::Warning),
    }
}

/// 目的：把一次 shell 执行的输出与收尾状态装配成回执（正文 + 一条收尾说明）。
pub fn shell_receipt(out: &str, err: &str, end: ShellEnd) -> ShellReceipt {
    ShellReceipt {
        payload: shell_payload(out, err),
        notes: vec![shell_end_note(end)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 目的：stderr 与 stdout 分区，且正文里没有任何状态句子（状态是说明）。
    #[test]
    fn payload_carries_only_process_output_and_the_stderr_section() {
        let got = shell_receipt("hello\n", "warn\n", ShellEnd::Exit(Some(3)));
        assert_eq!(got.payload, "hello\n\n[stderr]\nwarn\n");
        assert!(!got.payload.contains("[shell]"), "状态不进正文");
        assert_eq!(got.notes.len(), 1);
    }

    /// 目的：空输出不留开头的空行；只有 stderr 时不加前导换行。
    #[test]
    fn empty_streams_do_not_leave_blank_lines() {
        assert_eq!(shell_payload("", ""), "");
        assert_eq!(shell_payload("", "boom"), "[stderr]\nboom");
    }

    /// 目的：收尾说明的码与轻重——正常退出 info，非零 warning，超时 / 信号 error，用户停止 warning。
    #[test]
    fn end_notes_map_to_the_severity_table() {
        let ok = shell_end_note(ShellEnd::Exit(Some(0)));
        assert_eq!(
            (ok.code.as_str(), ok.severity),
            (codes::SHELL_EXIT, Severity::Info)
        );
        let bad = shell_end_note(ShellEnd::Exit(Some(3)));
        assert_eq!(bad.args.get("code").map(String::as_str), Some("3"));
        assert_eq!(bad.severity, Severity::Warning);
        assert_eq!(
            shell_end_note(ShellEnd::Exit(None)).severity,
            Severity::Error
        );
        let timed = shell_end_note(ShellEnd::TimedOut { secs: 2 });
        assert_eq!(timed.code, codes::SHELL_TIMEOUT);
        assert_eq!(timed.args.get("secs").map(String::as_str), Some("2"));
        assert_eq!(timed.severity, Severity::Error);
        let stopped = shell_end_note(ShellEnd::Cancelled);
        assert_eq!(stopped.code, codes::SHELL_CANCELLED);
        assert_eq!(stopped.severity, Severity::Warning, "用户停止不是故障");
    }

    /// 目的：装配层原样交出正文——截断只有工具预算一处，否则标记会叠加、分段可能被劈开。
    #[test]
    fn long_output_is_handed_over_whole() {
        let long = "x".repeat(40_000);
        let got = shell_receipt(&long, "", ShellEnd::Exit(Some(0)));
        assert_eq!(got.payload.chars().count(), 40_000);
        assert!(!got.payload.contains("[截断]"), "装配层不做预算");
    }
}
