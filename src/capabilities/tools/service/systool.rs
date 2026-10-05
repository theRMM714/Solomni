//! 内置工具的**执行编排**：驱动 `SysIo` 读写盘上的文件（read / write / edit / patch / list / search）。
//!
//! 纯规则在 `domain/systool.rs`（参数校验、回执文案、观察账本、`ToolOutcome`）——本文件只调用它们；
//! 因此这里引 `ports`（IO 机制），`domain/` 不引（见 ARCHITECTURE.md §九.3）。

use crate::capabilities::tools::domain::systool::*;
use crate::capabilities::tools::ports::SysIo;
use crate::capabilities::workspace::api::{Place, Sandbox};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
/// 执行一次内置工具调用（args_json = 模型信封里的 args 对象）。
/// 顺序：解析 JSON → 认工具 → 按声明校验参数 → 补缺省 → 寻址（内置工具一律需要一个 path）。
pub fn execute(
    sb: &Sandbox,
    book: &crate::capabilities::tools::api::ToolBook,
    io: &dyn SysIo,
    obs: &mut Observations,
    name: &str,
    args_json: &str,
) -> ToolOutcome {
    let texts = &sb.texts;
    // 自由格式工具：输入是一段原样文本（不是 JSON），也不吃参数校验——认工具后就交给它自己解释。
    if is_freeform(name) {
        if !book.contains_key(name) {
            return fail(texts.render(&texts.unknown_builtin, &[("name", name.to_string())]));
        }
        return match name {
            PATCH => apply_patch(sb, io, obs, args_json),
            other => fail(texts.render(&texts.unknown_builtin, &[("name", other.to_string())])),
        };
    }
    let mut args: serde_json::Value = match serde_json::from_str(args_json) {
        Ok(v) => v,
        Err(e) => return fail(texts.render(&texts.bad_args_json, &[("error", e.to_string())])),
    };
    let Some(schema) = book.get(name) else {
        return fail(texts.render(&texts.unknown_builtin, &[("name", name.to_string())]));
    };
    if let Err(fault) = schema.check(&args) {
        return fail(arg_fault_text(texts, name, schema, &fault));
    }
    schema.apply_defaults(&mut args);
    // 回报工具：不碰文件，回执就是把回报原样带出来（节点产出由它承载）。
    if name == REPORT {
        let get = |k: &str| {
            args.get(k)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        let (s, c, o) = (get("summary"), get("changes"), get("open"));
        let mut out = format!("summary：{}\nchanges：{}", s, c);
        if !o.trim().is_empty() {
            out.push_str(&format!("\nopen：{}", o));
        }
        return ToolOutcome {
            ok: true,
            output: out,
        };
    }
    let spec = match args.get("path").and_then(|p| p.as_str()) {
        Some(p) => p.to_string(),
        // 声明里每个内置工具都声明了必填 path，走到这里说明声明与实现不一致。
        None => return fail(texts.render(&texts.arg_missing, &[("name", "path".to_string())])),
    };
    let (place, path) = match sb.resolve(&spec) {
        Ok(x) => x,
        Err(e) => return fail(e),
    };
    // 写类工具先过写权限（共享主副本是否可写 + 路径白黑名单 + 模块目录只读）；
    // 读类工具过读权限（只对共享主副本生效；私有沙箱与模块目录照旧可读）。
    if matches!(name, WRITE | EDIT) {
        if !sb.can_write(&place, &path) {
            return fail(sb.write_refusal(&place, &path));
        }
    } else if matches!(name, READ | LIST | SEARCH) && !sb.can_read(&place, &path) {
        return fail(sb.read_refusal(&place, &path));
    }
    match name {
        READ => read(sb, io, obs, &args, &spec, &path),
        WRITE => write(sb, io, obs, &args, &spec, &path, &place),
        EDIT => edit(sb, io, obs, &args, &spec, &path),
        LIST => list(sb, io, &spec, &path),
        SEARCH => search(sb, io, &args, &spec, &path),
        other => fail(
            sb.texts
                .render(&sb.texts.unknown_builtin, &[("name", other.to_string())]),
        ),
    }
}

/// list：列目录（名字 / 是否目录 / 字节数，按名字排序）。
/// 存在的理由：确认"资料齐不齐、脚本在不在、运行包装没装"必须能列目录——read 只读文件。
fn list(sb: &Sandbox, io: &dyn SysIo, spec: &str, path: &Path) -> ToolOutcome {
    let texts = &sb.texts;
    let entries = match io.list(path) {
        Ok(e) => e,
        Err(e) => return fail(e),
    };
    let rows = if entries.is_empty() {
        texts.list_empty.clone()
    } else {
        entries
            .iter()
            .map(|e| {
                texts.render(
                    &texts.list_row,
                    &[
                        ("name", e.name.clone()),
                        (
                            "dir_mark",
                            if e.is_dir {
                                texts.list_dir_mark.clone()
                            } else {
                                String::new()
                            },
                        ),
                        ("bytes", e.bytes.to_string()),
                    ],
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let header = texts.render(
        &texts.list_header,
        &[
            ("path", spec.to_string()),
            ("count", entries.len().to_string()),
        ],
    );
    ToolOutcome {
        ok: true,
        output: format!("{}\n{}", header, rows),
    }
}

/// read：按行区间返回，行号从 1 数起；回执末尾告诉模型接着用哪个 offset，并如实记账"读到的是不是全文"。
fn read(
    sb: &Sandbox,
    io: &dyn SysIo,
    obs: &mut Observations,
    args: &serde_json::Value,
    spec: &str,
    path: &Path,
) -> ToolOutcome {
    let texts = &sb.texts;
    let got = match io.read(path) {
        Ok(g) => g,
        // 目录不是文件：如实引导到 list，而不是把 IO 错原样丢给模型。
        Err(e) => match io.list(path) {
            Ok(_) => return fail(texts.render(&texts.read_is_dir, &[("path", spec.to_string())])),
            Err(_) => return fail(e),
        },
    };
    let header = |body: String| {
        texts.render(
            &texts.read_header,
            &[
                ("path", spec.to_string()),
                ("bytes", got.bytes.to_string()),
                ("text", body),
            ],
        )
    };
    let lines: Vec<&str> = got.text.lines().collect();
    let total = lines.len();
    let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(total as u64)
        .max(1) as usize;
    let hash = fnv1a(&got.text);
    if offset > total {
        obs.note_read(path, false, texts.read_partial_none.clone(), hash);
        let note = texts.render(&texts.read_past_end, &[("total", total.to_string())]);
        return ToolOutcome {
            ok: true,
            output: header(note),
        };
    }
    let mut body: Vec<String> = Vec::new();
    let mut used = 0usize;
    let mut to = offset - 1;
    for (i, line) in lines.iter().enumerate().skip(offset - 1).take(limit) {
        let (text, cut) = truncate_chars(line, MAX_READ_LINE_CHARS);
        let mut row = texts.render(
            &texts.read_line,
            &[("n", (i + 1).to_string()), ("line", text)],
        );
        if cut {
            row.push_str(&texts.render(
                &texts.read_line_capped,
                &[("limit", MAX_READ_LINE_CHARS.to_string())],
            ));
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
    // 记账：只有"从头到尾、没有截断、没有编码损失"的一次读取才算见过全文。
    let complete = !got.cut && !got.lossy && offset == 1 && to == total;
    let why = if got.lossy {
        texts.read_partial_lossy.clone()
    } else if got.cut {
        texts.read_partial_cut.clone()
    } else {
        texts.render(
            &texts.read_partial_range,
            &[
                ("from", offset.to_string()),
                ("to", to.to_string()),
                ("total", total.to_string()),
            ],
        )
    };
    obs.note_read(path, complete, why, hash);
    let mut out = header(body.join("\n"));
    let tail = if got.cut {
        // 机制层只读了开头：总行数不可知，也不能声称"到文件末尾"。
        texts.render(
            &texts.read_more_cut,
            &[("from", offset.to_string()), ("to", to.to_string())],
        )
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
    ToolOutcome {
        ok: true,
        output: out,
    }
}

/// write：整份写入（同名文件被整份覆盖）。
/// 覆盖已存在的文件必须先有"完整读过、且读后没被改过"的证据——否则拒绝，并让模型改用 edit。
fn write(
    sb: &Sandbox,
    io: &dyn SysIo,
    obs: &mut Observations,
    args: &serde_json::Value,
    spec: &str,
    path: &Path,
    place: &Place,
) -> ToolOutcome {
    let texts = &sb.texts;
    let content = args
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or_default();
    match io.read(path) {
        // 读不到 = 还没有这个文件（新建不需要读过什么）；机制层的真实错误由下面的 write 如实报出。
        Err(_) => {}
        Ok(got) => {
            if let Err(why) = overwrite_check(texts, obs, spec, path, &got.text) {
                return fail(why);
            }
        }
    }
    if let Err(e) = io.write(path, content) {
        return fail(e);
    }
    obs.note_written(path, fnv1a(content));
    let mut out = texts.render(
        &texts.write_header,
        &[
            ("path", spec.to_string()),
            ("chars", content.chars().count().to_string()),
        ],
    );
    // 写进模块目录属于「动了自己的能力」：放行，但如实提示（模型可见；工具轨迹据此也给用户一句）。
    if let Place::Module(id) = place {
        out.push_str(&format!(
            "\n{}",
            texts.render(
                &texts.write_module_note,
                &[("mark", MODULE_WRITE_MARK.to_string()), ("id", id.clone())]
            )
        ));
    }
    ToolOutcome {
        ok: true,
        output: out,
    }
}

/// edit：按字面替换一处（默认要求唯一命中）。证据是 old_string 本身——它就是要改的那段原文，不需要先读过。
/// 被截断或含非法 UTF-8 的文件一律不改（改写会把没读到的部分或原始字节一起弄丢）。
fn edit(
    sb: &Sandbox,
    io: &dyn SysIo,
    obs: &mut Observations,
    args: &serde_json::Value,
    spec: &str,
    path: &Path,
) -> ToolOutcome {
    let texts = &sb.texts;
    let old = args
        .get("old_string")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let new = args
        .get("new_string")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let all = args
        .get("replace_all")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if old == new {
        return fail(texts.edit_same.clone());
    }
    let got = match io.read(path) {
        Ok(g) => g,
        Err(e) => return fail(e),
    };
    if got.cut {
        return fail(texts.render(&texts.edit_file_cut, &[("path", spec.to_string())]));
    }
    if got.lossy {
        return fail(texts.render(&texts.edit_file_lossy, &[("path", spec.to_string())]));
    }
    let hay = got.text;
    let hits = find_all(&hay, old);
    if hits.is_empty() {
        let mut why = texts.render(
            &texts.edit_no_match,
            &[("total", hay.lines().count().to_string())],
        );
        if let Some((line, actual)) = near_match(&hay, old) {
            why.push('\n');
            why.push_str(&texts.render(
                &texts.edit_no_match_near,
                &[("line", line.to_string()), ("actual", actual)],
            ));
        }
        return fail(why);
    }
    if hits.len() > 1 && !all {
        let lines: Vec<String> = hits.iter().map(|i| line_of(&hay, *i).to_string()).collect();
        return fail(texts.render(
            &texts.edit_multi,
            &[("n", hits.len().to_string()), ("lines", lines.join("、"))],
        ));
    }
    let (out, n) = if all {
        (hay.replace(old, new), hits.len())
    } else {
        let at = hits[0];
        (
            format!("{}{}{}", &hay[..at], new, &hay[at + old.len()..]),
            1,
        )
    };
    if let Err(e) = io.write(path, &out) {
        return fail(e);
    }
    obs.note_written(path, fnv1a(&out));
    ToolOutcome {
        ok: true,
        output: texts.render(
            &texts.edit_header,
            &[
                ("path", spec.to_string()),
                ("n", n.to_string()),
                ("lines", out.lines().count().to_string()),
            ],
        ),
    }
}

/// search：逐行找关键词，返回带行号的命中行（命中总数与截断如实报）。
fn search(
    sb: &Sandbox,
    io: &dyn SysIo,
    args: &serde_json::Value,
    spec: &str,
    path: &Path,
) -> ToolOutcome {
    let texts = &sb.texts;
    let keyword = args
        .get("keyword")
        .and_then(|k| k.as_str())
        .unwrap_or_default()
        .to_string();
    let ignore_case = args
        .get("ignore_case")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let got = match io.read(path) {
        Ok(g) => g,
        Err(e) => return fail(e),
    };
    let needle = if ignore_case {
        keyword.to_lowercase()
    } else {
        keyword.clone()
    };
    let mut hits: Vec<(usize, String)> = Vec::new();
    let mut matched = 0usize;
    let mut total = 0usize;
    for (i, line) in got.text.lines().enumerate() {
        total += 1;
        let hay = if ignore_case {
            line.to_lowercase()
        } else {
            line.to_string()
        };
        if hay.contains(&needle) {
            matched += 1;
            if hits.len() < MAX_SEARCH_HITS {
                hits.push((i + 1, truncate_chars(line, MAX_SEARCH_LINE_CHARS).0));
            }
        }
    }
    let mode = if ignore_case {
        texts.search_mode_insensitive.clone()
    } else {
        texts.search_mode_sensitive.clone()
    };
    let mut out = texts.render(
        &texts.search_header,
        &[
            ("path", spec.to_string()),
            ("keyword", keyword.clone()),
            ("mode", mode),
        ],
    );
    if hits.is_empty() {
        out.push_str(&format!("\n{}", texts.search_no_hits));
    }
    for (n, line) in &hits {
        out.push_str(&format!(
            "\n{}",
            texts.render(
                &texts.search_hit_line,
                &[("n", n.to_string()), ("line", line.clone())]
            )
        ));
    }
    out.push_str(&format!(
        "\n{}",
        texts.render(
            &texts.search_summary,
            &[("hits", matched.to_string()), ("total", total.to_string())]
        )
    ));
    if matched > hits.len() {
        out.push_str(&format!(
            "\n{}",
            texts.render(
                &texts.search_truncated,
                &[("limit", MAX_SEARCH_HITS.to_string())]
            )
        ));
    }
    if got.lossy {
        out.push_str(&format!("\n{}", texts.lossy_note));
    }
    if got.cut {
        out.push_str(&format!("\n{}", texts.search_truncated_bytes));
    }
    ToolOutcome {
        ok: true,
        output: out,
    }
}

/// patch：把一段**自由格式**的补丁应用到一处或多处文件（信封之后原样跟补丁文本，不走 JSON）。
/// 原子性：先解析 + 寻址 + 读入 + 在内存里算出全部新内容；任何一块不成立就**一个文件都不写**，
/// 回执点名第几块、为什么。同一个文件被多块改到时，后一块看到前一块的结果。
fn apply_patch(sb: &Sandbox, io: &dyn SysIo, obs: &mut Observations, body: &str) -> ToolOutcome {
    use crate::capabilities::tools::domain::patch::Block;
    let texts = &sb.texts;
    let blocks = match crate::capabilities::tools::domain::patch::parse(body) {
        Ok(b) => b,
        Err(f) => return fail(patch_fault(texts, &f)),
    };
    // 首次出现的顺序 = 写盘顺序（同一路径只写一次）
    let mut order: Vec<PathBuf> = Vec::new();
    let mut places: BTreeMap<PathBuf, Place> = BTreeMap::new();
    let mut pending: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut lines: Vec<String> = Vec::new();
    for (i, b) in blocks.iter().enumerate() {
        let n = i + 1;
        let spec = b.path().to_string();
        let (place, path) = match sb.resolve(&spec) {
            Ok(x) => x,
            Err(e) => return fail(block_fault(texts, n, e)),
        };
        if !sb.can_write(&place, &path) {
            return fail(block_fault(texts, n, sb.write_refusal(&place, &path)));
        }
        // 同一个文件被前面的块改过：以后面算出来的内容为准（不能回到盘上的旧内容）
        let source: Option<(String, bool, bool)> = match pending.get(&path) {
            Some(t) => Some((t.clone(), false, false)),
            None => match io.read(&path) {
                Ok(g) => Some((g.text, g.cut, g.lossy)),
                Err(_) => None,
            },
        };
        match b {
            Block::Add { content, .. } => match &source {
                Some((cur, _, _)) => {
                    if let Err(why) = overwrite_check(texts, obs, &spec, &path, cur) {
                        return fail(block_fault(texts, n, why));
                    }
                    lines.push(texts.render(
                        &texts.patch_block_overwrite,
                        &[
                            ("path", spec.clone()),
                            ("chars", content.chars().count().to_string()),
                        ],
                    ));
                    pending.insert(path.clone(), align_eol(content, cur));
                }
                None => {
                    lines.push(texts.render(
                        &texts.patch_block_add,
                        &[
                            ("path", spec.clone()),
                            ("chars", content.chars().count().to_string()),
                        ],
                    ));
                    pending.insert(path.clone(), content.clone());
                }
            },
            Block::Update { edits, .. } => {
                let Some((cur, cut, lossy)) = source else {
                    let why = texts.render(&texts.patch_file_missing, &[("path", spec.clone())]);
                    return fail(block_fault(texts, n, why));
                };
                // 只看到开头 / 看到的是替换字符：照这个视图改写会把没看到的东西弄丢 → 不改
                if cut {
                    let why = texts.render(&texts.edit_file_cut, &[("path", spec.clone())]);
                    return fail(block_fault(texts, n, why));
                }
                if lossy {
                    let why = texts.render(&texts.edit_file_lossy, &[("path", spec.clone())]);
                    return fail(block_fault(texts, n, why));
                }
                match crate::capabilities::tools::domain::patch::apply_edits(&cur, edits) {
                    Ok(new) => {
                        lines.push(texts.render(
                            &texts.patch_block_update,
                            &[("path", spec.clone()), ("n", edits.len().to_string())],
                        ));
                        pending.insert(path.clone(), new);
                    }
                    Err((k, f)) => {
                        let why = edit_fault_reason(texts, &f);
                        return fail(texts.render(
                            &texts.patch_edit_fault,
                            &[
                                ("n", n.to_string()),
                                ("k", k.to_string()),
                                ("path", spec.clone()),
                                ("why", why),
                            ],
                        ));
                    }
                }
            }
        }
        places.entry(path.clone()).or_insert(place);
        if !order.contains(&path) {
            order.push(path.clone());
        }
    }
    // 全部就绪 → 写盘（一个文件只写一次）
    for (i, path) in order.iter().enumerate() {
        let content = pending.get(path).expect("刚刚算好");
        if let Err(e) = io.write(path, content) {
            return fail(texts.render(
                &texts.patch_write_failed,
                &[("n", (i + 1).to_string()), ("error", e)],
            ));
        }
        obs.note_written(path, fnv1a(content));
    }
    let mut out = texts.render(
        &texts.patch_header,
        &[
            ("blocks", blocks.len().to_string()),
            ("files", order.len().to_string()),
            ("lines", lines.join("\n")),
        ],
    );
    // 写进模块目录属于「动了自己的能力」：放行，但如实提示（与 write 同一句）
    for path in &order {
        if let Some(Place::Module(id)) = places.get(path) {
            out.push_str(&format!(
                "\n{}",
                texts.render(
                    &texts.write_module_note,
                    &[("mark", MODULE_WRITE_MARK.to_string()), ("id", id.clone())]
                )
            ));
        }
    }
    ToolOutcome {
        ok: true,
        output: out,
    }
}
