//! 目的：工具结果的**单次预算**——所有工具回执在送回模型前统一 head+tail 截断。
//! 管：字符上限、按完整行取整的截断、以及「怎么继续取」的标记文案；read 的续读 offset、shell 的状态行 / 末行都留在尾部。
//! 不管：各工具自己的分页（在 tools 的 systool）与分页文案；上下文压缩（在 session）。
//! 联动：tools 的 service（本机执行路径）与会话引擎（虚拟机档、核心自有工具）都调 cap_result；细则见 docs/tools/tools-and-roles.md 二。

/// 目的：单次工具结果回执的字符上限（约 4k tokens）。
/// 约束：超出按 head+tail 截断（开头约 1/4、结尾约 3/4），只加**一条**标记，报告实际保留字符数与续读办法。
pub const MAX_RESULT_CHARS: usize = 16_000;

/// 目的：截断标记末尾给模型的**续读办法**：按这个工具真能做到的写法给，不给做不到的建议。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Continuation {
    /// 目的：执行类（shell 与模块工具）——stdout 不是文件，read 的 offset/limit 接不上。
    Command,
    /// 目的：文件类（read / search / list 等）——真的能按 offset/limit 接着读。
    Read,
}

impl Continuation {
    /// 目的：这条建议的文案本身（随 [工具结果] 进模型上下文）。
    fn hint(self) -> &'static str {
        match self {
            Continuation::Command => {
                "需要更多请缩小命令的输出（只取要看的行、或用 head/tail 之类的筛选），或把输出写到文件再用 read 分段查看。"
            }
            Continuation::Read => "需要更多请用 offset/limit 分段读取。",
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

/// 目的：把一次工具回执收进统一预算；未超限原样返回。
/// 约束：**优先在行边界切**（开头取完整行、结尾从整行开始），不把一行劈成两段；超长单行退化为按字符切。
///   已经把预算收过的回执原样返回——预算只该收一次，再收一次只会多出一条自相矛盾的标记。
pub fn cap_result(text: String, hint: Continuation) -> String {
    let total = text.chars().count();
    if total <= MAX_RESULT_CHARS || already_capped(&text, total) {
        return text;
    }
    let head_budget = MAX_RESULT_CHARS / 4;
    let tail_budget = MAX_RESULT_CHARS - head_budget;
    let (head, head_on_line) = cut_head(&text, head_budget);
    let (tail, tail_on_line) = cut_tail(&text, total - tail_budget);
    let how = if head_on_line && tail_on_line {
        "按完整行取整"
    } else {
        "超长单行按字符切"
    };
    let marker = format!(
        "[结果截断] 输出共 {} 字符；保留开头 {} 与结尾 {} 字符（{}）。{}",
        total,
        head.chars().count(),
        tail.chars().count(),
        how,
        hint.hint()
    );
    let mut out = String::with_capacity(MAX_RESULT_CHARS + marker.len() + 2);
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

/// 目的：这条回执是不是已经收过预算（标记是行首标记；上限附近才认，避免把正文里恰好同形的行当标记）。
fn already_capped(text: &str, total: usize) -> bool {
    total <= MAX_RESULT_CHARS + 600
        && text.lines().any(|line| {
            line.starts_with("[结果截断] ") || line.starts_with("[截断] 输出共 ")
        })
}

/// 目的：开头窗口——取 budgets 个字符内的最后一个换行之前；第一行就超预算时按字符切（返回 false）。
fn cut_head(text: &str, budget: usize) -> (String, bool) {
    let end = text
        .char_indices()
        .nth(budget)
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    let raw = &text[..end];
    match raw.rfind('\n') {
        Some(at) if at > 0 => (raw[..at].to_string(), true),
        _ => (raw.to_string(), false),
    }
}

/// 目的：结尾窗口——从 start 个字符处起；不在行首时优先从下一整行开始，但若会丢掉窗口大半则按字符切（返回 false）。
fn cut_tail(text: &str, start: usize) -> (String, bool) {
    let at = text
        .char_indices()
        .nth(start)
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    let raw = &text[at..];
    if at == 0 || text[..at].ends_with('\n') {
        return (raw.to_string(), true);
    }
    match raw.find('\n') {
        Some(nl) if raw[nl + 1..].chars().count() * 2 >= raw.chars().count() => {
            (raw[nl + 1..].to_string(), true)
        }
        _ => (raw.to_string(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_results_are_untouched() {
        let s = "短回执".to_string();
        assert_eq!(cap_result(s.clone(), Continuation::Read), s);
    }

    /// 目的：长回执只加一条标记，开头结尾都按完整行取整，并报实际保留的字符数。
    #[test]
    fn long_results_keep_whole_lines_with_one_marker() {
        let mut text = String::new();
        for i in 0..3_000 {
            text.push_str(&format!("line-{:04}\n", i));
        }
        let out = cap_result(text, Continuation::Read);
        assert_eq!(out.matches("[结果截断]").count(), 1, "只许一条标记");
        assert!(out.contains("按完整行取整"), "要说明切分口径");
        assert!(out.contains("offset/limit"), "文件类要给续读办法");
        assert!(out.chars().count() <= MAX_RESULT_CHARS + 200, "预算要真的生效");
        assert!(out.ends_with("line-2999\n"), "结尾保留最后一行");
        assert!(out.starts_with("line-0000\n"), "开头保留第一行");
        for line in out.lines().filter(|l| l.starts_with("line-")) {
            assert_eq!(line.len(), 9, "不能把一行劈成两段：{:?}", line);
        }
    }

    /// 目的：执行类的续读办法不给 read 的 offset/limit——shell 的 stdout 不是文件，接不上。
    #[test]
    fn command_results_get_a_command_hint() {
        let out = cap_result("x".repeat(MAX_RESULT_CHARS * 2), Continuation::Command);
        assert!(!out.contains("offset/limit"), "执行类不能建议 read 接续");
        assert!(out.contains("输出写到文件"), "要给真能用的出路");
    }

    /// 目的：整段只有一条超长行时退化为按字符切，结尾内容不被行取整丢掉。
    #[test]
    fn a_single_long_line_falls_back_to_char_cut() {
        let mut text = "a".repeat(20_000);
        text.push_str("TAIL\n");
        let out = cap_result(text, Continuation::Read);
        assert!(out.contains("超长单行按字符切"), "要如实说清口径");
        assert!(out.contains("TAIL"), "结尾内容不能丢");
        assert!(out.starts_with(&"a".repeat(4_000)), "开头超长单行按字符切");
    }

    /// 目的：已经收过预算的回执再收一次不变——预算只落在最末一处，不会叠出第二条标记。
    #[test]
    fn a_capped_result_is_left_alone() {
        let once = cap_result("y".repeat(MAX_RESULT_CHARS * 3), Continuation::Read);
        let twice = cap_result(once.clone(), Continuation::Command);
        assert_eq!(twice, once);
    }
}
