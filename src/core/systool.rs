//! 核心自带的内置工具：read / write / edit / search。
//! 策略在 core（名字固定、放行、寻址、根内校验、参数校验、改动前的"读过"证据、回执文案）；机制在 SysIo 端口（适配层）。
//! **参数契约不在本文件里**：声明在 systools/tools.yaml 的 tools，本文件只按声明校验、取缺省值与拼回执。
//! 存在的理由：读盘落盘不经过任何外部进程，编码问题不进本程序——模型自己看内容自己决定。
//! 路径一律是真实绝对路径（根目录经提示词册如实告知）；模块声明的外部工具与内置工具用同一套路径。

use crate::core::ports::{SysIo, ToolOutcome};
use crate::core::prompt::{Prompts, ToolTexts};
use crate::core::schema::{ArgFault, ToolSchema};
use crate::core::workspace::{Place, Sandbox};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const READ: &str = "read";
pub const WRITE: &str = "write";
pub const EDIT: &str = "edit";
pub const PATCH: &str = "patch";
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
/// 与 systools/tools.yaml 的 tools 是同一份名单，测试「builtin_tool_book_is_the_one_source_of_names_and_paths」锁死两者一致。
pub fn is_builtin(name: &str) -> bool {
    name == READ || name == WRITE || name == EDIT || name == PATCH || name == SEARCH
}

/// 内置工具名清单（拼错误提示用）。
pub fn names() -> Vec<String> {
    vec![
        READ.to_string(),
        WRITE.to_string(),
        EDIT.to_string(),
        PATCH.to_string(),
        SEARCH.to_string(),
    ]
}

/// patch 在**原生通道**上的声明：参数只有一个 body。
/// 原生协议要求参数是 JSON 对象，所以补丁正文当字符串值传——转义交给供应商的解码器，
/// 模型不必自己写转义（这正是原生通道相对手写信封的收益）。
pub fn patch_decl() -> crate::core::ports::ToolDecl {
    crate::core::ports::ToolDecl {
        name: PATCH.to_string(),
        description:
            "用一段补丁文本改文件（*** Add File: 路径 / *** Update File: 路径 … *** End File）"
                .to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "body": { "type": "string", "description": "补丁正文（原样写，不用转义换行）" }
            },
            "required": ["body"],
            "additionalProperties": false,
        }),
    }
}

/// 自由格式工具：输入不是 JSON 参数，而是**信封之后的那段原样文本**（不必转义）。
/// 存在的理由：把大段内容塞进 JSON 字符串要转义换行/引号，是真实会话里反复出事的点。
pub fn is_freeform(name: &str) -> bool {
    name == PATCH
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
                    &[
                        ("id", id.clone()),
                        ("root", crate::core::workspace::slash(root)),
                    ],
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
            (
                "tool_params",
                crate::core::schema::render_book(&sb.builtin_tools),
            ),
            (
                "patch_guide",
                prompts.render(
                    &prompts.core.patch_guide,
                    &[
                        ("work_root", crate::core::workspace::slash(&sb.shared)),
                        ("sandbox_root", crate::core::workspace::slash(&sb.private)),
                    ],
                ),
            ),
        ],
    )
}

/// 观察账本：**本次会话里核心见过哪些文件的什么内容**。
/// 它存在的唯一理由：整份覆盖（write）会丢掉没读到的内容，所以要求"先完整读过、且读后没被改过"。
/// 语义边界：账本随会话（AgentSession）保存；回档（rewind）清空——转录里那段读取证据被截掉了，证据随之作废；
/// 重启后按落盘重建的会话不带账本（从零开始，模型重新读一遍即可）。edit 不需要账本：old_string 本身就是要改的那段原文。
#[derive(Debug, Clone, Default)]
pub struct Observations {
    seen: BTreeMap<PathBuf, Seen>,
}

#[derive(Debug, Clone)]
enum Seen {
    /// 完整读过（或由核心写入）：当时的内容指纹。
    Known(u64),
    /// 只读到一部分（原因文案）：整份覆盖会被拒。
    Partial(String),
}

impl Observations {
    /// 回档时清空（转录里那段读取证据已经不存在了）。
    pub fn clear(&mut self) {
        self.seen.clear();
    }

    /// 记一次读取：完整读到 = 记指纹（可覆盖）；不完整 = 记下"只读到一部分"的原因。
    /// 不完整的读取**不覆盖**已有的完整记录（读到一半不会让已知变成未知）。
    fn note_read(&mut self, path: &Path, complete: bool, why: String, hash: u64) {
        if complete {
            self.seen.insert(path.to_path_buf(), Seen::Known(hash));
        } else {
            self.seen
                .entry(path.to_path_buf())
                .or_insert(Seen::Partial(why));
        }
    }

    /// 记一次写入：文件内容由核心产生，所以核心确切知道它现在是什么。
    fn note_written(&mut self, path: &Path, hash: u64) {
        self.seen.insert(path.to_path_buf(), Seen::Known(hash));
    }

    /// 合并一个**并发分支**的账本（分支里只跑声明可并发的只读工具）。
    /// 规则与逐个记账一致：完整读到记为所见、部分读到只补空白（读到一半不会让已知变成未知）。
    /// 调用方**必须按原始调用顺序**合并——那样并发批次与串行执行的结果完全相同。
    pub fn absorb(&mut self, branch: &Observations) {
        for (path, seen) in &branch.seen {
            match seen {
                Seen::Known(h) => {
                    self.seen.insert(path.clone(), Seen::Known(*h));
                }
                Seen::Partial(why) => {
                    self.seen
                        .entry(path.clone())
                        .or_insert(Seen::Partial(why.clone()));
                }
            }
        }
    }

    fn known(&self, path: &Path) -> Option<u64> {
        match self.seen.get(path) {
            Some(Seen::Known(h)) => Some(*h),
            _ => None,
        }
    }

    fn partial_why(&self, path: &Path) -> Option<String> {
        match self.seen.get(path) {
            Some(Seen::Partial(w)) => Some(w.clone()),
            _ => None,
        }
    }
}

/// 内容指纹（FNV-1a 64 位）：只用来判"读过的和现在的是不是同一份"，不做安全用途。
fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// 字节位置对应的行号（从 1 数起）。
fn line_of(s: &str, at: usize) -> usize {
    s[..at].matches('\n').count() + 1
}

/// 字面匹配的全部命中位置（不重叠）。
fn find_all(hay: &str, needle: &str) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let mut from = 0usize;
    while let Some(i) = hay[from..].find(needle) {
        let at = from + i;
        out.push(at);
        from = at + needle.len();
    }
    out
}

/// "是不是只差空白"：按行比较（每行内空白折叠）。命中则给（行号, 该行原文）——模型据此改对缩进。
fn near_match(hay: &str, old: &str) -> Option<(usize, String)> {
    fn flat(line: &str) -> String {
        line.split_whitespace().collect::<Vec<_>>().join(" ")
    }
    let want: Vec<String> = old.lines().map(flat).collect();
    if want.is_empty() {
        return None;
    }
    let lines: Vec<&str> = hay.lines().collect();
    if want.len() > lines.len() {
        return None;
    }
    for i in 0..=(lines.len() - want.len()) {
        let got: Vec<String> = lines[i..i + want.len()].iter().map(|l| flat(l)).collect();
        if got == want {
            return Some((i + 1, lines[i].to_string()));
        }
    }
    None
}

/// 执行一次内置工具调用（args_json = 模型信封里的 args 对象）。
/// 顺序：解析 JSON → 认工具 → 按声明校验参数 → 补缺省 → 寻址（内置工具一律需要一个 path）。
pub fn execute(
    sb: &Sandbox,
    io: &dyn SysIo,
    obs: &mut Observations,
    name: &str,
    args_json: &str,
) -> ToolOutcome {
    let texts = &sb.texts;
    // 自由格式工具：输入是一段原样文本（不是 JSON），也不吃参数校验——认工具后就交给它自己解释。
    if is_freeform(name) {
        if !sb.builtin_tools.contains_key(name) {
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
        READ => read(sb, io, obs, &args, &spec, &path),
        WRITE => write(sb, io, obs, &args, &spec, &path, &place),
        EDIT => edit(sb, io, obs, &args, &spec, &path),
        SEARCH => search(sb, io, &args, &spec, &path),
        other => fail(
            sb.texts
                .render(&sb.texts.unknown_builtin, &[("name", other.to_string())]),
        ),
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
        Err(e) => return fail(e),
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

/// 整份覆盖已存在文件的证据检查：本次会话里**完整读过**它、且读后没被改过（write 与 patch 的 Add 共用）。
fn overwrite_check(
    texts: &ToolTexts,
    obs: &Observations,
    spec: &str,
    path: &Path,
    current: &str,
) -> Result<(), String> {
    match obs.known(path) {
        Some(seen) if seen == fnv1a(current) => Ok(()),
        Some(_) => Err(texts.render(&texts.write_stale, &[("path", spec.to_string())])),
        None => Err(match obs.partial_why(path) {
            Some(w) => texts.render(
                &texts.write_partial,
                &[("path", spec.to_string()), ("why", w)],
            ),
            None => texts.render(&texts.write_need_read, &[("path", spec.to_string())]),
        }),
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
    use crate::core::patch::Block;
    let texts = &sb.texts;
    let blocks = match crate::core::patch::parse(body) {
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
                match crate::core::patch::apply_edits(&cur, edits) {
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

/// 整份覆盖已有文件时，内容按原文件的行尾风格写（免得把 CRLF 文件改成 LF）。
fn align_eol(content: &str, current: &str) -> String {
    if current.contains("\r\n") {
        content.replace('\n', "\r\n")
    } else {
        content.to_string()
    }
}

/// 某一块不成立的回执（整体不写盘，所以措辞要直说这一点）。
fn block_fault(texts: &ToolTexts, n: usize, why: String) -> String {
    texts.render(
        &texts.patch_block_fault,
        &[("n", n.to_string()), ("why", why)],
    )
}

/// patch 解析失败的回执（每一类都说清事实）。
fn patch_fault(texts: &ToolTexts, f: &crate::core::patch::Fault) -> String {
    use crate::core::patch::Fault as F;
    match f {
        F::NoBlocks => texts.patch_no_blocks.clone(),
        F::UnknownMarker { line, text } => texts.render(
            &texts.patch_unknown_marker,
            &[("line", line.to_string()), ("text", text.clone())],
        ),
        F::MissingPath { line, marker } => texts.render(
            &texts.patch_missing_path,
            &[("line", line.to_string()), ("marker", marker.clone())],
        ),
        F::EmptyAdd { line } => texts.render(&texts.patch_empty_add, &[("line", line.to_string())]),
        F::EmptyUpdate { line } => {
            texts.render(&texts.patch_empty_update, &[("line", line.to_string())])
        }
        F::EmptySearch { line } => {
            texts.render(&texts.patch_empty_search, &[("line", line.to_string())])
        }
        F::MissingEnd { line } => {
            texts.render(&texts.patch_missing_end, &[("line", line.to_string())])
        }
        F::SearchWithoutReplace { line } => texts.render(
            &texts.patch_search_no_replace,
            &[("line", line.to_string())],
        ),
        F::ReplaceWithoutSearch { line } => texts.render(
            &texts.patch_replace_no_search,
            &[("line", line.to_string())],
        ),
    }
}

/// 一处改动失败的原因句（外面再套"第 N 块第 k 处"）。
fn edit_fault_reason(texts: &ToolTexts, f: &crate::core::patch::EditFault) -> String {
    use crate::core::patch::EditFault as E;
    match f {
        E::NotFound { total } => {
            texts.render(&texts.patch_not_found, &[("total", total.to_string())])
        }
        E::Multiple { lines } => texts.render(
            &texts.patch_multiple,
            &[
                ("n", lines.len().to_string()),
                (
                    "lines",
                    lines
                        .iter()
                        .map(|l| l.to_string())
                        .collect::<Vec<_>>()
                        .join("、"),
                ),
            ],
        ),
        E::NearMiss { line, actual } => texts.render(
            &texts.patch_near,
            &[("line", line.to_string()), ("actual", actual.clone())],
        ),
    }
}

/// 参数不符的回执：说清是哪一条不合（由声明判定），再把工具签名原样发回去。
pub fn arg_fault_text(
    texts: &ToolTexts,
    name: &str,
    schema: &ToolSchema,
    fault: &ArgFault,
) -> String {
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
        &[
            ("why", why),
            (
                "signature",
                format!("{}\n{}", name, schema.render_for_prompt()),
            ),
        ],
    )
}

fn fail(msg: String) -> ToolOutcome {
    ToolOutcome {
        ok: false,
        output: msg,
    }
}

/// 按字符截断（不劈开 UTF-8）；返回（文本, 是否截断）。
fn truncate_chars(s: &str, max: usize) -> (String, bool) {
    if s.chars().count() <= max {
        return (s.to_string(), false);
    }
    (s.chars().take(max).collect(), true)
}
