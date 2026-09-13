//! 核心自带的内置工具：read / write / search。
//! 策略在 core（名字固定、放行、寻址、根内校验、回执文案）；机制在 SysIo 端口（适配层）。
//! 存在的理由：读盘落盘不经过任何外部进程，编码问题不进本程序——模型自己看内容自己决定。
//! 路径一律是真实绝对路径（根目录经提示词册如实告知）；模块声明的外部工具与内置工具用同一套路径。

use crate::core::ports::{SysIo, ToolOutcome};
use crate::core::prompt::Prompts;
use crate::core::workspace::{Place, Sandbox};

pub const READ: &str = "read";
pub const WRITE: &str = "write";
pub const SEARCH: &str = "search";

/// 单次读取回传的字符上限（超出如实截断，避免一次读爆上下文）。
pub const MAX_READ_CHARS: usize = 60_000;
/// 单次 search 回传的命中行数上限（超出如实截断；命中总数照实报）。
pub const MAX_SEARCH_HITS: usize = 200;
/// 单条命中行回传的字符上限（长行截断，避免一行吃掉整个上下文）。
pub const MAX_SEARCH_LINE_CHARS: usize = 300;

/// 写进模块目录的标记：回执里带上它，给模型看；工具轨迹据此给用户一句可见提示（同一常量，两处共用）。
pub const MODULE_WRITE_MARK: &str = "[模块目录]";

/// 内置工具名（保留名）。
pub fn is_builtin(name: &str) -> bool {
    name == READ || name == WRITE || name == SEARCH
}

/// 内置工具名清单（拼错误提示用）。
pub fn names() -> Vec<String> {
    vec![READ.to_string(), WRITE.to_string(), SEARCH.to_string()]
}

/// 内置工具说明块：提示词册 sys_tools 渲染（含本 agent 的真实根目录与模块目录）。
pub fn guide(prompts: &Prompts, sb: &Sandbox) -> String {
    let module_roots = if sb.modules.is_empty() {
        prompts.core.no_module_dirs.clone()
    } else {
        sb.modules
            .iter()
            .map(|(id, root)| {
                sb.texts.render(
                    &sb.texts.module_root_line,
                    &[("id", id.clone()), ("root", crate::core::workspace::slash(root))],
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    prompts.render(
        &prompts.core.sys_tools,
        &[
            ("work_name", sb.work_name.clone()),
            ("agent", sb.agent.clone()),
            ("work_root", crate::core::workspace::slash(&sb.shared)),
            ("sandbox_root", crate::core::workspace::slash(&sb.private)),
            ("module_roots", module_roots),
        ],
    )
}

/// 执行一次内置工具调用（args_json = 模型信封里的 args 对象）。
pub fn execute(sb: &Sandbox, io: &dyn SysIo, name: &str, args_json: &str) -> ToolOutcome {
    let texts = &sb.texts;
    let args: serde_json::Value = match serde_json::from_str(args_json) {
        Ok(v) => v,
        Err(e) => return fail(texts.render(&texts.bad_args_json, &[("error", e.to_string())])),
    };
    let spec = match args.get("path").and_then(|p| p.as_str()) {
        Some(p) => p.to_string(),
        None => return fail(texts.missing_path.clone()),
    };
    let (place, path) = match sb.resolve(&spec) {
        Ok(x) => x,
        Err(e) => return fail(e),
    };
    match name {
        READ => {
            let got = match io.read(&path) {
                Ok(g) => g,
                Err(e) => return fail(e),
            };
            let (text, cut) = truncate_chars(&got.text, MAX_READ_CHARS);
            let mut out = texts.render(
                &texts.read_header,
                &[("path", spec.clone()), ("bytes", got.bytes.to_string()), ("text", text)],
            );
            if cut {
                out.push_str(&format!(
                    "\n{}",
                    texts.render(&texts.read_truncated_chars, &[("limit", MAX_READ_CHARS.to_string())])
                ));
            }
            if got.lossy {
                out.push_str(&format!("\n{}", texts.lossy_note));
            }
            if got.cut {
                out.push_str(&format!("\n{}", texts.read_truncated_bytes));
            }
            ToolOutcome { ok: true, output: out }
        }
        WRITE => {
            let content = match args.get("content").and_then(|c| c.as_str()) {
                Some(c) => c,
                None => return fail(texts.missing_content.clone()),
            };
            if let Err(e) = io.write(&path, content) {
                return fail(e);
            }
            let mut out = texts.render(
                &texts.write_header,
                &[("path", spec.clone()), ("chars", content.chars().count().to_string())],
            );
            // 写进模块目录属于「动了自己的能力」：放行，但如实提示（模型可见；工具轨迹据此也给用户一句）。
            if let Place::Module(id) = &place {
                out.push_str(&format!(
                    "\n{}",
                    texts.render(&texts.write_module_note, &[("mark", MODULE_WRITE_MARK.to_string()), ("id", id.clone())])
                ));
            }
            ToolOutcome { ok: true, output: out }
        }
        SEARCH => {
            let keyword = match args.get("keyword").and_then(|k| k.as_str()) {
                Some(k) if !k.is_empty() => k.to_string(),
                _ => return fail(texts.missing_keyword.clone()),
            };
            let ignore_case = args.get("ignore_case").and_then(|v| v.as_bool()).unwrap_or(false);
            let got = match io.read(&path) {
                Ok(g) => g,
                Err(e) => return fail(e),
            };
            let needle = if ignore_case { keyword.to_lowercase() } else { keyword.clone() };
            let mut hits: Vec<(usize, String)> = Vec::new();
            let mut matched = 0usize;
            let mut total = 0usize;
            for (i, line) in got.text.lines().enumerate() {
                total += 1;
                let hay = if ignore_case { line.to_lowercase() } else { line.to_string() };
                if hay.contains(&needle) {
                    matched += 1;
                    if hits.len() < MAX_SEARCH_HITS {
                        hits.push((i + 1, truncate_chars(line, MAX_SEARCH_LINE_CHARS).0));
                    }
                }
            }
            let mode = if ignore_case { texts.search_mode_insensitive.clone() } else { texts.search_mode_sensitive.clone() };
            let mut out = texts.render(
                &texts.search_header,
                &[("path", spec.clone()), ("keyword", keyword.clone()), ("mode", mode)],
            );
            if hits.is_empty() {
                out.push_str(&format!("\n{}", texts.search_no_hits));
            }
            for (n, line) in &hits {
                out.push_str(&format!(
                    "\n{}",
                    texts.render(&texts.search_hit_line, &[("n", n.to_string()), ("line", line.clone())])
                ));
            }
            out.push_str(&format!(
                "\n{}",
                texts.render(&texts.search_summary, &[("hits", matched.to_string()), ("total", total.to_string())])
            ));
            if matched > hits.len() {
                out.push_str(&format!(
                    "\n{}",
                    texts.render(&texts.search_truncated, &[("limit", MAX_SEARCH_HITS.to_string())])
                ));
            }
            if got.lossy {
                out.push_str(&format!("\n{}", texts.lossy_note));
            }
            if got.cut {
                out.push_str(&format!("\n{}", texts.search_truncated_bytes));
            }
            ToolOutcome { ok: true, output: out }
        }
        other => fail(texts.render(&texts.unknown_builtin, &[("name", other.to_string())])),
    }
}

fn fail(msg: String) -> ToolOutcome {
    ToolOutcome { ok: false, output: msg }
}

/// 按字符截断（不劈开 UTF-8）；返回（文本, 是否截断）。
fn truncate_chars(s: &str, max: usize) -> (String, bool) {
    if s.chars().count() <= max {
        return (s.to_string(), false);
    }
    (s.chars().take(max).collect(), true)
}
