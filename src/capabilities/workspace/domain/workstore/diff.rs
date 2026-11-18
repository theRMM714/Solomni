//! 目的：工作区两条线之间的行级差异——请求 / 报告形状与 Myers 差异算法。
//! 管：`DiffRequest` / `DiffReport` / `DiffEntry` / `Hunk` / `DiffLine` 的纯数据，`diff_text` 的行级比较与 hunk 归并。
//! 不管：取内容（走 `WorkStore`）、权限与回执文案（在 `service` 与 `work_tools`）。
//! 联动：`service/diff.rs` 取两侧树与对象、`domain/work_tools.rs` 渲染。

use super::ChangeKind;

/// 目的：单文件参与行级比较的字节上限；超过只报大小与指纹，不展开 hunk。
pub const DIFF_MAX_BYTES: u64 = 1024 * 1024;
/// 一个 hunk 前后各带几行上下文（与 git 默认一致）。
const CONTEXT: usize = 3;

/// 目的：一次 `work_diff` 的请求：`from` 缺省为工作线头，`to` 缺省为沙箱；`paths` 为空 = 全部。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffRequest {
    /// 目的：起始提交号（缺省 = 该 agent 的工作线头）。
    pub from: Option<u64>,
    /// 目的：目标提交号（缺省 = 沙箱当前内容）。
    pub to: Option<u64>,
    /// 目的：只比这些路径（工作树相对路径）。
    pub paths: Vec<String>,
}

/// 目的：一行差异的记号：上下文 / 删除 / 新增。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffSign {
    /// 两边都有这一行。
    Context,
    /// 只在起始侧。
    Remove,
    /// 只在目标侧。
    Add,
}

/// 目的：一行差异：记号 + 行正文（不含行尾换行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub sign: DiffSign,
    pub text: String,
}

/// 目的：一段 hunk：两侧起始行与行数（1 起数，0 行表示纯新增 / 纯删除的落点）+ 行内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u64,
    pub old_len: u64,
    pub new_start: u64,
    pub new_len: u64,
    pub lines: Vec<DiffLine>,
}

/// 目的：一条路径的差异：路径 + 变更类型 + 是否二进制 / 过大 + hunk 列表。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffEntry {
    pub path: String,
    pub kind: ChangeKind,
    pub binary: bool,
    pub too_large: bool,
    pub hunks: Vec<Hunk>,
}

/// 目的：一次比较的结果：两侧标签 + 逐路径差异；`truncated` = 行级内容超出预算被截。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffReport {
    pub from: String,
    pub to: String,
    pub entries: Vec<DiffEntry>,
    pub truncated: bool,
}

/// 目的：行级差异（文本 → hunk 列表）；空文件与尾部换行都不产生空行差异。
pub fn diff_text(old: &str, new: &str) -> Vec<Hunk> {
    let a = split_lines(old);
    let b = split_lines(new);
    hunks(&diff_ops(&a, &b), &a, &b)
}

/// 目的：拆行；末尾换行不算一行，空文件 = 零行。
fn split_lines(s: &str) -> Vec<&str> {
    if s.is_empty() {
        return Vec::new();
    }
    s.strip_suffix('\n').unwrap_or(s).split('\n').collect()
}

#[derive(Debug, Clone, Copy)]
enum Op {
    Eq(usize),
    Del(usize),
    Ins(usize),
}

/// 目的：Myers 最短编辑脚本（按行比较）；返回按先后顺序的等 / 删 / 增操作。
fn diff_ops(a: &[&str], b: &[&str]) -> Vec<Op> {
    if a.is_empty() {
        return (0..b.len()).map(Op::Ins).collect();
    }
    if b.is_empty() {
        return (0..a.len()).map(Op::Del).collect();
    }
    let (n, m) = (a.len() as isize, b.len() as isize);
    let max = n + m;
    let offset = max;
    let mut v = vec![0isize; (2 * max + 1) as usize];
    let mut trace: Vec<Vec<isize>> = Vec::new();
    for d in 0..=max {
        trace.push(v.clone());
        let mut k = -d;
        while k <= d {
            let idx = (k + offset) as usize;
            let mut x = if k == -d || (k != d && v[idx - 1] < v[idx + 1]) {
                v[idx + 1]
            } else {
                v[idx - 1] + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[idx] = x;
            if x >= n && y >= m {
                return backtrack(&trace, d, offset, n, m);
            }
            k += 2;
        }
    }
    // 上面的循环在 d = n + m 时必然命中终点。
    Vec::new()
}

/// 目的：沿 trace 从终点倒推编辑脚本，再反过来给出正序。
fn backtrack(trace: &[Vec<isize>], d_end: isize, offset: isize, n: isize, m: isize) -> Vec<Op> {
    let (mut x, mut y) = (n, m);
    let mut ops = Vec::new();
    for d in (0..=d_end).rev() {
        let v = &trace[d as usize];
        let k = x - y;
        let (prev_x, prev_y) = if d == 0 {
            (0, 0)
        } else if k == -d || (k != d && v[(k - 1 + offset) as usize] < v[(k + 1 + offset) as usize])
        {
            let pk = k + 1;
            let px = v[(pk + offset) as usize];
            (px, px - pk)
        } else {
            let pk = k - 1;
            let px = v[(pk + offset) as usize];
            (px, px - pk)
        };
        while x > prev_x && y > prev_y {
            x -= 1;
            y -= 1;
            ops.push(Op::Eq(x as usize));
        }
        if d > 0 {
            if x == prev_x {
                ops.push(Op::Ins((y - 1) as usize));
                y -= 1;
            } else {
                ops.push(Op::Del((x - 1) as usize));
                x -= 1;
            }
        }
    }
    ops.reverse();
    ops
}

/// 目的：把编辑脚本切成 hunk（前后各带 `CONTEXT` 行；相隔不超过两倍上下文的改动并进同一段）。
fn hunks(ops: &[Op], a: &[&str], b: &[&str]) -> Vec<Hunk> {
    let mut changed = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        if !matches!(op, Op::Eq(_)) {
            changed.push(i);
        }
    }
    if changed.is_empty() {
        return Vec::new();
    }
    let mut groups: Vec<(usize, usize)> = Vec::new();
    for i in changed {
        match groups.last_mut() {
            Some((_, end)) if i <= *end + 2 * CONTEXT => *end = i,
            _ => groups.push((i, i)),
        }
    }
    let mut out = Vec::new();
    for (first, last) in groups {
        let start = first.saturating_sub(CONTEXT);
        let end = (last + CONTEXT).min(ops.len() - 1);
        let mut lines = Vec::new();
        for op in &ops[start..=end] {
            lines.push(match *op {
                Op::Eq(i) => DiffLine {
                    sign: DiffSign::Context,
                    text: a[i].to_string(),
                },
                Op::Del(i) => DiffLine {
                    sign: DiffSign::Remove,
                    text: a[i].to_string(),
                },
                Op::Ins(i) => DiffLine {
                    sign: DiffSign::Add,
                    text: b[i].to_string(),
                },
            });
        }
        let old_before = ops[..start]
            .iter()
            .filter(|o| matches!(o, Op::Eq(_) | Op::Del(_)))
            .count();
        let new_before = ops[..start]
            .iter()
            .filter(|o| matches!(o, Op::Eq(_) | Op::Ins(_)))
            .count();
        let old_len = lines.iter().filter(|l| l.sign != DiffSign::Add).count();
        let new_len = lines.iter().filter(|l| l.sign != DiffSign::Remove).count();
        out.push(Hunk {
            old_start: if old_len == 0 {
                old_before as u64
            } else {
                old_before as u64 + 1
            },
            old_len: old_len as u64,
            new_start: if new_len == 0 {
                new_before as u64
            } else {
                new_before as u64 + 1
            },
            new_len: new_len as u64,
            lines,
        });
    }
    out
}
