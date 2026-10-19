//! 信封不合法的**回执文案装配**：按判定出的类别给出对应修法。
//!
//! 这是「协议形状 → 文案」的映射，必须认识 `Malformed` / `Tail` 的结构；文案归本模块所有，
//! 不再从提示词册取模板。

use crate::capabilities::llm::domain::envelope::{Malformed, Tail};

/// 未闭合的修法：内容写完只是少了收尾括号 → 直接说还差什么；断在字符串中间 → 才谈"分次写"。
fn unclosed_report(tail: &Tail) -> String {
    // 一段回复里起了两段信封：这不是"补个括号"能救的（末尾补括号补不到中间那段），
    // 而且补哪一段都是猜——如实说清，让模型只发一段。
    if tail.envelopes > 1 {
        return format!(
            "这一回复里起了 **{} 段**工具信封（第一段还差 {}，后面又起了一段）：核心一次只认一段，所以没有执行任何工具。请只发一段，并把它写完整；要一次发多个调用，请写在同一段的 calls 数组里。",
            tail.envelopes, tail.missing
        );
    }
    if tail.in_string {
        "工具信封**断在字符串中间**：内容没写完，补引号会让核心拿到半截内容，所以核心不猜、也没有执行工具。请重新输出完整信封；内容太长就分次写入。".to_string()
    } else {
        format!(
            "工具信封没写完：**还差 {}**（字符串里的内容已经闭合，只是信封没收尾；补上就完整了）。完整格式：{{\"type\":\"tool\",\"name\":\"工具名\",\"args\":{{...}}}}。",
            tail.missing
        )
    }
}

/// 附带说明：信封还差什么（与其它类别叠加时用）。
fn tail_note(tail: &Tail) -> String {
    if tail.in_string {
        "另外：这次输出**断在字符串中间**（内容没写完），请重新完整输出，或分次写入。".to_string()
    } else {
        format!(
            "另外：信封还差 {} 才完整（内容字符串已经闭合，补上收尾括号就对了）。",
            tail.missing
        )
    }
}

/// 工具信封不合法的回执：按**判定出的类别**给出对应修法（类别由 envelope 判定）。
pub fn malformed_report(kind: &Malformed) -> String {
    match kind {
        Malformed::Unclosed(tail) => unclosed_report(tail),
        Malformed::RawControl { ch, line, tail } => {
            let what = match ch {
                '\n' => "换行".to_string(),
                '\r' => "回车".to_string(),
                '\t' => "制表符".to_string(),
                other => format!("控制字符 U+{:04X}", *other as u32),
            };
            let mut out = format!(
                "工具信封的字符串里出现了未转义的裸{}（第 {} 行），核心没有执行任何工具。字符串里的换行要写成 \\n、制表符写成 \\t、回车写成 \\r（反斜杠加字母），不能直接换行。",
                what, line
            );
            // 同时还没闭合就一并说清（只说一处会让模型改错方向）
            if let Some(t) = tail {
                out.push('\n');
                out.push_str(&tail_note(t));
            }
            out
        }
        Malformed::Syntax(why) => format!(
            "工具信封不是合法 JSON：{}。核心没有执行任何工具，请按报出的位置检查该处（引号是否成对、逗号是否多余、字符串里的引号要转义）。",
            why
        ),
        Malformed::Shape(why) => format!(
            "工具信封是合法 JSON，但字段不合法：{}。核心没有执行任何工具。信封格式：{{\"type\":\"tool\",\"name\":\"工具名\",\"args\":{{...参数对象...}}}}；一次要发多个就改用 {{\"type\":\"tool\",\"calls\":[{{...}}, {{...}}]}}（两种形态不能混用）。",
            why
        ),
    }
}
