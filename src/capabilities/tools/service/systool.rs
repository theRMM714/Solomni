//! 内置工具的**执行编排**：驱动 `SysIo` 读写盘上的文件（read / write / edit / patch / delete / list / search）。
//!
//! 纯规则在 `domain/systool.rs`（参数校验、回执文案、观察账本、`ToolOutcome`）——本文件只调用它们；
//! 因此这里引 `ports`（IO 机制），`domain/` 不引（见 ARCHITECTURE.md §九.3）。

use crate::capabilities::tools::domain::systool::*;
use crate::capabilities::tools::ports::SysIo;
use crate::capabilities::workspace::api::{Place, Sandbox};
use std::collections::{BTreeMap, BTreeSet};
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
    // 自由格式工具：输入是一段原样文本（不是 JSON），也不吃参数校验——认工具后就交给它自己解释。
    if is_freeform(name) {
        if !book.contains_key(name) {
            return fail(format!("未知的内置工具：{}", name));
        }
        return match name {
            PATCH => apply_patch(sb, io, obs, args_json),
            other => fail(format!("未知的内置工具：{}", other)),
        };
    }
    let mut args: serde_json::Value = match serde_json::from_str(args_json) {
        Ok(v) => v,
        Err(e) => return fail(format!("参数不是合法 JSON：{}", e)),
    };
    let Some(schema) = book.get(name) else {
        return fail(format!("未知的内置工具：{}", name));
    };
    if let Err(fault) = schema.check(&args) {
        return fail(arg_fault_text(name, schema, &fault));
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
    if name == DELETE {
        return delete(sb, io, &args);
    }
    let spec = match args.get("path").and_then(|p| p.as_str()) {
        Some(p) => p.to_string(),
        // 声明里每个内置工具都声明了必填 path，走到这里说明声明与实现不一致。
        None => return fail("缺少必填参数 path".to_string()),
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
    // 词法上在根内不等于真实落点在根内：符号链接 / junction 指向根外时一律拒绝。
    let Some(root) = sb.root_of(&place) else {
        return fail(format!("找不到落点对应的根：{}", spec));
    };
    if let Err(e) = io.within(root, &path) {
        return fail(e);
    }
    match name {
        READ => read(io, obs, &args, &spec, &path),
        WRITE => write(io, obs, &args, &spec, &path),
        EDIT => edit(io, obs, &args, &spec, &path),
        LIST => list(io, &spec, &path),
        SEARCH => search(io, &args, &spec, &path),
        other => fail(format!("未知的内置工具：{}", other)),
    }
}

/// list：列目录（名字 / 是否目录 / 字节数，按名字排序）。
/// 存在的理由：确认"资料齐不齐、脚本在不在、运行包装没装"必须能列目录——read 只读文件。
fn list(io: &dyn SysIo, spec: &str, path: &Path) -> ToolOutcome {
    let entries = match io.list(path) {
        Ok(e) => e,
        Err(e) => return fail(e),
    };
    let rows = if entries.is_empty() {
        "（空目录）".to_string()
    } else {
        entries
            .iter()
            .map(|e| {
                format!(
                    "  {}{}  {} 字节",
                    e.name,
                    if e.is_dir { "/" } else { "" },
                    e.bytes
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let header = format!("[list] {} （{} 项）", spec, entries.len());
    ToolOutcome {
        ok: true,
        output: format!("{}\n{}", header, rows),
    }
}

/// delete：只删**该 agent 自己的沙箱**。默认只删文件；目录必须显式 recursive=true。
/// 共享区走 work_commit 的 deletes，模块目录（含 userdata）由模块自己的工具维护。
/// 删除链接只删链接本身，绝不顺着链接去删目标。
fn delete(sb: &Sandbox, io: &dyn SysIo, args: &serde_json::Value) -> ToolOutcome {
    let Some(raw_paths) = args.get("paths").and_then(|v| v.as_array()) else {
        return fail("缺少必填参数 paths（要删除的路径数组）".to_string());
    };
    if raw_paths.is_empty() {
        return fail("paths 不能为空".to_string());
    }
    let recursive = args
        .get("recursive")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    struct Plan {
        spec: String,
        path: PathBuf,
        is_dir: bool,
    }
    let mut plans: Vec<Plan> = Vec::new();
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    // 先全部校验、一个都不删：目录没 recursive、越权、路径不存在都在动盘前整体拒绝。
    for raw in raw_paths {
        let Some(spec) = raw.as_str() else {
            return fail("paths 里每一项都必须是路径字符串".to_string());
        };
        let (place, path) = match sb.resolve(spec) {
            Ok(x) => x,
            Err(e) => return fail(e),
        };
        if !matches!(&place, Place::Private) {
            let why = match &place {
                Place::Shared => "共享区不在这里删：用 work_commit 的 deletes 显式列出",
                Place::Module(_) => {
                    "模块目录（含 userdata）由模块自己的工具 / 服务进程维护；delete 只删你自己的沙箱"
                }
                Place::Private => unreachable!(),
            };
            return fail(format!("delete 拒绝 {}：{}", spec, why));
        }
        if !seen.insert(path.clone()) {
            continue;
        }
        let Some(root) = sb.root_of(&place) else {
            return fail(format!("找不到落点对应的根：{}", spec));
        };
        if let Err(e) = io.within_parent(root, &path) {
            return fail(e);
        }
        let is_dir = match io.is_dir(&path) {
            Ok(v) => v,
            Err(e) => return fail(e),
        };
        if is_dir && !recursive {
            return fail(format!("{} 是目录；要整棵删除，加 recursive=true", spec));
        }
        plans.push(Plan {
            spec: spec.to_string(),
            path,
            is_dir,
        });
    }
    // 先删更深的路径：paths 里同时给了父目录和子路径时，顺序不影响结果。
    plans.sort_by(|a, b| {
        b.path
            .components()
            .count()
            .cmp(&a.path.components().count())
    });
    let mut out = String::from("已删除：");
    for p in plans {
        match io.delete(&p.path, recursive) {
            Ok(()) => out.push_str(&format!(
                "\n- {}（{}）",
                p.spec,
                if p.is_dir { "目录" } else { "文件/链接" }
            )),
            Err(e) => {
                return fail(format!(
                    "删除中断：{} 失败：{}；它前面的已删，后面的未动",
                    p.spec, e
                ))
            }
        }
    }
    ToolOutcome {
        ok: true,
        output: out,
    }
}

/// read：按行区间返回，行号从 1 数起；回执末尾告诉模型接着用哪个 offset，并如实记账"读到的是不是全文"。
fn read(
    io: &dyn SysIo,
    obs: &mut Observations,
    args: &serde_json::Value,
    spec: &str,
    path: &Path,
) -> ToolOutcome {
    let got = match io.read(path) {
        Ok(g) => g,
        // 目录不是文件：如实引导到 list，而不是把 IO 错原样丢给模型。
        Err(e) => match io.list(path) {
            Ok(_) => {
                return fail(format!(
                    "这是目录，不是文件：用 list 列它的内容（{}）。",
                    spec
                ))
            }
            Err(_) => return fail(e),
        },
    };
    let header = |body: String| format!("[read] {} （{} 字节）\n{}", spec, got.bytes, body);
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
        obs.note_read(
            path,
            false,
            "什么都没读到（offset 越过了末行）".to_string(),
            hash,
        );
        let note = format!("（超出末行：该文件共 {} 行，offset 从 1 数起）", total);
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
        let mut row = format!("{}: {}", i + 1, text);
        if cut {
            row.push_str(&format!(
                "［本行超过 {} 字符，已截断］",
                MAX_READ_LINE_CHARS
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
        "文件含非法 UTF-8 字节，读到的不是全部原文".to_string()
    } else if got.cut {
        "文件超过单次读取上限，只读到开头部分".to_string()
    } else {
        format!("只读了第 {}-{} 行（共 {} 行）", offset, to, total)
    };
    obs.note_read(path, complete, why, hash);
    let mut out = header(body.join("\n"));
    let tail = if got.cut {
        // 机制层只读了开头：总行数不可知，也不能声称"到文件末尾"。
        format!(
            "（已显示第 {}-{} 行；文件超过单次读取上限，只读到开头部分，总行数这次无法统计）",
            offset, to
        )
    } else if to < total {
        format!(
            "（已显示第 {}-{} 行，共 {} 行；继续读用 offset={}）",
            offset,
            to,
            total,
            to + 1
        )
    } else {
        format!("（已到文件末尾，共 {} 行）", total)
    };
    out.push_str(&format!("\n{}", tail));
    if got.lossy {
        out.push_str(
            "\n[编码] 该文件含非法 UTF-8 字节，以上是按替换字符呈现的近似内容；本程序不猜编码。",
        );
    }
    ToolOutcome {
        ok: true,
        output: out,
    }
}

/// write：整份写入（同名文件被整份覆盖）。
/// 覆盖已存在的文件必须先有"完整读过、且读后没被改过"的证据——否则拒绝，并让模型改用 edit。
fn write(
    io: &dyn SysIo,
    obs: &mut Observations,
    args: &serde_json::Value,
    spec: &str,
    path: &Path,
) -> ToolOutcome {
    let content = args
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or_default();
    match io.read(path) {
        // 读不到 = 还没有这个文件（新建不需要读过什么）；机制层的真实错误由下面的 write 如实报出。
        Err(_) => {}
        Ok(got) => {
            if let Err(why) = overwrite_check(obs, spec, path, &got.text) {
                return fail(why);
            }
        }
    }
    if let Err(e) = io.write(path, content) {
        return fail(e);
    }
    obs.note_written(path, fnv1a(content));
    ToolOutcome {
        ok: true,
        output: format!(
            "[write] 已写入 {}（{} 字符，UTF-8）",
            spec,
            content.chars().count()
        ),
    }
}

/// edit：按字面替换一处（默认要求唯一命中）。证据是 old_string 本身——它就是要改的那段原文，不需要先读过。
/// 被截断或含非法 UTF-8 的文件一律不改（改写会把没读到的部分或原始字节一起弄丢）。
fn edit(
    io: &dyn SysIo,
    obs: &mut Observations,
    args: &serde_json::Value,
    spec: &str,
    path: &Path,
) -> ToolOutcome {
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
        return fail(
            "old_string 与 new_string 相同：这次调用什么都不会变，核心没有写盘。".to_string(),
        );
    }
    let got = match io.read(path) {
        Ok(g) => g,
        Err(e) => return fail(e),
    };
    if got.cut {
        return fail(format!(
            "{} 超过单次读取上限，核心只读到开头部分：照这个视图改写会丢掉没读到的内容，所以这次没有写盘。",
            spec
        ));
    }
    if got.lossy {
        return fail(format!(
            "{} 含非法 UTF-8 字节，本程序不猜编码：改写会把整份文件重写成 UTF-8，所以这次没有写盘。",
            spec
        ));
    }
    let hay = got.text;
    let hits = find_all(&hay, old);
    if hits.is_empty() {
        let mut why = format!(
            "没找到 old_string（按**字面**匹配，含缩进与换行）。该文件共 {} 行；请先用 read 分段看清原文，或改用 write 整份重写。",
            hay.lines().count()
        );
        if let Some((line, actual)) = near_match(&hay, old) {
            why.push('\n');
            why.push_str(&format!("（第 {} 行与它只差空白：{}）", line, actual));
        }
        return fail(why);
    }
    if hits.len() > 1 && !all {
        let lines: Vec<String> = hits.iter().map(|i| line_of(&hay, *i).to_string()).collect();
        return fail(format!(
            "old_string 命中 {} 处（第 {} 行）：补足上下文让它唯一，或加 \"replace_all\":true 全改。",
            hits.len(),
            lines.join("、")
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
        output: format!(
            "[edit] 已改 {}（替换 {} 处；该文件现 {} 行）",
            spec,
            n,
            out.lines().count()
        ),
    }
}

/// search：逐行找关键词，返回带行号的命中行（命中总数与截断如实报）。
fn search(io: &dyn SysIo, args: &serde_json::Value, spec: &str, path: &Path) -> ToolOutcome {
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
        "（忽略大小写）"
    } else {
        "（区分大小写）"
    };
    let mut out = format!("[search] {} 关键词「{}」{}", spec, keyword, mode);
    if hits.is_empty() {
        out.push_str("\n（没有命中行）");
    }
    for (n, line) in &hits {
        out.push_str(&format!("\n{} | {}", n, line));
    }
    out.push_str(&format!("\n命中 {} 行 / 全文 {} 行", matched, total));
    if matched > hits.len() {
        out.push_str(&format!(
            "\n[截断] 命中行超过 {} 行，仅列出前 {} 行。",
            MAX_SEARCH_HITS, MAX_SEARCH_HITS
        ));
    }
    if got.lossy {
        out.push_str(
            "\n[编码] 该文件含非法 UTF-8 字节，以上是按替换字符呈现的近似内容；本程序不猜编码。",
        );
    }
    if got.cut {
        out.push_str("\n[截断] 文件超过单次读取上限，只搜了开头部分。");
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
    let blocks = match crate::capabilities::tools::domain::patch::parse(body) {
        Ok(b) => b,
        Err(f) => return fail(patch_fault(&f)),
    };
    // 首次出现的顺序 = 写盘顺序（同一路径只写一次）
    let mut order: Vec<PathBuf> = Vec::new();
    let mut pending: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut lines: Vec<String> = Vec::new();
    for (i, b) in blocks.iter().enumerate() {
        let n = i + 1;
        let spec = b.path().to_string();
        let (place, path) = match sb.resolve(&spec) {
            Ok(x) => x,
            Err(e) => return fail(block_fault(n, e)),
        };
        if !sb.can_write(&place, &path) {
            return fail(block_fault(n, sb.write_refusal(&place, &path)));
        }
        let Some(root) = sb.root_of(&place) else {
            return fail(block_fault(n, format!("找不到落点对应的根：{}", spec)));
        };
        if let Err(e) = io.within(root, &path) {
            return fail(block_fault(n, e));
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
                    if let Err(why) = overwrite_check(obs, &spec, &path, cur) {
                        return fail(block_fault(n, why));
                    }
                    lines.push(format!(
                        "- 整份覆盖 {}（{} 字符）",
                        spec,
                        content.chars().count()
                    ));
                    pending.insert(path.clone(), align_eol(content, cur));
                }
                None => {
                    lines.push(format!(
                        "- 新建 {}（{} 字符）",
                        spec,
                        content.chars().count()
                    ));
                    pending.insert(path.clone(), content.clone());
                }
            },
            Block::Update { edits, .. } => {
                let Some((cur, cut, lossy)) = source else {
                    let why = format!(
                        "{} 不存在：Update 只能改已有文件，新建请用 *** Add File:",
                        spec
                    );
                    return fail(block_fault(n, why));
                };
                // 只看到开头 / 看到的是替换字符：照这个视图改写会把没看到的东西弄丢 → 不改
                if cut {
                    let why = format!(
                        "{} 超过单次读取上限，核心只读到开头部分：照这个视图改写会丢掉没读到的内容，所以这次没有写盘。",
                        spec
                    );
                    return fail(block_fault(n, why));
                }
                if lossy {
                    let why = format!(
                        "{} 含非法 UTF-8 字节，本程序不猜编码：改写会把整份文件重写成 UTF-8，所以这次没有写盘。",
                        spec
                    );
                    return fail(block_fault(n, why));
                }
                match crate::capabilities::tools::domain::patch::apply_edits(&cur, edits) {
                    Ok(new) => {
                        lines.push(format!("- 更新 {}（替换 {} 处）", spec, edits.len()));
                        pending.insert(path.clone(), new);
                    }
                    Err((k, f)) => {
                        let why = edit_fault_reason(&f);
                        return fail(format!(
                            "[patch] 第 {} 块的第 {} 处改动不成立，**没有任何文件被写入**：{}（{}）",
                            n, k, why, spec
                        ));
                    }
                }
            }
        }
        if !order.contains(&path) {
            order.push(path.clone());
        }
    }
    // 全部就绪 → 写盘（一个文件只写一次）
    for (i, path) in order.iter().enumerate() {
        let content = pending.get(path).expect("刚刚算好");
        if let Err(e) = io.write(path, content) {
            return fail(format!(
                "[patch] 写第 {} 个文件失败：{}（它前面的文件已经写入，它后面的没有）",
                i + 1,
                e
            ));
        }
        obs.note_written(path, fnv1a(content));
    }
    ToolOutcome {
        ok: true,
        output: format!(
            "[patch] 已应用 {} 块改动，写了 {} 个文件：\n{}",
            blocks.len(),
            order.len(),
            lines.join("\n")
        ),
    }
}
