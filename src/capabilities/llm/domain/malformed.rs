//! 信封不合法的**回执文案装配**：按判定出的类别给出对应修法。
//!
//! **为什么在 llm 而不是 prompt**：这是「协议形状 → 文案」的映射，必须认识 `Malformed` / `Tail` 的结构；
//! 模板本身仍住在 `prompt`（`ToolTexts`），这里只做选择与拼装。放在 `prompt` 会让提示词能力
//! 反过来依赖模型通道能力，把 `prompt` 拉进环（见 docs/architecture/refactor-plan.md §4.2 批次 10）。

use crate::capabilities::llm::domain::envelope::{Malformed, Tail};
use crate::capabilities::prompt::api::ToolTexts;

/// 未闭合的修法：内容写完只是少了收尾括号 → 直接说还差什么；断在字符串中间 → 才谈"分次写"。
fn unclosed_report(texts: &ToolTexts, tail: &Tail) -> String {
    // 一段回复里起了两段信封：这不是"补个括号"能救的（末尾补括号补不到中间那段），
    // 而且补哪一段都是猜——如实说清，让模型只发一段。
    if tail.envelopes > 1 {
        return texts.render(
            &texts.malformed_multi,
            &[
                ("n", tail.envelopes.to_string()),
                ("missing", tail.missing.clone()),
            ],
        );
    }
    if tail.in_string {
        texts.malformed_unclosed_string.clone()
    } else {
        texts.render(
            &texts.malformed_unclosed_brace,
            &[("missing", tail.missing.clone())],
        )
    }
}

/// 附带说明：信封还差什么（与其它类别叠加时用）。
fn tail_note(texts: &ToolTexts, tail: &Tail) -> String {
    if tail.in_string {
        texts.malformed_cut_string.clone()
    } else {
        texts.render(
            &texts.malformed_missing_tail,
            &[("missing", tail.missing.clone())],
        )
    }
}

/// 工具信封不合法的回执：按**判定出的类别**给出对应修法（类别由 envelope 判定，文案在 prompt）。
pub fn malformed_report(texts: &ToolTexts, kind: &Malformed) -> String {
    match kind {
        Malformed::Unclosed(tail) => unclosed_report(texts, tail),
        Malformed::RawControl { ch, line, tail } => {
            let what = match ch {
                '\n' => texts.control_lf.clone(),
                '\r' => texts.control_cr.clone(),
                '\t' => texts.control_tab.clone(),
                other => texts.render(
                    &texts.control_other,
                    &[("code", format!("{:04X}", *other as u32))],
                ),
            };
            let mut out = texts.render(
                &texts.malformed_control,
                &[("what", what), ("line", line.to_string())],
            );
            // 同时还没闭合就一并说清（只说一处会让模型改错方向）
            if let Some(t) = tail {
                out.push('\n');
                out.push_str(&tail_note(texts, t));
            }
            out
        }
        Malformed::Syntax(why) => texts.render(&texts.malformed_syntax, &[("why", why.clone())]),
        Malformed::Shape(why) => texts.render(&texts.malformed_shape, &[("why", why.clone())]),
    }
}
