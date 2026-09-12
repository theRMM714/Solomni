//! 发言信封：讨论/执行回复的最外层结构。
//! 信封是唯一的机器锚；信封里的 text 完全自由。
//! 解析失败不猜测：按 say 呈现原文（转录即内容），并向用户如实注明。

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Say,
    Ask,
    Leave,
    Agree,
}

#[derive(Debug, Clone)]
pub struct Reply {
    pub verb: Verb,
    pub text: String,
    /// 信封解析是否干净；不干净时 text = 原始输出（照进转录，不丢字）。
    pub degraded: bool,
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(rename = "type")]
    verb: String,
    text: String,
}

/// 解析模型输出为信封。尽力提取 JSON 对象，失败则降级为 say(原文)。
pub fn parse(raw: &str) -> Reply {
    if let Some(obj) = extract_json_object(raw) {
        if let Ok(env) = serde_json::from_str::<Envelope>(&obj) {
            let verb = match env.verb.as_str() {
                "ask" => Verb::Ask,
                "leave" => Verb::Leave,
                "agree" => Verb::Agree,
                _ => Verb::Say,
            };
            return Reply { verb, text: env.text, degraded: false };
        }
    }
    Reply { verb: Verb::Say, text: raw.trim().to_string(), degraded: true }
}

/// 提取首个平衡的 JSON 对象（模型常在 JSON 外包裹说明文字）。
fn extract_json_object(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let start = s.find('{')?;
    let mut depth = 0usize;
    let mut in_str = false;
    let mut esc = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if esc { esc = false; }
            else if b == b'\\' { esc = true; }
            else if b == b'"' { in_str = false; }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(s[start..=i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}
