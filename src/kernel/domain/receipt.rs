//! 目的：进程执行回执的纯装配——本机守门进程与 guest 命令通道共用同一份 shell 回执形状。
//! 管：stdout 与 stderr 分区（`[stderr]` 头）、head+tail 截断、shell 状态行（退出码 / 超时 / 用户停止）。
//! 不管：超时 / 取消的判定（由执行层给）、文案本身（来自 ProcessTexts）、cwd 与耗时（由工具层追加）。
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
/// 参数：`texts` = 收尾文案；`out` / `err` = 两路输出；`end` = 收尾状态；`max_chars` = 正文上限（字符）。
/// 约束：截断只作用在正文，状态行永远留在最后；空输出不再留开头的空行。
pub fn shell_receipt(
    texts: &dyn ProcessTexts,
    out: &str,
    err: &str,
    end: ShellEnd,
    max_chars: usize,
) -> String {
    let mut body = out.to_string();
    if !err.is_empty() {
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str(&texts.stderr_header());
        body.push('\n');
        body.push_str(err);
    }
    let mut output = truncate_middle(texts, &body, max_chars);
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

/// 目的：head+tail 截断（开头约 1/4、结尾约 3/4），截断标记在中间；未超限原样返回。
/// 约束：**优先在行边界切**（开头取完整行、结尾从整行开始），不把一行劈成两段；
///   第一行超过开头预算、或"从整行开始"会丢掉结尾窗口大半（超长单行 + 后面的 stderr）时，
///   退化为按字符切——保住内容优先于凑整行。标记里报的是**实际保留**的字符数。
fn truncate_middle(texts: &dyn ProcessTexts, text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    let head_budget = max_chars / 4;
    let tail_budget = max_chars - head_budget;
    // 开头：截到预算内最后一个换行；换行本身让给标记行，不留在正文里。
    let head_raw: String = text.chars().take(head_budget).collect();
    let mut head = match head_raw.rfind('\n') {
        Some(at) => head_raw[..at].to_string(),
        None => head_raw.clone(),
    };
    if head.is_empty() {
        // 第一行就超过预算：没有行边界可用，按字符切。
        head = head_raw;
    }
    // 结尾：预算窗口若从行中间开始，优先从下一完整行开始；但若会丢掉窗口大半
    // （超长单行 + 后面只有 stderr），保留窗口原文——不能为了凑行把结尾内容丢掉。
    let tail_start = count - tail_budget;
    let tail_raw: String = text.chars().skip(tail_start).collect();
    let at_line_start = tail_start == 0 || text.chars().nth(tail_start - 1) == Some('\n');
    let line_started = if at_line_start {
        tail_raw.clone()
    } else {
        match tail_raw.find('\n') {
            Some(at) => tail_raw[at + 1..].to_string(),
            None => tail_raw.clone(),
        }
    };
    let tail = if line_started.chars().count() * 2 >= tail_raw.chars().count() {
        line_started
    } else {
        // 起步的半行太长（超长单行）：按字符切，保住结尾窗口。
        tail_raw
    };
    let kept_head = head.chars().count();
    let kept_tail = tail.chars().count();
    let marker = texts.truncated_middle(
        &count.to_string(),
        &kept_head.to_string(),
        &kept_tail.to_string(),
    );
    let mut out = String::new();
    if !head.is_empty() {
        out.push_str(&head);
        out.push('\n');
    }
    out.push_str(&marker);
    if !tail.is_empty() {
        out.push('\n');
        out.push_str(&tail);
    }
    out
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
        let out = shell_receipt(
            &texts(),
            "hello\n",
            "warn\n",
            ShellEnd::Exit(Some(3)),
            16_000,
        );
        assert_eq!(out, "hello\n\n[stderr]\nwarn\n\n[shell] 退出码 3");
        assert!(!out.contains("[shell] cwd"), "cwd 由工具层追加，不在装配里");
    }

    #[test]
    fn timeout_and_cancel_have_no_exit_line() {
        let timed = shell_receipt(&texts(), "", "", ShellEnd::TimedOut { secs: 2 }, 16_000);
        assert_eq!(
            timed,
            "[shell] 超时（2 秒）：工具进程与它拉起的整棵进程树已被终止"
        );
        let stopped = shell_receipt(&texts(), "", "", ShellEnd::Cancelled, 16_000);
        assert_eq!(
            stopped,
            "[shell] 用户停止：工具进程与它拉起的整棵进程树已被终止"
        );
        let signal = shell_receipt(&texts(), "", "", ShellEnd::Exit(None), 16_000);
        assert_eq!(signal, "[shell] 退出码未知（被信号结束）");
    }

    #[test]
    fn truncation_keeps_the_status_line() {
        let long = "x".repeat(40_000);
        let out = shell_receipt(&texts(), &long, "", ShellEnd::Exit(Some(0)), 16_000);
        assert!(out.contains("[截断]"), "超限要如实报");
        assert!(
            out.ends_with("[shell] 退出码 0"),
            "状态行必须在最后：{}",
            &out[out.len().saturating_sub(80)..]
        );
    }

    #[test]
    fn truncation_cuts_on_line_boundaries_and_reports_actual_counts() {
        let mut text = String::new();
        for i in 0..1_000 {
            text.push_str(&format!("line-{:04}\n", i));
        }
        let out = shell_receipt(&texts(), &text, "", ShellEnd::Exit(Some(0)), 120);
        let (before, after) = out.split_once("[截断]").expect("要有截断标记");
        let head = before.trim_end_matches('\n');
        let marker = after.lines().next().unwrap_or_default();
        assert!(
            marker.contains("按完整行取整"),
            "标记要说明切分口径：{}",
            marker
        );
        let marker_numbers: Vec<usize> = marker
            .split(|c: char| !c.is_ascii_digit())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .collect();
        assert_eq!(marker_numbers.len(), 3, "总数 / 开头 / 结尾：{}", marker);
        let rest = after.split_once('\n').map(|(_, t)| t).unwrap_or_default();
        // 尾行（状态行）由 shell_receipt 追加在正文之后，不算在「结尾保留」里。
        let tail = rest
            .rsplit_once("\n[shell] ")
            .map(|(t, _)| t)
            .unwrap_or(rest);
        assert_eq!(
            head.chars().count(),
            marker_numbers[1],
            "开头要报实际保留数"
        );
        assert_eq!(
            tail.chars().count(),
            marker_numbers[2],
            "结尾要报实际保留数"
        );
        assert!(
            marker_numbers[1] <= 30,
            "开头不超预算：{}",
            marker_numbers[1]
        );
        assert!(
            marker_numbers[2] <= 90,
            "结尾不超预算：{}",
            marker_numbers[2]
        );
        for line in head.lines().chain(tail.lines()) {
            assert!(
                line.starts_with("line-"),
                "截断不能把一行劈成两段：{:?}",
                line
            );
        }
    }

    #[test]
    fn tail_keeps_a_single_long_line_instead_of_dropping_it() {
        // macOS 真机回归：整段 stdout 是一条 2 万字符长行 + TAIL，后面只有一行 stderr。
        // 若"从整行开始"会丢掉整条长行、结尾只剩 53 字符；这里必须保留 12000 字符窗口。
        let mut text = "a".repeat(20_000);
        text.push_str("TAIL\n[stderr]\n[围栏] seatbelt 规则 37 条\n");
        let out = shell_receipt(&texts(), &text, "", ShellEnd::Exit(Some(0)), 16_000);
        let (before, after) = out.split_once("[截断]").expect("要有截断标记");
        let marker = after.lines().next().unwrap_or_default();
        assert_eq!(
            before.trim_end_matches('\n').chars().count(),
            4_000,
            "开头超长单行按字符切"
        );
        assert!(
            marker.contains("结尾 12000 字符"),
            "结尾要保住整个窗口：{}",
            marker
        );
        assert!(out.contains("TAIL"), "结尾内容不能被行取整丢掉");
        assert!(out.contains("seatbelt"), "stderr 段也要保留");
    }
}
