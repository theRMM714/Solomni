//! 核心自带的内置工具：read / write / search。
//! 策略在 core（名字固定、放行、寻址、根内校验、参数校验、回执文案）；机制在 SysIo 端口（适配层）。
//! **参数契约不在本文件里**：声明在 prompts.yaml 的 builtin_tools，本文件只按声明校验、取缺省值与拼回执。
//! 存在的理由：读盘落盘不经过任何外部进程，编码问题不进本程序——模型自己看内容自己决定。
//! 路径一律是真实绝对路径（根目录经提示词册如实告知）；模块声明的外部工具与内置工具用同一套路径。

use crate::core::ports::{SysIo, ToolOutcome};
use crate::core::prompt::{Prompts, ToolTexts};
use crate::core::schema::{ArgFault, ToolSchema};
use crate::core::workspace::{Place, Sandbox};
use std::path::Path;

pub const READ: &str = "read";
pub const WRITE: &str = "write";
pub const SEARCH: &str = "search";

/// 单次读取回传的字符上限（超出如实截断，并在回执里给出接着读的 offset）。
pub const MAX_READ_CHARS: usize = 60_000;
/// 回执里单行的字符上限（长行截断，避免一行吃掉整个上下文）。
pub const MAX_READ_LINE_CHARS: usize = 2_000;
/// 单次 search 回传的命中行数上限（超出如实截断；命中总数照实报）。
pub const MAX_SEARCH_HITS: usize = 200;
/// 单条命中行回传的字符上限（长行截断，避免一行吃掉整个上下文）。
pub const MAX_SEARCH_LINE_CHARS: usize = 300;

/// 写进模块目录的标记：回执里带上它，给模型看；工具轨迹据此给用户一句可见提示（同一常量，两处共用）。
pub const MODULE_WRITE_MARK: &str = "[模块目录]";

/// 内置工具名（保留名）。
/// 与 prompts.yaml 的 builtin_tools 是同一份名单，测试「builtin_names_match_book」锁死两者一致。
pub fn is_builtin(name: &str) -> bool {
    name == READ || name == WRITE || name == SEARCH
}

/// 内置工具名清单（拼错误提示用）。
pub fn names() -> Vec<String> {
    vec![READ.to_string(), WRITE.to_string(), SEARCH.to_string()]
}

/// 内置工具说明块：提示词册 sys_tools 渲染（含本 agent 的真实根目录、模块目录与工具参数）。
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
            ("tool_params", crate::core::schema::render_book(&sb.builtin_tools)),
        ],
    )
}

/// 执行一次内置工具调用（args_json = 模型信封里的 args 对象）。
/// 顺序：解析 JSON → 认工具 → 按声明校验参数 → 补缺省 → 寻址（内置工具一律需要一个 path）。
pub fn execute(sb: &Sandbox, io: &dyn SysIo, name: &str, args_json: &str) -> ToolOutcome {
    let texts = &sb.texts;
    let mut args: serde_json::Value = match serde_json::from_str(args_json) {
        Ok(v) => v,
        Err(e) => return fail(texts.render(&texts.bad_args_json, &[("error", e.to_string())])),
    };
    let Some(schema) = sb.builtin_tools.get(name) else {
        return fail(texts.render(&texts.unknown_builtin, &[("name", name.to_string())]));
    };
    if let Err(fault) = schema.check(&args) {
        return fail(arg_fault_text(texts, name, schema, &fault));
    }
    schema.apply_defaults(&mut args);
    let spec = match args.get("path").and_then(|p| p.as_str()) {
        Some(p) => p.to_string(),
        // 声明里每个内置工具都声明了必填 path，走到这里说明声明与实现不一致。
        None => return fail(texts.render(&texts.arg_missing, &[("name", "path".to_string())])),
    };
    let (place, path) = match sb.resolve(&spec) {
        Ok(x) => x,
        Err(e) => return fail(e),
    };
    match name {
        READ => read(sb, io, &args, &spec, &path),
        WRITE => write(sb, io, &args, &spec, &path, &place),
        SEARCH => search(sb, io, &args, &spec, &path),
        other => fail(sb.texts.render(&sb.texts.unknown_builtin, &[("name", other.to_string())])),
    }
}

/// read：按行区间返回，行号从 1 数起；回执末尾告诉模型接着用哪个 offset。
fn read(sb: &Sandbox, io: &dyn SysIo, args: &serde_json::Value, spec: &str, path: &Path) -> ToolOutcome {
    let texts = &sb.texts;
    let got = match io.read(path) {
        Ok(g) => g,
        Err(e) => return fail(e),
    };
    let header = |body: String| {
        texts.render(
            &texts.read_header,
            &[("path", spec.to_string()), ("bytes", got.bytes.to_string()), ("text", body)],
        )
    };
    let lines: Vec<&str> = got.text.lines().collect();
    let total = lines.len();
    let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(total as u64).max(1) as usize;
    if offset > total {
        let note = texts.render(&texts.read_past_end, &[("total", total.to_string())]);
        return ToolOutcome { ok: true, output: header(note) };
    }
    let mut body: Vec<String> = Vec::new();
    let mut used = 0usize;
    let mut to = offset - 1;
    for (i, line) in lines.iter().enumerate().skip(offset - 1).take(limit) {
        let (text, cut) = truncate_chars(line, MAX_READ_LINE_CHARS);
        let mut row = texts.render(&texts.read_line, &[("n", (i + 1).to_string()), ("line", text)]);
        if cut {
            row.push_str(&texts.render(&texts.read_line_capped, &[("limit", MAX_READ_LINE_CHARS.to_string())]));
        }
        // 字符预算用完就停：至少给出第一行，绝不空手而归。
        let cost = row.chars().count() + 1;
        if !body.is_empty() && used + cost > MAX_READ_CHARS {
            break;
        }
        used += cost;
        to = i + 1;
        body.push(row);
    }
    let mut out = header(body.join("\n"));
    let tail = if got.cut {
        // 机制层只读了开头：总行数不可知，也不能声称"到文件末尾"。
        texts.render(&texts.read_more_cut, &[("from", offset.to_string()), ("to", to.to_string())])
    } else if to < total {
        texts.render(
            &texts.read_more,
            &[
                ("from", offset.to_string()),
                ("to", to.to_string()),
                ("total", total.to_string()),
                ("next", (to + 1).to_string()),
            ],
        )
    } else {
        texts.render(&texts.read_end, &[("total", total.to_string())])
    };
    out.push_str(&format!("\n{}", tail));
    if got.lossy {
        out.push_str(&format!("\n{}", texts.lossy_note));
    }
    ToolOutcome { ok: true, output: out }
}

/// write：整份写入（同名文件被整份覆盖）。
fn write(sb: &Sandbox, io: &dyn SysIo, args: &serde_json::Value, spec: &str, path: &Path, place: &Place) -> ToolOutcome {
    let texts = &sb.texts;
    let content = args.get("content").and_then(|c| c.as_str()).unwrap_or_default();
    if let Err(e) = io.write(path, content) {
        return fail(e);
    }
    let mut out = texts.render(
        &texts.write_header,
        &[("path", spec.to_string()), ("chars", content.chars().count().to_string())],
    );
    // 写进模块目录属于「动了自己的能力」：放行，但如实提示（模型可见；工具轨迹据此也给用户一句）。
    if let Place::Module(id) = place {
        out.push_str(&format!(
            "\n{}",
            texts.render(&texts.write_module_note, &[("mark", MODULE_WRITE_MARK.to_string()), ("id", id.clone())])
        ));
    }
    ToolOutcome { ok: true, output: out }
}

/// search：逐行找关键词，返回带行号的命中行（命中总数与截断如实报）。
fn search(sb: &Sandbox, io: &dyn SysIo, args: &serde_json::Value, spec: &str, path: &Path) -> ToolOutcome {
    let texts = &sb.texts;
    let keyword = args.get("keyword").and_then(|k| k.as_str()).unwrap_or_default().to_string();
    let ignore_case = args.get("ignore_case").and_then(|v| v.as_bool()).unwrap_or(false);
    let got = match io.read(path) {
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
        &[("path", spec.to_string()), ("keyword", keyword.clone()), ("mode", mode)],
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

/// 参数不符的回执：说清是哪一条不合（由声明判定），再把工具签名原样发回去。
pub fn arg_fault_text(texts: &ToolTexts, name: &str, schema: &ToolSchema, fault: &ArgFault) -> String {
    let why = match fault {
        ArgFault::NotObject => texts.arg_not_object.clone(),
        ArgFault::Missing(n) => texts.render(&texts.arg_missing, &[("name", n.clone())]),
        ArgFault::WrongType { name, want } => texts.render(
            &texts.arg_wrong_type,
            &[("name", name.clone()), ("want", want.to_string())],
        ),
        ArgFault::Empty(n) => texts.render(&texts.arg_empty, &[("name", n.clone())]),
        ArgFault::TooSmall { name, min } => texts.render(
            &texts.arg_too_small,
            &[("name", name.clone()), ("min", min.clone())],
        ),
        ArgFault::TooBig { name, max } => texts.render(
            &texts.arg_too_big,
            &[("name", name.clone()), ("max", max.clone())],
        ),
        ArgFault::Unknown(n) => texts.render(&texts.arg_unknown, &[("name", n.clone())]),
    };
    texts.render(
        &texts.arg_fault,
        &[("why", why), ("signature", format!("{}\n{}", name, schema.render_for_prompt()))],
    )
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
