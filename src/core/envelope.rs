//! 发言信封：讨论/执行回复的最外层结构。
//! 信封是唯一的机器锚；信封里的 text 完全自由。
//! tool 信封是模块申请使用工具的唯一通道；解析失败不猜测：按 say 呈现原文（转录即内容），并向用户如实注明。

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Say,
    Ask,
    Leave,
    Agree,
    /// 模块申请调用工具（联动引擎的工具循环；本形态无工具时按原文收录）。
    Tool,
}

/// 一次工具调用申请：name 必须在模块清单的工具表内；args 原样转给执行端口。
#[derive(Debug, Clone)]
pub struct ToolInvoke {
    pub name: String,
    /// 规范化后的 JSON 参数文本（经执行端口送入工具 stdin）。
    pub args_json: String,
}

#[derive(Debug, Clone)]
pub struct Reply {
    pub verb: Verb,
    pub text: String,
    /// 信封解析是否干净；不干净时 text = 原始输出（照进转录，不丢字）。
    pub degraded: bool,
    /// verb = Tool 时的调用申请；其余动词恒为 None。
    pub tool: Option<ToolInvoke>,
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(rename = "type")]
    verb: String,
    text: String,
}

#[derive(Deserialize)]
struct ToolEnvelope {
    #[serde(rename = "type")]
    kind: String,
    name: String,
    #[serde(default)]
    args: serde_json::Value,
}

/// 解析模型输出为信封。尽力提取 JSON 对象，失败则降级为 say(原文)。
pub fn parse(raw: &str) -> Reply {
    if let Some(obj) = extract_json_object(raw) {
        // tool 信封优先：name 缺失即视为不合法，落回普通信封解析（不猜测）。
        if let Ok(t) = serde_json::from_str::<ToolEnvelope>(&obj) {
            if t.kind == "tool" {
                return Reply {
                    verb: Verb::Tool,
                    text: raw.trim().to_string(),
                    degraded: false,
                    tool: Some(ToolInvoke { name: t.name, args_json: t.args.to_string() }),
                };
            }
        }
        if let Ok(env) = serde_json::from_str::<Envelope>(&obj) {
            let verb = match env.verb.as_str() {
                "ask" => Verb::Ask,
                "leave" => Verb::Leave,
                "agree" => Verb::Agree,
                _ => Verb::Say,
            };
            return Reply { verb, text: env.text, degraded: false, tool: None };
        }
    }
    Reply { verb: Verb::Say, text: raw.trim().to_string(), degraded: true, tool: None }
}

/// 提取首个平衡的 JSON 数组（验收清单用），同一扫描器，换括号。
pub fn extract_json_array(s: &str) -> Option<String> {
    extract_balanced(s, '[', ']')
}

/// 提取首个平衡的 JSON 对象（模型常在 JSON 外包裹说明文字）。
pub fn extract_json_object(s: &str) -> Option<String> {
    extract_balanced(s, '{', '}')
}

fn extract_balanced(s: &str, open: char, close: char) -> Option<String> {
    let bytes = s.as_bytes();
    let start = s.find(open)?;
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
            c if c == open as u8 => depth += 1,
            c if c == close as u8 => {
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
