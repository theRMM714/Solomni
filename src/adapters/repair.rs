//! 默认的信封修复器：**只做一件无歧义的事**——把 JSON 字符串里未转义的控制字符转义。
//! 为什么只做这一件：内容里直接换行/制表符是"看得出想写什么"的错（真实事故里 write 的 content 就是这么写坏的），
//! 而缺括号、引号不配对之类的错都可能有多解——那些一律不修，让模型重发一次。
//! 机制在适配层：任何满足 core::ports::EnvelopeRepair 契约的实现都能整体替换本实现。

use crate::core::envelope::Malformed;
use crate::core::ports::{EnvelopeRepair, RepairOutcome};

/// 只转义字符串内部的裸控制字符（其余一律不碰）。
pub struct EscapeControls;

impl EnvelopeRepair for EscapeControls {
    fn repair(&self, raw: &str, kind: &Malformed) -> RepairOutcome {
        // 只有"字符串里有裸控制字符"这一类是能无歧义修好的；其余类别交给模型重发。
        if !matches!(kind, Malformed::RawControl { .. }) {
            return RepairOutcome {
                repaired: None,
                what: Vec::new(),
            };
        }
        let (repaired, what) = escape_controls(raw);
        if what.is_empty() {
            return RepairOutcome {
                repaired: None,
                what: Vec::new(),
            };
        }
        RepairOutcome {
            repaired: Some(repaired),
            what,
        }
    }
}

/// 转义字符串内部的裸控制字符；返回（新文本, 做了哪些转义的说明）。
/// 字符串外的换行是合法 JSON 空白，一律不动（避免顺手改掉信封之外的正文）。
fn escape_controls(raw: &str) -> (String, Vec<String>) {
    let mut out = String::with_capacity(raw.len());
    let mut what: Vec<String> = Vec::new();
    let mut in_str = false;
    let mut esc = false;
    for c in raw.chars() {
        if !in_str {
            if c == '"' {
                in_str = true;
            }
            out.push(c);
            continue;
        }
        if esc {
            esc = false;
            out.push(c);
            continue;
        }
        match c {
            '\\' => {
                esc = true;
                out.push(c);
            }
            '"' => {
                in_str = false;
                out.push(c);
            }
            '\n' => {
                out.push_str("\\n");
                note(&mut what, "换行 → \\n");
            }
            '\r' => {
                out.push_str("\\r");
                note(&mut what, "回车 → \\r");
            }
            '\t' => {
                out.push_str("\\t");
                note(&mut what, "制表符 → \\t");
            }
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
                note(
                    &mut what,
                    &format!("控制字符 U+{:04X} → \\u{:04x}", c as u32, c as u32),
                );
            }
            c => out.push(c),
        }
    }
    (out, what)
}

/// 同一种转义只说一次（回执要短、要能一眼看完）。
fn note(list: &mut Vec<String>, what: &str) {
    if !list.iter().any(|x| x == what) {
        list.push(what.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_controls_inside_strings_are_escaped() {
        let raw = "好的。{\"type\":\"tool\",\"name\":\"write\",\"args\":{\"path\":\"a\",\"content\":\"第一行\n第二行\"}}";
        let out = EscapeControls.repair(raw, &Malformed::RawControl { ch: '\n', line: 1 });
        let fixed = out.repaired.expect("这一类必须能修");
        assert!(fixed.contains("第一行\\n第二行"), "{}", fixed);
        assert!(fixed.starts_with("好的。"), "信封之外的正文不动：{}", fixed);
        // 修好之后必须是**合法工具信封**，而且字段含义不变（这是"无歧义"的判据）
        let again = crate::core::envelope::parse(&fixed);
        let inv = again.tool.expect("修好后就是工具信封");
        assert!(inv.malformed.is_none(), "修好即合法");
        let args: serde_json::Value = serde_json::from_str(&inv.args_json).expect("参数是 JSON");
        assert_eq!(
            args["content"],
            serde_json::json!("第一行\n第二行"),
            "内容与模型想写的一致"
        );
        assert_eq!(out.what.len(), 1, "同一种转义只说一次：{:?}", out.what);
        // 信封之外的换行（合法空白）不动
        let with_prose =
            "好的。\n{\"type\":\"tool\",\"name\":\"read\",\"args\":{\"path\":\"a\n\"}}";
        let out2 = EscapeControls.repair(with_prose, &Malformed::RawControl { ch: '\n', line: 2 });
        let fixed2 = out2.repaired.expect("也能修");
        assert!(
            fixed2.starts_with("好的。\n{"),
            "正文里的换行不动：{:?}",
            fixed2
        );
    }

    #[test]
    fn other_kinds_are_left_to_the_model() {
        for kind in [
            Malformed::Unclosed,
            Malformed::Syntax("x".into()),
            Malformed::Shape("y".into()),
        ] {
            let out = EscapeControls.repair("{\"type\":\"tool\"", &kind);
            assert!(
                out.repaired.is_none() && out.what.is_empty(),
                "别的类别一律不猜：{:?}",
                kind
            );
        }
        // 声明是控制字符、实际却找不到（诊断与实际不符）：如实说没修
        let out = EscapeControls.repair("{\"a\":1}", &Malformed::RawControl { ch: '\n', line: 1 });
        assert!(out.repaired.is_none());
    }
}
