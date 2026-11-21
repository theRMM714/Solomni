//! 目的：工具结果的**单次预算**——所有工具回执在送回模型前统一 head+tail 截断。
//! 管：字符上限、按完整行取整的截断，以及「被截断」与「怎么继续取」的结构化说明（稳定码，文案在目录里）。
//! 不管：各工具自己的分页（在 tools 的 systool）与分页文案；上下文压缩（在 session）。
//! 联动：tools 的 service 与 conversation 的引擎都调 cap_result；细则见 docs/tools/tools-and-roles.md。

use crate::kernel::api::note::Note;
use crate::kernel::api::note::codes;
use crate::kernel::api::types::ToolOutcome;

/// 目的：单次工具结果回执的字符上限（约 4k tokens）。
/// 约束：超出按 head+tail 截断（开头约 1/4、结尾约 3/4），只加**一组**说明，报告实际保留字符数与续读办法。
pub const MAX_RESULT_CHARS: usize = 16_000;

/// 目的：截断之后给模型的**续读办法**：按这个工具真能做到的写法给，不给做不到的建议。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Continuation {
    /// 目的：执行类（shell 与模块工具）——stdout 不是文件，read 的 offset/limit 接不上。
    Command,
    /// 目的：文件类（read / search / list 等）——真的能按 offset/limit 接着读。
    Read,
}

impl Continuation {
    /// 目的：这条续读办法的稳定码（文案在 `locales/<lang>.ftl`）。
    fn code(self) -> &'static str {
        match self {
            Continuation::Command => codes::RESULT_TRUNCATED_COMMAND,
            Continuation::Read => codes::RESULT_TRUNCATED_READ,
        }
    }
}

/// 目的：按「模块 id + 工具名」判这次调用怎么续读：模块工具与 shell 属执行类，其余按文件类。
pub fn continuation_for(module: &str, tool: &str) -> Continuation {
    if !module.is_empty() || crate::capabilities::tools::api::is_exec(tool) {
        Continuation::Command
    } else {
        Continuation::Read
    }
}

/// 目的：把一次工具回执收进统一预算；未超限原样不动。
/// 约束：**优先在行边界切**（开头取完整行、结尾从整行开始），不把一行劈成两段；超长单行退化为按字符切。
///   已经把预算收过的回执原样不动——预算只该收一次，再收一次只会多出一组自相矛盾的说明。
/// 约束：真截断时正文换成「开头 + 结尾」，并追加 `result.truncated` 与一条续读说明。
pub fn cap_result(outcome: &mut ToolOutcome, hint: Continuation) {
    let text = std::mem::take(&mut outcome.output);
    let total = text.chars().count();
    if total <= MAX_RESULT_CHARS || already_capped(&outcome.notes) {
        outcome.output = text;
        return;
    }
    let head_budget = MAX_RESULT_CHARS / 4;
    let tail_budget = MAX_RESULT_CHARS - head_budget;
    let head = cut_head(&text, head_budget);
    let tail = cut_tail(&text, total - tail_budget);
    let mut out = String::with_capacity(MAX_RESULT_CHARS + 2);
    if !head.is_empty() {
        out.push_str(&head);
    }
    if !tail.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&tail);
    }
    outcome.output = out;
    outcome.push_note(
        Note::warning(codes::RESULT_TRUNCATED)
            .arg("total", total)
            .arg("head", head.chars().count())
            .arg("tail", tail.chars().count()),
    );
    outcome.push_note(Note::info(hint.code()));
}

/// 目的：这条回执是不是已经收过预算——只看说明里的稳定码。
fn already_capped(notes: &[Note]) -> bool {
    notes.iter().any(|n| n.code == codes::RESULT_TRUNCATED)
}

/// 目的：开头窗口——取 budget 个字符内的最后一个换行之前；第一行就超预算时按字符切。
fn cut_head(text: &str, budget: usize) -> String {
    let end = text
        .char_indices()
        .nth(budget)
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    let raw = &text[..end];
    match raw.rfind('\n') {
        Some(at) if at > 0 => raw[..at].to_string(),
        _ => raw.to_string(),
    }
}

/// 目的：结尾窗口——从 start 个字符处起；不在行首时优先从下一整行开始，但若会丢掉窗口大半则按字符切。
fn cut_tail(text: &str, start: usize) -> String {
    let at = text
        .char_indices()
        .nth(start)
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    let raw = &text[at..];
    if at == 0 || text[..at].ends_with('\n') {
        return raw.to_string();
    }
    match raw.find('\n') {
        Some(nl) if raw[nl + 1..].chars().count() * 2 >= raw.chars().count() => {
            raw[nl + 1..].to_string()
        }
        _ => raw.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::api::note::Severity;

    fn outcome(text: String) -> ToolOutcome {
        ToolOutcome::new(true, text)
    }

    fn codes_of(out: &ToolOutcome) -> Vec<&str> {
        out.notes.iter().map(|n| n.code.as_str()).collect()
    }

    #[test]
    fn short_results_are_untouched() {
        let mut out = outcome("短回执".to_string());
        cap_result(&mut out, Continuation::Read);
        assert_eq!(out.output, "短回执");
        assert!(out.notes.is_empty());
        assert_eq!(out.severity, Severity::Info);
    }

    /// 目的：长回执只加一组说明，开头结尾都按完整行取整，并报实际保留的字符数。
    #[test]
    fn long_results_keep_whole_lines_with_one_group_of_notes() {
        let mut text = String::new();
        for i in 0..3_000 {
            text.push_str(&format!("line-{:04}\n", i));
        }
        let total = text.chars().count();
        let mut out = outcome(text);
        cap_result(&mut out, Continuation::Read);
        assert_eq!(
            codes_of(&out),
            vec![codes::RESULT_TRUNCATED, codes::RESULT_TRUNCATED_READ],
            "一组说明：截断 + 续读办法"
        );
        assert_eq!(out.severity, Severity::Warning, "截断要显眼");
        let truncated = &out.notes[0];
        assert_eq!(
            truncated.args.get("total").map(String::as_str),
            Some(total.to_string().as_str())
        );
        assert!(
            out.output.chars().count() <= MAX_RESULT_CHARS + 200,
            "预算要真的生效"
        );
        assert!(out.output.ends_with("line-2999\n"), "结尾保留最后一行");
        assert!(out.output.starts_with("line-0000\n"), "开头保留第一行");
        for line in out.output.lines().filter(|l| l.starts_with("line-")) {
            assert_eq!(line.len(), 9, "不能把一行劈成两段：{:?}", line);
        }
        assert!(
            !out.output.contains("[结果截断]"),
            "文案不进正文，只在说明里"
        );
    }

    /// 目的：执行类的续读办法不给 read 的 offset/limit——shell 的 stdout 不是文件，接不上。
    #[test]
    fn command_results_get_a_command_hint() {
        let mut out = outcome("x".repeat(MAX_RESULT_CHARS * 2));
        cap_result(&mut out, Continuation::Command);
        assert_eq!(codes_of(&out)[1], codes::RESULT_TRUNCATED_COMMAND);
        assert!(
            !out.notes
                .iter()
                .any(|n| n.code == codes::RESULT_TRUNCATED_READ)
        );
    }

    /// 目的：整段只有一条超长行时退化为按字符切，结尾内容不被行取整丢掉。
    #[test]
    fn a_single_long_line_falls_back_to_char_cut() {
        let mut text = "a".repeat(20_000);
        text.push_str("TAIL\n");
        let mut out = outcome(text);
        cap_result(&mut out, Continuation::Read);
        assert!(out.output.contains("TAIL"), "结尾内容不能丢");
        assert!(out.output.starts_with(&"a".repeat(4_000)), "开头按字符切");
    }

    /// 目的：已经收过预算的回执再收一次不变——预算只落在最末一处，不会叠出第二组说明。
    #[test]
    fn a_capped_result_is_left_alone() {
        let mut once = outcome("y".repeat(MAX_RESULT_CHARS * 3));
        cap_result(&mut once, Continuation::Read);
        let before = once.clone();
        cap_result(&mut once, Continuation::Command);
        assert_eq!(once, before);
    }
}
