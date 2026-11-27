//! 目的：内置工具的**执行编排**——驱动 `SysIo` 读写盘上的文件（read / write / edit / patch / delete / list / grep）。
//! 管：解析 JSON → 认工具 → 按声明校验参数 → 补缺省 → 寻址 → 调用 `domain` 的纯规则。
//! 不管：纯规则（参数校验、回执文案、观察账本、`ToolOutcome`）在 `domain/systool.rs`。
//! 联动：引 `ports` 的 `SysIo` 做 IO；`domain/` 不引 `ports`（见 docs/architecture/boundaries.md 的「三分法」）。

use crate::units::business::tools::api::{
    Coverage, GateFile, GateItem, LineRange, Need, ObservationFact, ObservationSource,
};
use crate::units::business::tools::domain::systool::*;
use crate::units::business::tools::ports::{Expect, Mutation, SysIo, WriteFault, WriteOp};
use crate::units::business::workspace::api::{Place, Sandbox};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
/// 目的：把门禁拒绝同时写进 output 与结构化说明（稳定错误码供 UI / 模型分流）。
fn gate_note(
    fault: &crate::units::business::tools::api::GateFault,
    path: &str,
) -> crate::kernel::api::note::Note {
    crate::kernel::api::note::Note::new(fault.code(), crate::kernel::api::note::Severity::Error)
        .arg("path", path)
}

fn gate_fail(
    fault: &crate::units::business::tools::api::GateFault,
    path: &str,
) -> crate::kernel::api::types::ToolOutcome {
    let mut out = fail(fault.message(path));
    out.push_note(gate_note(fault, path));
    out
}

/// 目的：把写盘失败同时写进 output 与结构化说明（稳定错误码供 UI / 模型分流）。
fn write_fail(fault: &WriteFault, spec: &str) -> crate::kernel::api::types::ToolOutcome {
    let mut out = fail(format!("[{}] {}", fault.code, fault.message));
    out.push_note(
        crate::kernel::api::note::Note::new(fault.code, crate::kernel::api::note::Severity::Error)
            .arg("path", spec),
    );
    out
}

/// 目的：执行一次内置工具调用（`args_json` = 模型信封里的 args 对象），使用不按上下文收紧的静态预算。
/// 约束：只给测试直调；生产路径一律走 execute_with_budget。
#[cfg(test)]
pub fn execute(
    sb: &Sandbox,
    book: &crate::units::business::tools::api::ToolBook,
    io: &dyn SysIo,
    obs: &mut Observations,
    gate: Option<&dyn crate::units::business::tools::api::WriteGate>,
    name: &str,
    args_json: &str,
) -> ToolOutcome {
    execute_with_budget(
        sb,
        book,
        io,
        obs,
        gate,
        crate::units::business::tools::api::ResultBudget::full(),
        name,
        args_json,
    )
}

/// 目的：带本批预算执行内置工具——read 在**执行前**按它选行，事后再由调用方按同一预算兜底。
#[allow(clippy::too_many_arguments)]
pub fn execute_with_budget(
    sb: &Sandbox,
    book: &crate::units::business::tools::api::ToolBook,
    io: &dyn SysIo,
    obs: &mut Observations,
    gate: Option<&dyn crate::units::business::tools::api::WriteGate>,
    budget: crate::units::business::tools::api::ResultBudget,
    name: &str,
    args_json: &str,
) -> ToolOutcome {
    // 自由格式工具：输入是一段原样文本（不是 JSON），也不吃参数校验——认工具后就交给它自己解释。
    if is_freeform(name) {
        if !book.contains_key(name) {
            return fail(format!("未知的内置工具：{}", name));
        }
        return match name {
            PATCH => apply_patch(sb, io, obs, gate, args_json),
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
        return ToolOutcome::new(true, out);
    }
    if name == DELETE {
        return delete(sb, io, obs, &args);
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
        if !sb.can_write(&place) {
            return fail(sb.write_refusal(&place, &path));
        }
    } else if matches!(name, READ | LIST | GREP) && !sb.can_read(&place, &path) {
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
        READ => read(sb, &place, io, obs, budget, &args, &spec, &path),
        WRITE => write(
            gate,
            io,
            obs,
            &args,
            &spec,
            &path,
            &Origin::of(sb, &place, &path),
        ),
        EDIT => edit(
            gate,
            io,
            obs,
            &args,
            &spec,
            &path,
            &Origin::of(sb, &place, &path),
        ),
        LIST => list(io, &spec, &path),
        GREP => super::grep::run(sb, &place, io, budget, &args, &spec, &path),
        other => fail(format!("未知的内置工具：{}", other)),
    }
}

/// 目的：一条绝对路径在工作树内的相对写法（斜杠分隔）；拿不到根或不在根内 = None。
fn rel_of(sb: &Sandbox, place: &Place, path: &Path) -> Option<String> {
    let root = sb.root_of(place)?;
    let rel = path.strip_prefix(root).ok()?;
    Some(crate::kernel::api::path::slash(rel))
}

/// 目的：观察事实里记哪个工作树：共享区为空串，其余记 agent 名。
fn worktree_of(sb: &Sandbox, place: &Place) -> String {
    if matches!(place, Place::Shared) {
        String::new()
    } else {
        sb.agent.clone()
    }
}

/// 目的：一条路径在观察账本里的坐标（工作树相对路径 + 工作树名）；拿不到相对写法就没有坐标。
struct Origin {
    rel: Option<String>,
    worktree: String,
}

impl Origin {
    fn of(sb: &Sandbox, place: &Place, path: &Path) -> Origin {
        Origin {
            rel: rel_of(sb, place, path),
            worktree: worktree_of(sb, place),
        }
    }
}

/// 目的：记一条观察事实；路径拿不到相对写法就不记（宁可多要求一次读，也不记错路径）。
fn observe(
    obs: &mut Observations,
    origin: &Origin,
    version: String,
    coverage: Coverage,
    total_lines: usize,
    source: ObservationSource,
) {
    let Some(rel) = origin.rel.as_ref() else {
        return;
    };
    obs.note_fact(ObservationFact {
        schema: crate::units::business::tools::api::OBSERVATION_FORMAT,
        reply: obs.reply(),
        path: rel.clone(),
        worktree: origin.worktree.clone(),
        version,
        coverage,
        total_lines: total_lines as u32,
        source,
    });
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
    ToolOutcome::new(true, format!("{}\n{}", header, rows))
}

/// delete：只删**该 agent 自己的沙箱**。默认只删文件；目录必须显式 recursive=true。
/// 共享区走 work_commit 的 deletes，模块目录（含 userdata）由模块自己的工具维护。
/// 删除链接只删链接本身，绝不顺着链接去删目标。
fn delete(
    sb: &Sandbox,
    io: &dyn SysIo,
    obs: &mut Observations,
    args: &serde_json::Value,
) -> ToolOutcome {
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
            Ok(()) => {
                // 自动提交要按路径发 deletes；递归删目录暂不逐文件展开。
                if !p.is_dir {
                    obs.note_removed(&p.path);
                }
                out.push_str(&format!(
                    "\n- {}（{}）",
                    p.spec,
                    if p.is_dir { "目录" } else { "文件/链接" }
                ))
            }
            Err(e) => {
                return fail(format!(
                    "删除中断：{} 失败：{}；它前面的已删，后面的未动",
                    p.spec, e
                ));
            }
        }
    }
    ToolOutcome::new(true, out)
}

/// read：按行区间返回，行号从 1 数起；回执末尾告诉模型接着用哪个 offset，并如实记账"读到的是不是全文"。
#[allow(clippy::too_many_arguments)]
fn read(
    sb: &Sandbox,
    place: &Place,
    io: &dyn SysIo,
    obs: &mut Observations,
    budget: crate::units::business::tools::api::ResultBudget,
    args: &serde_json::Value,
    spec: &str,
    path: &Path,
) -> ToolOutcome {
    let offset = args
        .get("offset")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .max(1);
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(MAX_READ_LINES as u64)
        .clamp(1, MAX_READ_LINES as u64);
    // 机制层只取请求窗口，但会扫完整份文件算行数与版本：>1MB 的文件也能直接读后半段。
    let page = match io.read_page(path, offset as u32, limit as u32) {
        Ok(p) => p,
        // 目录不是文件：如实引导到 list，而不是把 IO 错原样丢给模型。
        Err(e) => match io.list(path) {
            Ok(_) => {
                return fail(format!(
                    "这是目录，不是文件：用 list 列它的内容（{}）。",
                    spec
                ));
            }
            Err(_) => return fail(e),
        },
    };
    let header = |body: String| format!("[read] {} （{} 字节）\n{}", spec, page.bytes, body);
    let total = page.total_lines as usize;
    let offset = offset as usize;
    let lines: Vec<&str> = page.text.lines().collect();
    let hash = fnv1a(&page.text);
    if offset > total {
        obs.note_read(
            path,
            false,
            "什么都没读到（offset 越过了末行）".to_string(),
            hash,
        );
        let note = format!("（超出末行：该文件共 {} 行，offset 从 1 数起）", total);
        return ToolOutcome::new(true, header(note));
    }
    // 正文预算：既不超过静态读取上限，也不超过本批分给这一条的剩余空间（给文件头与续读提示留 512 字符）。
    let body_budget = MAX_READ_CHARS
        .min(budget.limit().saturating_sub(512))
        .max(256);
    let mut body: Vec<String> = Vec::new();
    let mut used = 0usize;
    let mut to = offset - 1;
    let mut line_cut = page.line_cut;
    // 是否真的因为本批预算而少给了行：只有这时才提示“先压缩会话”，显式 limit 截断不提示。
    let mut budget_cut = false;
    for (i, line) in lines.iter().enumerate() {
        let line_no = offset + i;
        let (text, cut) = truncate_chars(line, MAX_READ_LINE_CHARS);
        line_cut |= cut;
        let mut row = format!("{}: {}", line_no, text);
        if cut {
            row.push_str(&format!(
                "［本行超过 {} 字符，已截断］",
                MAX_READ_LINE_CHARS
            ));
        }
        // 字符预算用完就停：至少给出第一行，绝不空手而归。
        let cost = row.chars().count() + 1;
        if used + cost > body_budget {
            budget_cut = true;
            if body.is_empty() {
                // 第一行就超过本批预算：按字符切到预算内并标成截断，绝不把它记成完整读过。
                let keep = body_budget.saturating_sub(40).max(32);
                let (head, _) = truncate_chars(&row, keep);
                row = format!("{}［本行超过本次预算，已截断］", head);
                line_cut = true;
                to = line_no;
                body.push(row);
            }
            break;
        }
        used += cost;
        to = line_no;
        body.push(row);
    }
    // 记账：只有"从头到尾、每一行都完整、没有编码损失"的一次读取才算见过全文。
    // 单行被 MAX_READ_LINE_CHARS 截断时，模型看到的是省略号，不是那一行的原文。
    let complete = !page.lossy && !line_cut && offset == 1 && to == total;
    let why = if page.lossy {
        "文件含非法 UTF-8 字节，读到的不是全部原文".to_string()
    } else if line_cut {
        "有单行超过显示上限被截断，读到的不是全部原文".to_string()
    } else if page.window_cut && to < total {
        format!(
            "只读了第 {}-{} 行（共 {} 行；本次到单次读取上限）",
            offset, to, total
        )
    } else {
        format!("只读了第 {}-{} 行（共 {} 行）", offset, to, total)
    };
    obs.note_read(path, complete, why, hash);
    // 观察事实：完整读 = 全文；分段读 = 读到的行区间。单行截断、编码损失都不记
    //（模型看到的不是原文，拿去授权改写会覆盖掉自己没看见的字节）。
    if !page.lossy && !line_cut {
        let coverage = if complete {
            Coverage::Full
        } else {
            Coverage::Ranges(vec![LineRange::new(offset as u32, to as u32)])
        };
        observe(
            obs,
            &Origin::of(sb, place, path),
            page.hash.clone(),
            coverage,
            total,
            ObservationSource::Read,
        );
    }
    let mut out = header(body.join("\n"));
    let tail = if to < total {
        if page.window_cut {
            format!(
                "（已显示第 {}-{} 行，共 {} 行；本次到单次读取上限，继续读用 offset={}）",
                offset,
                to,
                total,
                to + 1
            )
        } else {
            format!(
                "（已显示第 {}-{} 行，共 {} 行；继续读用 offset={}）",
                offset,
                to,
                total,
                to + 1
            )
        }
    } else {
        format!("（已到文件末尾，共 {} 行）", total)
    };
    out.push_str(&format!("\n{}", tail));
    if page.lossy {
        out.push_str(
            "\n[编码] 该文件含非法 UTF-8 字节，以上是按替换字符呈现的近似内容；本程序不猜编码。",
        );
    }
    let mut outcome = ToolOutcome::new(true, out);
    if budget_cut {
        outcome.push_note(
            crate::kernel::api::note::Note::warning(
                crate::kernel::api::note::codes::RESULT_TRUNCATED_CONTEXT,
            )
            .arg("limit", body_budget),
        );
    }
    outcome
}

/// write：整份写入（同名文件被整份覆盖）。
/// 覆盖已存在的文件必须先有"完整读过、且读后没被改过"的证据——否则拒绝，并让模型改用 edit。
fn write(
    gate: Option<&dyn crate::units::business::tools::api::WriteGate>,
    io: &dyn SysIo,
    obs: &mut Observations,
    args: &serde_json::Value,
    spec: &str,
    path: &Path,
    origin: &Origin,
) -> ToolOutcome {
    let content = args
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or_default();
    let current = match io.fingerprint(path) {
        // 读不到 = 还没有这个文件；机制层的真实错误由下面的 write 如实报出。
        Err(_) => {
            return fail(format!(
                "{} 现在读不到，无法判断是不是要覆盖已有内容；这次没有写盘。",
                spec
            ));
        }
        Ok(Some(hash)) => Some(GateFile { hash, size: None }),
        Ok(None) => None,
    };
    if let Some(gate) = gate {
        let Some(rel) = origin.rel.as_deref() else {
            return fail(format!(
                "找不到 {} 在工作树里的相对路径；这次没有写盘。",
                spec
            ));
        };
        if let Err(fault) = gate.check(rel, Need::Full, current.as_ref()) {
            return gate_fail(&fault, spec);
        }
    } else if current.is_some() {
        // 直调兜底（旧测试）：没有会话门禁时仍按本次会话的读账本判。
        if let Ok(got) = io.read(path)
            && let Err(why) = overwrite_check(obs, spec, path, &got.text)
        {
            return fail(why);
        }
    }
    // 期望版本在锁内再核一次：检查与落盘之间被别人改过就整份拒绝，而不是覆盖回去。
    let expect = match current.as_ref() {
        Some(file) => Expect::Version(file.hash.clone()),
        None => Expect::Absent,
    };
    let receipt = match io.mutate(path, &expect, Mutation::Replace(content.to_string())) {
        Ok(receipt) => receipt,
        Err(fault) => return write_fail(&fault, spec),
    };
    obs.note_written(path, fnv1a(content));
    observe(
        obs,
        origin,
        receipt.version,
        Coverage::Full,
        receipt.total_lines as usize,
        ObservationSource::Write,
    );
    ToolOutcome::new(
        true,
        format!(
            "[write] 已写入 {}（{} 字符，UTF-8）",
            spec,
            content.chars().count()
        ),
    )
}

/// edit：按字面替换一处（默认要求唯一命中）。证据是 old_string 本身——它就是要改的那段原文，不需要先读过。
/// 被截断或含非法 UTF-8 的文件一律不改（改写会把没读到的部分或原始字节一起弄丢）。
fn edit(
    gate: Option<&dyn crate::units::business::tools::api::WriteGate>,
    io: &dyn SysIo,
    obs: &mut Observations,
    args: &serde_json::Value,
    spec: &str,
    path: &Path,
    origin: &Origin,
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
    if let Some(gate) = gate {
        let Some(rel) = origin.rel.as_deref() else {
            return fail(format!(
                "找不到 {} 在工作树里的相对路径；这次没有写盘。",
                spec
            ));
        };
        let need = if all { Need::Full } else { Need::Observed };
        let current = GateFile {
            hash: got.hash.clone(),
            size: Some(got.bytes as u64),
        };
        if let Err(fault) = gate.check(rel, need, Some(&current)) {
            return gate_fail(&fault, spec);
        }
    }
    let version = got.hash;
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
    // 落盘交给 SysIo：锁内按"读到的那个版本"再校验，锚点也在同一份内容上重新定位。
    let mutation = Mutation::Edit {
        old: old.to_string(),
        new: new.to_string(),
        all,
    };
    let receipt = match io.mutate(path, &Expect::Version(version), mutation) {
        Ok(receipt) => receipt,
        Err(fault) => return write_fail(&fault, spec),
    };
    obs.note_touched(path);
    observe(
        obs,
        origin,
        receipt.version,
        Coverage::Ranges(receipt.changed),
        receipt.total_lines as usize,
        ObservationSource::Edit,
    );
    ToolOutcome::new(
        true,
        format!(
            "[edit] 已改 {}（替换 {} 处；该文件现 {} 行）",
            spec, receipt.replaced, receipt.total_lines
        ),
    )
}

/// patch：把一段**自由格式**的补丁应用到一处或多处文件（信封之后原样跟补丁文本，不走 JSON）。
/// 原子性：先解析 + 寻址 + 读入 + 在内存里算出全部新内容；任何一块不成立就**一个文件都不写**，
/// 回执点名第几块、为什么。同一个文件被多块改到时，后一块看到前一块的结果。
fn apply_patch(
    sb: &Sandbox,
    io: &dyn SysIo,
    obs: &mut Observations,
    gate: Option<&dyn crate::units::business::tools::api::WriteGate>,
    body: &str,
) -> ToolOutcome {
    use crate::units::business::tools::domain::patch::Block;
    let blocks = match crate::units::business::tools::domain::patch::parse(body) {
        Ok(b) => b,
        Err(f) => return fail(patch_fault(&f)),
    };
    // 首次出现的顺序 = 写盘顺序（同一路径只写一次）
    let mut order: Vec<PathBuf> = Vec::new();
    let mut pending: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut coverage: BTreeMap<PathBuf, Coverage> = BTreeMap::new();
    let mut sources: BTreeMap<PathBuf, ObservationSource> = BTreeMap::new();
    let mut places: BTreeMap<PathBuf, Place> = BTreeMap::new();
    // 每个路径的磁盘基线：事务提交时在锁内再比一次（不能拿补丁算出的新内容当期望）。
    let mut expects: BTreeMap<PathBuf, Expect> = BTreeMap::new();
    // 门禁检查攒到全部块解析完再一次性做：一批补丁只取一次工作区快照。
    let mut checks: Vec<(usize, String, String, Need, Option<GateFile>)> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    for (i, b) in blocks.iter().enumerate() {
        let n = i + 1;
        let spec = b.path().to_string();
        let (place, path) = match sb.resolve(&spec) {
            Ok(x) => x,
            Err(e) => return fail(block_fault(n, e)),
        };
        if !sb.can_write(&place) {
            return fail(block_fault(n, sb.write_refusal(&place, &path)));
        }
        let Some(root) = sb.root_of(&place) else {
            return fail(block_fault(n, format!("找不到落点对应的根：{}", spec)));
        };
        if let Err(e) = io.within(root, &path) {
            return fail(block_fault(n, e));
        }
        // 同一个文件被前面的块改过：以后面算出来的内容为准（不能回到盘上的旧内容）
        let source: Option<(String, bool, bool, String)> = match pending.get(&path) {
            Some(t) => Some((
                t.clone(),
                false,
                false,
                crate::units::business::versions::api::content_hash(t.as_bytes()),
            )),
            None => match io.read(&path) {
                Ok(g) => Some((g.text, g.cut, g.lossy, g.hash)),
                Err(e) => match io.fingerprint(&path) {
                    // 确实不存在：Add 才是新建。
                    Ok(None) => None,
                    // 文件在、但这次读不出来：不能当新建，否则后面的整份覆盖会把没读到的内容弄丢。
                    Ok(Some(_)) => {
                        return fail(block_fault(
                            n,
                            format!("读取 {} 失败，且它确实存在：{}；这次没有写盘。", spec, e),
                        ));
                    }
                    Err(f) => {
                        return fail(block_fault(
                            n,
                            format!("读取 {} 失败，也无法确认它是否存在：{}；{}", spec, e, f),
                        ));
                    }
                },
            },
        };
        if !order.contains(&path) {
            let expect = match &source {
                Some((_, _, _, hash)) => Expect::Version(hash.clone()),
                None => Expect::Absent,
            };
            expects.insert(path.clone(), expect);
        }
        // 每个路径只在第一块登记门禁：同一份补丁里后面的块看到的是前面算出来的内容。
        if !order.contains(&path) && gate.is_some() {
            let Some(rel) = rel_of(sb, &place, &path) else {
                return fail(block_fault(
                    n,
                    format!("找不到 {} 在工作树里的相对路径", spec),
                ));
            };
            let need = match b {
                Block::Add { .. } => Need::Full,
                Block::Update { .. } => Need::Observed,
            };
            let current = source.as_ref().map(|(_, _, _, hash)| GateFile {
                hash: hash.clone(),
                size: None,
            });
            checks.push((n, spec.clone(), rel, need, current));
        }
        match b {
            Block::Add { content, .. } => match &source {
                Some((cur, _, _, _)) => {
                    if gate.is_none()
                        && let Err(why) = overwrite_check(obs, &spec, &path, cur)
                    {
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
                let Some((cur, cut, lossy, _)) = source else {
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
                match crate::units::business::tools::domain::patch::apply_edits_tracked(&cur, edits)
                {
                    Ok((new, ranges)) => {
                        lines.push(format!("- 更新 {}（替换 {} 处）", spec, edits.len()));
                        if !matches!(coverage.get(&path), Some(Coverage::Full)) {
                            let mut all = match coverage.get(&path) {
                                Some(Coverage::Ranges(v)) => v.clone(),
                                _ => Vec::new(),
                            };
                            all.extend(
                                ranges
                                    .iter()
                                    .map(|(from, to)| LineRange::new(*from as u32, *to as u32)),
                            );
                            coverage.insert(path.clone(), Coverage::Ranges(all).normalized());
                        }
                        sources
                            .entry(path.clone())
                            .or_insert(ObservationSource::PatchUpdate);
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
        if matches!(b, Block::Add { .. }) {
            coverage.insert(path.clone(), Coverage::Full);
            sources.insert(path.clone(), ObservationSource::PatchAdd);
        }
        places.insert(path.clone(), place);
        if !order.contains(&path) {
            order.push(path.clone());
        }
    }
    // 写入前一次性过门禁：整批只取一次快照；任一路径不成立就一个文件都不写。
    if let Some(gate) = gate
        && !checks.is_empty()
    {
        let items: Vec<GateItem<'_>> = checks
            .iter()
            .map(|(_, _, rel, need, current)| GateItem {
                path: rel.as_str(),
                need: *need,
                current: current.as_ref(),
            })
            .collect();
        if let Err((k, fault)) = gate.check_many(&items) {
            let (n, spec, _, _, _) = &checks[k];
            let mut out = fail(block_fault(*n, fault.message(spec)));
            out.push_note(gate_note(&fault, spec));
            return out;
        }
    }
    // 全部就绪 → 一个事务写盘：任一文件失败，已发布的文件按原字节回滚。
    let ops: Vec<WriteOp> = order
        .iter()
        .map(|path| WriteOp {
            path: path.clone(),
            expect: expects.remove(path).unwrap_or(Expect::Absent),
            mutation: Mutation::Replace(pending.get(path).expect("刚刚算好").clone()),
        })
        .collect();
    let receipts = match io.transact(ops) {
        Ok(receipts) => receipts,
        Err(fault) => {
            return fail(format!(
                "[patch] 写盘事务失败，**没有任何文件被写入**（已回滚）：{}",
                fault.message
            ));
        }
    };
    for (path, receipt) in order.iter().zip(receipts) {
        let content = pending.get(path).expect("刚刚算好");
        match sources.get(path) {
            Some(ObservationSource::PatchAdd) => obs.note_written(path, fnv1a(content)),
            _ => obs.note_touched(path),
        }
        let coverage = coverage.get(path).cloned().unwrap_or(Coverage::Full);
        let origin = Origin::of(sb, places.get(path).expect("刚刚记下落点"), path);
        observe(
            obs,
            &origin,
            receipt.version,
            coverage,
            receipt.total_lines as usize,
            sources
                .get(path)
                .copied()
                .unwrap_or(ObservationSource::PatchUpdate),
        );
    }
    ToolOutcome::new(
        true,
        format!(
            "[patch] 已应用 {} 块改动，写了 {} 个文件：\n{}",
            blocks.len(),
            order.len(),
            lines.join("\n")
        ),
    )
}
