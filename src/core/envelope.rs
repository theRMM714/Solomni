//! 发言信封：讨论/执行回复的最外层结构。
//! 信封是唯一的机器锚；信封里的 text 完全自由。
//! tool 信封是模块申请使用工具的唯一通道；解析失败不猜测：按 say 呈现原文（转录即内容），并向用户如实注明。
//! 例外：**看起来想发工具信封但 JSON 非法**时单独标记 malformed（既不执行工具，也不把 JSON 当正文渲染），
//! 由引擎记一条失败的工具行把"信封不合法"回注给模型，让它下一轮自己改。

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

/// 一次工具调用申请：module 指明工具属于哪个模块（多模块 agent 靠它消歧；内置工具与省略时为 None）。
#[derive(Debug, Clone)]
pub struct ToolInvoke {
    /// true = 输出看起来是工具信封但 JSON 非法：**绝不据此执行工具**，只记一条失败的工具行。
    /// name / module 此时是"尽力打捞"的结果（可能为空），仅用于显示与日志。
    pub malformed: bool,
    /// 工具所属模块 id（trim 后非空才 Some）。
    pub module: Option<String>,
    pub name: String,
    /// 普通调用 = 规范化后的参数 JSON（经执行端口送入工具 stdin）；malformed = 提取到的对象或原文开头。
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
    /// 可省略：缺省为空串（leave / agree 常只表态不留言）。
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
struct ToolEnvelope {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    module: Option<String>,
    name: String,
    #[serde(default)]
    args: serde_json::Value,
}

/// 从原文里去掉被提取出的那段 JSON（只去第一次出现的位置），剩下的就是信封之外的正文。
fn strip_once(raw: &str, obj: &str) -> String {
    match raw.find(obj) {
        Some(i) => format!("{}{}", &raw[..i], &raw[i + obj.len()..]),
        None => raw.to_string(),
    }
}

/// 解析模型输出为信封。尽力提取 JSON 对象，失败则降级为 say(原文)。
pub fn parse(raw: &str) -> Reply {
    if let Some(obj) = extract_json_object(raw) {
        // tool 信封优先：name 缺失即视为不合法，落回普通信封解析（不猜测）。
        if let Ok(t) = serde_json::from_str::<ToolEnvelope>(&obj) {
            if t.kind == "tool" {
                return Reply {
                    verb: Verb::Tool,
                    // text = 信封之外的那段正文（模型常在同一轮里先写一句再发信封）；只剩信封时为空串。
                    // 信封 JSON 永不进 text：界面因此不会把 JSON 糊上屏，view.raw 另存完整原文供重建。
                    text: strip_once(raw, &obj).trim().to_string(),
                    degraded: false,
                    tool: Some(ToolInvoke {
                        malformed: false,
                        module: t.module.map(|m| m.trim().to_string()).filter(|m| !m.is_empty()),
                        name: t.name,
                        args_json: t.args.to_string(),
                    }),
                };
            }
        }
        if let Ok(env) = serde_json::from_str::<Envelope>(&obj) {
            // type = tool 却走到这里 = ToolEnvelope 已判不合法（缺 name 等）：这是 malformed 信号，
            // 不能当普通信封收（否则缺 name 的工具信封会被静默当成发言）。
            if env.verb != "tool" {
                let verb = match env.verb.as_str() {
                    "ask" => Verb::Ask,
                    "leave" => Verb::Leave,
                    "agree" => Verb::Agree,
                    _ => Verb::Say,
                };
                return Reply { verb, text: env.text, degraded: false, tool: None };
            }
        }
    }
    // 像工具信封但 JSON 非法：给独立信号（degraded 是"信封缺失按发言收录"，语义不同）。
    // 两条互补口径，都不误判"正文里引用一个**完整**对象"：
    // ①**收尾未闭合**：扫到最后 depth > 0 → 取最后一个 0→1 的起点到结尾那段（即被截断的坏信封），
    //   它含 "type":"tool" 就判 malformed，text = 起点之前的正文（所以坏 JSON 绝不上屏）。
    //   这样"正文在前、坏信封在后"也能覆盖。
    // ②**以 JSON 对象为主体**（第一个非空白字符是 '{'）且该对象含 "type":"tool"：
    //   覆盖"平衡但字段不合法"（如缺 name）的输出。平衡的完整对象被正文引用时落两条之外，
    //   仍按发言收录（这就是反误判的那一半）。
    if let Some(start) = unclosed_start(raw) {
        let frag = &raw[start..];
        if mentions_tool_type(frag) {
            return Reply {
                verb: Verb::Tool,
                text: raw[..start].trim().to_string(),
                degraded: false,
                tool: Some(ToolInvoke {
                    malformed: true,
                    module: Some(salvage(frag, "module")).filter(|m| !m.is_empty()),
                    name: salvage(frag, "name"),
                    args_json: head_chars(frag, 200),
                }),
            };
        }
    }
    if raw.trim_start().starts_with('{') {
        let obj = extract_json_object(raw);
        let probe = obj.as_deref().unwrap_or(raw);
        if mentions_tool_type(probe) {
            // 正文 = 信封之外的那段（JSON 永不进 text）；括号不平衡时整段都算信封。
            let text = match &obj {
                Some(o) => strip_once(raw, o),
                None => raw[..raw.find('{').unwrap_or(0)].to_string(),
            };
            return Reply {
                verb: Verb::Tool,
                text: text.trim().to_string(),
                degraded: false,
                tool: Some(ToolInvoke {
                    malformed: true,
                    module: Some(salvage(probe, "module")).filter(|m| !m.is_empty()),
                    name: salvage(probe, "name"),
                    args_json: obj.clone().unwrap_or_else(|| head_chars(raw, 200)),
                }),
            };
        }
    }
    Reply { verb: Verb::Say, text: raw.trim().to_string(), degraded: true, tool: None }
}

/// 扫一遍输出（沿用 extract_balanced 的"字符串内不计数"规则），
/// 仅当**扫到结尾仍未闭合**时，返回最后一个「depth 从 0 变 1」的起点（即被截断的坏信封的开头）。
/// 平衡的完整对象会回到 depth 0，于是返回 None——这正是"正文里引用完整对象不误判"的依据。
fn unclosed_start(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0usize;
    let mut in_str = false;
    let mut esc = false;
    let mut last_open: Option<usize> = None;
    for (i, &b) in bytes.iter().enumerate() {
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => {
                if depth == 0 {
                    last_open = Some(i);
                }
                depth += 1;
            }
            b'}' => {
                if depth > 0 {
                    depth -= 1;
                    if depth == 0 {
                        last_open = None;
                    }
                }
            }
            _ => {}
        }
    }
    if depth > 0 {
        last_open
    } else {
        None
    }
}

/// 文本里是否出现 "type" : "tool"（允许冒号前后空白；大小写按原样匹配）。
fn mentions_tool_type(text: &str) -> bool {
    let mut from = 0;
    while let Some(i) = text[from..].find("\"type\"") {
        let at = from + i + 6;
        let rest = text[at..].trim_start();
        if let Some(rest) = rest.strip_prefix(':') {
            if rest.trim_start().starts_with("\"tool\"") {
                return true;
            }
        }
        from = at;
        if from >= text.len() {
            break;
        }
    }
    false
}

/// 从一段（可能非法的）JSON 文本里尽力打捞一个字符串字段；打捞不到就空串。**只用于显示与日志**。
fn salvage(text: &str, key: &str) -> String {
    let pat = format!("\"{}\"", key);
    let mut from = 0;
    while let Some(i) = text[from..].find(&pat) {
        let at = from + i + pat.len();
        let rest = text[at..].trim_start();
        if let Some(rest) = rest.strip_prefix(':') {
            let rest = rest.trim_start();
            if let Some(rest) = rest.strip_prefix('"') {
                if let Some(end) = rest.find('"') {
                    return rest[..end].to_string();
                }
            }
        }
        from = at;
        if from >= text.len() {
            break;
        }
    }
    String::new()
}

/// 取开头 n 个字符（不劈开 UTF-8）；超出加省略号。
fn head_chars(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n).collect();
    out.push('…');
    out
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
