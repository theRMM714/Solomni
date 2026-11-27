//! 目的：内置 `grep` 的**匹配与输出编排**——按模式扫文件或目录，逐行给命中与上下文，并按过滤规则剪枝。
//! 管：模式编译（正则 / 字面、忽略大小写）、glob 过滤、目录遍历后的目标收敛、逐文件流式扫描、上下文拼行、命中与扫描计数、回执与截断说明。
//! 不管：遍历与剪枝机制（`ignore` 与 `Rules` 在 `src/units/business/tools/detail/sys_io.rs` 的 `SysIo::walk`）、结果预算规则（`src/units/business/tools/domain/budget.rs`）、工具面发放（`systools/tools.yaml`）。
//! 联动：`src/units/business/tools/service/systool.rs` 验参后调 `run`；过滤规则来自 `src/units/business/workbench/domain/filter.rs`，IO 走 `SysIo` 端口。

use crate::units::business::tools::api::ResultBudget;
use crate::units::business::tools::domain::systool::*;
use crate::units::business::tools::ports::{SysIo, WalkOpts};
use crate::units::business::workbench::api::{Area, Bench, WORKIGNORE_SEED};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

/// 目的：grep 的匹配器：正则（缺省）或按字面；忽略大小写在构造时折进去，逐行只判一次。
enum GrepMatch {
    Literal { needle: String, fold: bool },
    Regex(regex::Regex),
}

impl GrepMatch {
    fn is_hit(&self, line: &str) -> bool {
        match self {
            GrepMatch::Literal { needle, fold } => {
                if *fold {
                    line.to_lowercase().contains(needle.as_str())
                } else {
                    line.contains(needle.as_str())
                }
            }
            GrepMatch::Regex(re) => re.is_match(line),
        }
    }
}

/// 目的：按 `fixed` / `ignore_case` 造匹配器；正则不合法时把编译器的原因如实给出来。
fn grep_matcher(pattern: &str, fixed: bool, ignore_case: bool) -> Result<GrepMatch, String> {
    if fixed {
        return Ok(GrepMatch::Literal {
            needle: if ignore_case {
                pattern.to_lowercase()
            } else {
                pattern.to_string()
            },
            fold: ignore_case,
        });
    }
    regex::RegexBuilder::new(pattern)
        .case_insensitive(ignore_case)
        .build()
        .map(GrepMatch::Regex)
        .map_err(|e| {
            format!(
                "pattern 不是合法正则：{}。要按字面匹配就加 \"fixed\":true。",
                e
            )
        })
}

/// 目的：glob 过滤集合；没有 glob = None（不过滤）。
fn grep_globs(globs: &[String]) -> Result<Option<globset::GlobSet>, String> {
    if globs.is_empty() {
        return Ok(None);
    }
    let mut builder = globset::GlobSetBuilder::new();
    for g in globs {
        let glob = globset::Glob::new(g).map_err(|e| format!("glob 不合法（{}）：{}", g, e))?;
        builder.add(glob);
    }
    builder
        .build()
        .map(Some)
        .map_err(|e| format!("glob 不合法：{}", e))
}

/// 目的：一条相对路径过不过 glob：整条相对路径或文件名命中都算（`*.rs` 因此能匹配任意深度）。
fn glob_ok(set: &Option<globset::GlobSet>, rel: &str) -> bool {
    let Some(set) = set else {
        return true;
    };
    if set.is_match(rel) {
        return true;
    }
    let name = rel.rsplit('/').next().unwrap_or(rel);
    set.is_match(name)
}

/// 目的：读一个布尔参数（缺省 false）。
fn flag(args: &serde_json::Value, key: &str) -> bool {
    args.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

/// 目的：一个文件的扫描结果：按出现顺序排好的（行号, 文本, 是不是命中），外加如实统计。
struct GrepFile {
    rows: Vec<(u32, String, bool)>,
    hits: usize,
    binary: bool,
    lossy: bool,
    long_lines: u64,
}

/// 目的：流式扫一个文件并按 `context` 拼出待输出的行（命中行 + 前后文）。
/// 约束：`room` 是这次还能报多少条命中；超出后只继续计数，不再收集行。
fn grep_file(
    io: &dyn SysIo,
    path: &Path,
    matcher: &GrepMatch,
    context: usize,
    room: usize,
) -> Result<GrepFile, String> {
    let mut rows: Vec<(u32, String, bool)> = Vec::new();
    let mut before: VecDeque<(u32, String)> = VecDeque::new();
    let mut after_left = 0usize;
    let mut last_out = 0u32;
    let mut hits = 0usize;
    let summary = io.scan_lines(path, &mut |n, line| {
        let (text, _) = truncate_chars(line, MAX_GREP_LINE_CHARS);
        if matcher.is_hit(line) {
            hits += 1;
            if hits > room {
                return;
            }
            // 命中前的上下文按顺序补齐；已经作为后文吐过的行不重复。
            for (cn, ctext) in before.drain(..) {
                if cn > last_out {
                    rows.push((cn, ctext, false));
                    last_out = cn;
                }
            }
            if n > last_out {
                rows.push((n, text, true));
                last_out = n;
            }
            after_left = context;
            return;
        }
        if after_left > 0 {
            if n > last_out {
                rows.push((n, text, false));
                last_out = n;
            }
            after_left -= 1;
            return;
        }
        before.push_back((n, text));
        while before.len() > context {
            before.pop_front();
        }
    })?;
    Ok(GrepFile {
        rows,
        hits,
        binary: summary.binary,
        lossy: summary.lossy,
        long_lines: summary.long_lines,
    })
}

/// 目的：grep 的执行体——按模式在文件或目录里找行，回执「路径:行号:正文」（上下文行用「路径-行号-正文」）并给命中 / 扫描统计。
/// 约束：默认按 `.gitignore` / `.ignore` 与工作区过滤规则 + 隐藏文件 + 二进制剪枝，`no_ignore` 一关全关；显式单文件 `path` 不走遍历与剪枝。
/// 返回：成功时 `ok=true`；模式、glob、遍历或扫描失败时如实给出原因。
pub(super) fn run(
    sb: &Bench,
    place: &Area,
    io: &dyn SysIo,
    budget: ResultBudget,
    args: &serde_json::Value,
    spec: &str,
    path: &Path,
) -> ToolOutcome {
    let pattern = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if pattern.is_empty() {
        return fail(
            "缺少必填参数 pattern（要搜的文本；想按字面匹配加 \"fixed\":true）".to_string(),
        );
    }
    let fixed = flag(args, "fixed");
    let ignore_case = flag(args, "ignore_case");
    let files_only = flag(args, "files_only");
    let hidden = flag(args, "hidden");
    let no_ignore = flag(args, "no_ignore");
    let context = args
        .get("context")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        .min(MAX_GREP_CONTEXT as u64) as usize;
    let max = args
        .get("max")
        .and_then(|v| v.as_u64())
        .unwrap_or(MAX_GREP_HITS as u64)
        .clamp(1, MAX_GREP_MAX as u64) as usize;
    let globs: Vec<String> = args
        .get("glob")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let matcher = match grep_matcher(&pattern, fixed, ignore_case) {
        Ok(m) => m,
        Err(e) => return fail(e),
    };
    let set = match grep_globs(&globs) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let is_dir = match io.is_dir(path) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    // 过滤规则只有一处：版本库目录里的 .workignore（缺文件按种子名单兜底）。
    let rules = if no_ignore {
        None
    } else {
        match io.read_small(&sb.filter_rules) {
            Ok(Some(text)) => Some(text),
            Ok(None) => Some(WORKIGNORE_SEED.to_string()),
            Err(e) => return fail(e),
        }
    };
    let mut targets: Vec<(String, PathBuf)> = Vec::new();
    let mut walked = 0usize;
    let mut denied = 0usize;
    if is_dir {
        let opts = WalkOpts {
            hidden,
            ignore: !no_ignore,
        };
        let entries = match io.walk(path, rules.as_deref(), opts) {
            Ok(v) => v,
            Err(e) => return fail(e),
        };
        walked = entries.len();
        for ent in entries.iter().take(MAX_GREP_FILES) {
            if !glob_ok(&set, &ent.rel) {
                continue;
            }
            let mut real = path.to_path_buf();
            for seg in ent.rel.split('/') {
                real.push(seg);
            }
            if !sb.can_read(place, &real) {
                denied += 1;
                continue;
            }
            targets.push((real.display().to_string(), real));
        }
    } else {
        targets.push((path.display().to_string(), path.to_path_buf()));
    }
    let body_budget = MAX_GREP_CHARS
        .min(budget.limit().saturating_sub(512))
        .max(256);
    let mut body: Vec<String> = Vec::new();
    let mut used = 0usize;
    let mut total_hits = 0usize;
    let mut hit_files = 0usize;
    let mut scanned = 0usize;
    let mut skipped_binary = 0usize;
    let mut lossy = false;
    let mut long_lines = 0u64;
    let mut budget_cut = false;
    let mut per_file_cut = false;
    let mut stopped_at_max = false;
    for (display, real) in &targets {
        if total_hits >= max {
            stopped_at_max = true;
            break;
        }
        scanned += 1;
        let room = max - total_hits;
        let file = match grep_file(io, real, &matcher, context, room) {
            Ok(f) => f,
            Err(e) => return fail(format!("扫描 {} 失败：{}", display, e)),
        };
        if file.binary {
            skipped_binary += 1;
            continue;
        }
        lossy |= file.lossy;
        long_lines += file.long_lines;
        if file.hits == 0 {
            continue;
        }
        hit_files += 1;
        total_hits += file.hits.min(room);
        if file.hits > MAX_GREP_PER_FILE {
            per_file_cut = true;
        }
        if files_only {
            body.push(format!("{}（{} 行）", display, file.hits));
            continue;
        }
        if !body.is_empty() {
            body.push(String::new());
        }
        for (n, text, hit) in file.rows {
            let row = if hit {
                format!("{}:{}:{}", display, n, text)
            } else {
                format!("{}-{}-{}", display, n, text)
            };
            let cost = row.chars().count() + 1;
            if used + cost > body_budget {
                budget_cut = true;
                break;
            }
            used += cost;
            body.push(row);
        }
        if budget_cut {
            break;
        }
    }
    let mode = format!(
        "{}{}",
        if fixed { "按字面" } else { "正则" },
        if ignore_case {
            "、忽略大小写"
        } else {
            ""
        }
    );
    let glob_note = if globs.is_empty() {
        String::new()
    } else {
        format!("，glob {:?}", globs)
    };
    let mut out = format!(
        "[grep] {} 匹配「{}」（{}{}）",
        spec, pattern, mode, glob_note
    );
    if body.is_empty() {
        out.push_str("\n（没有命中行）");
    } else {
        out.push('\n');
        out.push_str(&body.join("\n"));
    }
    out.push_str(&format!(
        "\n命中 {} 行 / {} 个文件；扫描 {} 个文件（跳过 {} 个二进制、{} 个无读权限）",
        total_hits, hit_files, scanned, skipped_binary, denied
    ));
    if total_hits == 0 {
        out.push_str(
            "\n（被 .gitignore / .workignore 排除的文件不会出现在结果里；要连它们一起搜加 \"no_ignore\":true，隐藏文件加 \"hidden\":true）",
        );
    }
    if walked > MAX_GREP_FILES {
        out.push_str(&format!(
            "\n[截断] 目录里有 {} 个文件，只扫了前 {} 个；用 glob 或更小的 path 缩小范围。",
            walked, MAX_GREP_FILES
        ));
    }
    if per_file_cut {
        out.push_str(&format!(
            "\n[截断] 有文件命中超过 {} 行，多出的只计数没列出；单独搜那个文件或缩小 pattern。",
            MAX_GREP_PER_FILE
        ));
    }
    if stopped_at_max {
        out.push_str(&format!(
            "\n[截断] 已达到 max={} 条上限，后面的文件没有扫。",
            max
        ));
    }
    if budget_cut {
        out.push_str("\n[截断] 回执超过本次预算，只列出了前一部分。");
    }
    if long_lines > 0 {
        out.push_str(&format!(
            "\n[长行] {} 行超过 64 KiB，超出部分没有参与匹配。",
            long_lines
        ));
    }
    if lossy {
        out.push_str("\n[编码] 有文件含非法 UTF-8 字节，命中行按替换字符呈现。");
    }
    if no_ignore {
        out.push_str(
            "\n[过滤] 这次关掉了忽略规则与工作区过滤，.git 与 node_modules 之类也会被扫。",
        );
    }
    ToolOutcome::new(true, out)
}
