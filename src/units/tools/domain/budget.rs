//! 目的：工具结果的**上下文预算**——所有工具回执在送回模型前统一 head+tail 截断。
//! 管：单条上限、按整批剩余上下文收紧的总预算、按完整行取整的截断，以及「被截断」与「怎么继续取」的结构化说明。
//! 不管：各工具自己的分页（在 tools 的 systool）与分页文案；上下文压缩（在 session）。
//! 联动：tools 的 service 与 conversation 的引擎都调 cap_result / cap_result_with；细则见 docs/tools/tools-and-roles.md。

use crate::kernel::api::note::Note;
use crate::kernel::api::note::codes;
use crate::kernel::api::types::ToolOutcome;

/// 目的：单条工具结果回执的字符上限（约 4k tokens）。
/// 约束：整批预算只能把它往下压，不能抬高；超出按 head+tail 截断（开头约 1/4、结尾约 3/4）。
pub const MAX_RESULT_CHARS: usize = 16_000;

/// 目的：单条回执的保底字符数（约 250 tokens）。
/// 约束：整批平分后低于它，说明剩余上下文连一条最小回执都放不下——整批预检失败，不执行任何工具。
pub const MIN_RESULT_CHARS: usize = 1_000;

/// 目的：把 token 粗算成字符（与 ContextUsage::estimated 同一口径）。
const CHARS_PER_TOKEN: u64 = 4;
/// 目的：整批工具结果最多占用剩余上下文的比例。
const CONTEXT_SHARE_PERCENT: u64 = 50;
/// 目的：给下一轮对话留的 token 余量上限。
const RESERVE_TOKENS: u64 = 2_000;

/// 目的：这一次工具批次的回执预算——单条上限按整批剩余上下文收紧。
/// 约束：exhausted 时调用方必须**整批**不执行；pressure 表示已经从单条上限往下压。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResultBudget {
    limit: usize,
    pressure: bool,
    exhausted: bool,
}

impl ResultBudget {
    /// 目的：不按上下文收紧的静态预算（非模型通道与测试直调使用）。
    pub fn full() -> ResultBudget {
        ResultBudget {
            limit: MAX_RESULT_CHARS,
            pressure: false,
            exhausted: false,
        }
    }

    /// 目的：按整批剩余上下文算每条回执的字符上限。
    /// 约束：window / used_tokens 来自供应商 usage 或会话估算；calls 是本批工具数（至少按 1 算）。
    ///   预算 = (窗口 - 已用 - reserve) x 50% x 4 字符/token，再按调用数平分；低于保底即枯竭。
    pub fn for_batch(window: u64, used_tokens: u64, calls: usize) -> ResultBudget {
        if window == 0 || calls == 0 {
            return ResultBudget::full();
        }
        let reserve = RESERVE_TOKENS.min(window / 10);
        let available = window.saturating_sub(used_tokens).saturating_sub(reserve);
        let batch_chars = available.saturating_mul(CONTEXT_SHARE_PERCENT) / 100 * CHARS_PER_TOKEN;
        let per_call = batch_chars / calls as u64;
        if per_call < MIN_RESULT_CHARS as u64 {
            return ResultBudget {
                limit: MIN_RESULT_CHARS,
                pressure: true,
                exhausted: true,
            };
        }
        let limit = per_call.min(MAX_RESULT_CHARS as u64) as usize;
        ResultBudget {
            limit,
            pressure: limit < MAX_RESULT_CHARS,
            exhausted: false,
        }
    }

    /// 目的：本批每条回执可用的字符上限。
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// 目的：是否已从静态单条上限往下压（截断时要额外提示压缩）。
    pub fn pressure(&self) -> bool {
        self.pressure
    }

    /// 目的：剩余上下文连一条最小回执都放不下——本批必须整体不执行。
    pub fn exhausted(&self) -> bool {
        self.exhausted
    }
}

/// 目的：截断之后给模型的**续读办法**：按这个工具真能做到的写法给，不给做不到的建议。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Continuation {
    /// 目的：执行类（shell 与模块工具）——stdout 不是文件，read 的 offset/limit 接不上。
    Command,
    /// 目的：文件类（read / grep / list 等）——真的能按 offset/limit 接着读。
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
    if !module.is_empty() || crate::units::tools::api::is_exec(tool) {
        Continuation::Command
    } else {
        Continuation::Read
    }
}

/// 目的：把一次工具回执收进静态预算；未超限原样不动。
/// 约束：只给测试直调；模型通道一律走 cap_result_with。
#[cfg(test)]
pub fn cap_result(outcome: &mut ToolOutcome, hint: Continuation) {
    cap_result_with(outcome, hint, ResultBudget::full());
}

/// 目的：把一次工具回执收进本批预算；未超限原样不动。
/// 约束：**优先在行边界切**（开头取完整行、结尾从整行开始），不把一行劈成两段；超长单行退化为按字符切。
///   已按更大上限截过的回执允许二次收缩；已按同等或更小上限截过的原样不动，避免叠出自相矛盾的说明。
/// 约束：真截断时正文换成「开头 + 结尾」，追加 `result.truncated`、一条续读说明；预算被压过时再加一条压缩提示。
pub fn cap_result_with(outcome: &mut ToolOutcome, hint: Continuation, budget: ResultBudget) {
    if budget.exhausted() {
        return;
    }
    let limit = budget.limit().max(1);
    let text = std::mem::take(&mut outcome.output);
    let total = text.chars().count();
    if total <= limit {
        outcome.output = text;
        return;
    }
    if let Some(prev) = capped_limit(&outcome.notes) {
        if prev <= limit {
            outcome.output = text;
            return;
        }
        // 上一次按更大的上限截过：先撤掉旧的截断说明，再按新上限重截（正文是已截过的，只能再收）。
        outcome.notes.retain(|n| !is_budget_note(n));
    }
    let head_budget = limit / 4;
    let tail_budget = limit - head_budget;
    let head = cut_head(&text, head_budget);
    let tail = cut_tail(&text, total - tail_budget);
    let mut out = String::with_capacity(limit + 2);
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
            .arg("tail", tail.chars().count())
            .arg("limit", limit),
    );
    if budget.pressure() {
        outcome.push_note(Note::warning(codes::RESULT_TRUNCATED_CONTEXT).arg("limit", limit));
    }
    outcome.push_note(Note::info(hint.code()));
}

/// 目的：这条回执上一次按多少字符截的——旧说明没带 limit 时按静态上限算。
fn capped_limit(notes: &[Note]) -> Option<usize> {
    notes
        .iter()
        .find(|n| n.code == codes::RESULT_TRUNCATED)
        .map(|n| {
            n.args
                .get("limit")
                .and_then(|v| v.parse().ok())
                .unwrap_or(MAX_RESULT_CHARS)
        })
}

/// 目的：哪些说明属于预算截断（二次收缩时要先撤掉，避免两组数字并存）。
fn is_budget_note(note: &Note) -> bool {
    matches!(
        note.code.as_str(),
        codes::RESULT_TRUNCATED
            | codes::RESULT_TRUNCATED_COMMAND
            | codes::RESULT_TRUNCATED_READ
            | codes::RESULT_TRUNCATED_CONTEXT
    )
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

    /// 目的：整批预算按剩余上下文与调用数平分，且最多只占剩余的一半。
    #[test]
    fn batch_budget_shares_the_remaining_context() {
        let full = ResultBudget::for_batch(128_000, 0, 1);
        assert_eq!(full.limit(), MAX_RESULT_CHARS);
        assert!(!full.pressure() && !full.exhausted());

        let tight = ResultBudget::for_batch(128_000, 100_000, 10);
        assert_eq!(tight.limit(), 5_200, "26k 可用 token 的一半、10 条平分");
        assert!(tight.pressure() && !tight.exhausted());
        assert!(
            tight.limit() * 10 <= (128_000 - 100_000) * 2,
            "整批不超剩余一半"
        );

        let low = ResultBudget::for_batch(20_000, 15_000, 1);
        assert_eq!(low.limit(), 6_000);
        assert!(low.pressure());

        let edge = ResultBudget::for_batch(20_000, 17_500, 1);
        assert_eq!(edge.limit(), MIN_RESULT_CHARS);
        assert!(!edge.exhausted());

        let gone = ResultBudget::for_batch(20_000, 18_000, 1);
        assert!(gone.exhausted(), "连一条最小回执都放不下");

        assert_eq!(ResultBudget::for_batch(0, 0, 0), ResultBudget::full());
    }

    /// 目的：预算被压过且真的截断时，额外提示先压缩会话。
    #[test]
    fn a_pressured_cut_asks_for_compaction() {
        let mut out = outcome("z".repeat(4_000));
        let budget = ResultBudget::for_batch(20_000, 16_500, 1);
        assert_eq!(budget.limit(), 3_000);
        cap_result_with(&mut out, Continuation::Read, budget);
        assert!(out.output.chars().count() <= 3_200, "按本批上限截");
        assert_eq!(
            codes_of(&out),
            vec![
                codes::RESULT_TRUNCATED,
                codes::RESULT_TRUNCATED_CONTEXT,
                codes::RESULT_TRUNCATED_READ
            ]
        );
        assert_eq!(
            out.notes[0].args.get("limit").map(String::as_str),
            Some("3000")
        );
    }

    /// 目的：已按更大上限截过的回执，遇到更小预算时允许二次收缩，且只留一组说明。
    #[test]
    fn a_larger_cap_can_shrink_to_a_smaller_budget() {
        let mut out = outcome("q".repeat(MAX_RESULT_CHARS * 3));
        cap_result(&mut out, Continuation::Read);
        let budget = ResultBudget::for_batch(20_000, 16_500, 1);
        cap_result_with(&mut out, Continuation::Command, budget);
        assert!(out.output.chars().count() <= 3_200);
        let truncations = out
            .notes
            .iter()
            .filter(|n| n.code == codes::RESULT_TRUNCATED)
            .count();
        assert_eq!(truncations, 1, "只留一组截断说明");
        assert!(
            out.notes
                .iter()
                .any(|n| n.code == codes::RESULT_TRUNCATED_COMMAND)
        );
    }
}
