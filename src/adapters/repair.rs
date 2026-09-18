//! 默认的信封修复器：**只做无歧义的修补**（两类，都可机器判定）。
//!
//! ①把 JSON 字符串里未转义的裸控制字符转义（\\n / \\r / \\t / \\uXXXX）——
//!   内容里直接换行/制表符是"看得出想写什么"的错（真实事故里 write 的 content 就是这么写坏的）。
//! ②信封还差收尾括号时，把**扫描器算出来的那几个字符**补上——真实会话里模型四次失败都是这个形状：
//!   内容字符串已经写完，只因信封少一个 } 而整轮作废（它还被误导去"分两次写"，白跑两轮）。
//!
//! 两条红线：
//! - 断在**字符串中间**（内容没写完）一律不修：补个引号会让核心拿到半截内容，那是更坏的结果。
//! - 只补"缺的那几个收尾字符"，不做任何猜测性改写；结果由引擎重新解析成**合法工具信封**才算数
//!   （解析通过就是"无歧义"的判据）。
//!
//! 机制在适配层：任何满足 core::ports::EnvelopeRepair 契约的实现都能整体替换本实现。

use crate::core::envelope::{Malformed, Tail};
use crate::core::ports::{EnvelopeRepair, RepairOutcome};

/// 只做上述两类无歧义修补。
pub struct UnambiguousRepair;

impl EnvelopeRepair for UnambiguousRepair {
    fn repair(&self, raw: &str, kind: &Malformed) -> RepairOutcome {
        match kind {
            // ① 裸控制字符（可能同时还差收尾括号）
            Malformed::RawControl { tail, .. } => {
                let (text, mut what) = escape_controls(raw);
                if what.is_empty() {
                    return nothing();
                }
                let text = match tail {
                    // 断在字符串中间：转义也救不回来（字符串没闭合），不修
                    Some(t) if t.in_string => return nothing(),
                    Some(t) if !t.missing.is_empty() => {
                        what.push(format!("补上缺的收尾 {}", t.missing));
                        format!("{}{}", text, t.missing)
                    }
                    _ => text,
                };
                RepairOutcome {
                    repaired: Some(text),
                    what,
                }
            }
            // ② 内容写完、只差信封收尾
            Malformed::Unclosed(tail) => match brace_fix(raw, tail) {
                Some(text) => RepairOutcome {
                    repaired: Some(text),
                    what: vec![format!("补上缺的收尾 {}", tail.missing)],
                },
                None => nothing(),
            },
            // 语法错/字段不合法都可能有多解，一律不猜（让模型重发）
            _ => nothing(),
        }
    }
}

/// 补收尾括号：断在字符串中间、或本来就不缺，都不修。
fn brace_fix(raw: &str, tail: &Tail) -> Option<String> {
    // 一段回复里起了两段信封：末尾补括号补不到中间那段的收尾，补哪一段都是猜（宁缺毋滥）。
    if tail.in_string || tail.missing.is_empty() || tail.envelopes > 1 {
        return None;
    }
    Some(format!("{}{}", raw, tail.missing))
}

fn nothing() -> RepairOutcome {
    RepairOutcome {
        repaired: None,
        what: Vec::new(),
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
    use crate::core::envelope::parse;

    fn raw_control(tail: Option<Tail>) -> Malformed {
        Malformed::RawControl {
            ch: '\n',
            line: 1,
            tail,
        }
    }

    fn tail(missing: &str, in_string: bool) -> Tail {
        Tail {
            missing: missing.to_string(),
            in_string,
            envelopes: 1,
        }
    }

    /// 真实事故的形状：一段回复里起了两段信封 → 末尾补括号救不了，一律不修。
    fn tail_multi(missing: &str) -> Tail {
        Tail {
            missing: missing.to_string(),
            in_string: false,
            envelopes: 2,
        }
    }

    #[test]
    fn raw_controls_inside_strings_are_escaped() {
        let raw = "好的。{\"type\":\"tool\",\"name\":\"write\",\"args\":{\"path\":\"a\",\"content\":\"第一行\n第二行\"}}";
        let out = UnambiguousRepair.repair(raw, &raw_control(None));
        let fixed = out.repaired.expect("这一类必须能修");
        assert!(fixed.contains("第一行\\n第二行"), "{}", fixed);
        assert!(fixed.starts_with("好的。"), "信封之外的正文不动：{}", fixed);
        // 修好之后必须是**合法工具信封**，而且字段含义不变（这是"无歧义"的判据）
        let again = parse(&fixed);
        let inv = again.tools.into_iter().next().expect("修好后就是工具信封");
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
        let out2 = UnambiguousRepair.repair(with_prose, &raw_control(None));
        let fixed2 = out2.repaired.expect("也能修");
        assert!(
            fixed2.starts_with("好的。\n{"),
            "正文里的换行不动：{:?}",
            fixed2
        );
    }

    #[test]
    fn a_missing_closing_brace_is_filled_in_when_the_content_is_complete() {
        // 真实会话的四次失败都是这个形状：内容写完、字符串闭合，只差信封的收尾括号
        let raw =
            "{\"type\":\"tool\",\"name\":\"write\",\"args\":{\"path\":\"a\",\"content\":\"内容\"}";
        let out = UnambiguousRepair.repair(raw, &Malformed::Unclosed(tail("}", false)));
        let fixed = out.repaired.expect("只差收尾括号必须能修");
        assert_eq!(fixed, format!("{}}}", raw));
        let again = parse(&fixed);
        let inv = again.tools.into_iter().next().expect("补完就是工具信封");
        assert!(inv.malformed.is_none());
        assert_eq!(inv.name, "write");
        assert!(
            out.what[0].contains("}"),
            "要如实说补了什么：{:?}",
            out.what
        );
        // 同时还有裸换行：两处一起修（真实会话里就是这样）
        let mixed = "{\"type\":\"tool\",\"name\":\"write\",\"args\":{\"path\":\"a\",\"content\":\"一\n二\"}";
        let out2 = UnambiguousRepair.repair(mixed, &raw_control(Some(tail("}", false))));
        let fixed2 = out2.repaired.expect("两处都能修");
        assert!(
            parse(&fixed2).tools.iter().all(|t| t.malformed.is_none()),
            "{}",
            fixed2
        );
        assert_eq!(out2.what.len(), 2, "两处各说一次：{:?}", out2.what);
    }

    #[test]
    fn a_cut_in_the_middle_of_a_string_is_never_patched() {
        // 断在字符串中间 = 内容没写完：补引号会让核心拿到半截内容 → 一律不修
        let cut =
            "{\"type\":\"tool\",\"name\":\"write\",\"args\":{\"path\":\"a\",\"content\":\"写了一半";
        assert!(UnambiguousRepair
            .repair(cut, &Malformed::Unclosed(tail("}\"}", true)))
            .repaired
            .is_none());
        assert!(UnambiguousRepair
            .repair(cut, &raw_control(Some(tail("}", true))))
            .repaired
            .is_none());
        // 本来就不缺收尾字符：没什么可补
        assert!(UnambiguousRepair
            .repair("{\"a\":1}", &Malformed::Unclosed(Tail::default()))
            .repaired
            .is_none());
    }

    #[test]
    fn two_envelopes_in_one_reply_are_never_patched() {
        // 第一段内容写完、只差一个 }，但后面又起了一段：补末尾括号补不到中间那段（真实事故）。
        let raw = "{\"type\":\"tool\",\"name\":\"write\",\"args\":{\"path\":\"a\",\"content\":\"正文\"}\n{\"type\":\"tool\",\"name\":\"write\",\"args\":{\"path\":\"a\",\"content\":\"重写\"}";
        let out = UnambiguousRepair.repair(raw, &Malformed::Unclosed(tail_multi("}}")));
        assert!(
            out.repaired.is_none() && out.what.is_empty(),
            "两段一律不猜：{:?}",
            out.what
        );
    }

    #[test]
    fn other_kinds_are_left_to_the_model() {
        for kind in [Malformed::Syntax("x".into()), Malformed::Shape("y".into())] {
            let out = UnambiguousRepair.repair("{\"type\":\"tool\"", &kind);
            assert!(
                out.repaired.is_none() && out.what.is_empty(),
                "别的类别一律不猜：{:?}",
                kind
            );
        }
        // 声明是控制字符、实际却找不到（诊断与实际不符）：如实说没修
        assert!(UnambiguousRepair
            .repair("{\"a\":1}", &raw_control(None))
            .repaired
            .is_none());
    }
}
