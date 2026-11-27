//! 目的：观察事实的契约形状——「某个路径的某个整份版本，被看过哪些行」，是写入门禁唯一的证据口径。
//! 管：`LineRange` / `Coverage` / `ObservationSource` / `ObservationFact`（含到 `ReadFact` 的还原）。
//! 不管：账本（`tools` 的 `Observations` 记它）；门禁判定（`workbench` 的 `WriteGate` 实现方）。
//! 联动：`tools` 记事实、`versions` 的门禁按它判、`agentloop` 把批前快照交给门禁。

use crate::units::business::workbench::domain::readfact::ReadFact;
/// 目的：一个从 1 开始、闭区间的行范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LineRange {
    /// 目的：起点行号（1 起）。
    pub from: u32,
    /// 目的：终点行号（1 起、含此行）。
    pub to: u32,
}

impl LineRange {
    /// 目的：按端点造一个闭区间。
    pub fn new(from: u32, to: u32) -> LineRange {
        LineRange { from, to }
    }
}

/// 目的：这次观察覆盖了文件的哪些行。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Coverage {
    /// 目的：整份原文都看过（完整读、整份写、patch Add）。
    Full,
    /// 目的：只看过这些行（分段读、edit、patch Update）。
    Ranges(Vec<LineRange>),
}

impl Coverage {
    /// 目的：把一组行区间排序、去重、合并相邻与重叠区间。
    /// 约束：只保留 from <= to 的合法区间；相邻区间合并（1-2 + 3-4 = 1-4）不改变覆盖语义。
    pub fn normalized(&self) -> Coverage {
        match self {
            Coverage::Full => Coverage::Full,
            Coverage::Ranges(ranges) => Coverage::Ranges(normalize_ranges(ranges.iter().copied())),
        }
    }

    /// 目的：两次观察的覆盖并集；任一方为 Full 则结果 Full。
    pub fn union(&self, other: &Coverage) -> Coverage {
        match (self, other) {
            (Coverage::Full, _) | (_, Coverage::Full) => Coverage::Full,
            (Coverage::Ranges(a), Coverage::Ranges(b)) => {
                Coverage::Ranges(normalize_ranges(a.iter().chain(b).copied()))
            }
        }
    }

    /// 目的：行区间并集是否覆盖 1..=total；Full 直接为真。
    /// 约束：空文件（total == 0）只有 Full 才能覆盖；区间不能凭空覆盖不存在的行。
    pub fn covers_all(&self, total: u32) -> bool {
        match self {
            Coverage::Full => true,
            Coverage::Ranges(ranges) => {
                if total == 0 {
                    return false;
                }
                let sorted: Vec<LineRange> = normalize_ranges(ranges.iter().copied())
                    .into_iter()
                    .filter(|r| r.from <= total)
                    .map(|r| LineRange::new(r.from, r.to.min(total)))
                    .collect();
                let mut next = 1u32;
                for r in sorted {
                    if r.from > next {
                        return false;
                    }
                    if r.to >= next {
                        next = r.to.saturating_add(1);
                    }
                    if next > total {
                        return true;
                    }
                }
                next > total
            }
        }
    }
}

/// 目的：排序、去重并合并相邻/重叠的行区间；输入里的空区间（from > to）直接丢弃。
pub fn normalize_ranges(ranges: impl IntoIterator<Item = LineRange>) -> Vec<LineRange> {
    let mut sorted: Vec<LineRange> = ranges.into_iter().filter(|r| r.from <= r.to).collect();
    sorted.sort_by_key(|r| (r.from, r.to));
    let mut out: Vec<LineRange> = Vec::with_capacity(sorted.len());
    for r in sorted {
        match out.last_mut() {
            Some(last) if r.from <= last.to.saturating_add(1) => {
                last.to = last.to.max(r.to);
            }
            _ => out.push(r),
        }
    }
    out
}

/// 目的：观察来源，只用于诊断与显示，不参与授权判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ObservationSource {
    /// 目的：完整或分段 read。
    Read,
    /// 目的：整份 write。
    Write,
    /// 目的：单处或多处 edit。
    Edit,
    /// 目的：patch 的 Add 块。
    PatchAdd,
    /// 目的：patch 的 Update 块。
    PatchUpdate,
    /// 目的：work_diff 的差异。
    Diff,
}

/// 目的：观察事实的持久化格式版本；与代码不符的事实一律降级为“没有观察”（失败关闭）。
pub const OBSERVATION_FORMAT: u32 = 1;

/// 目的：一条观察事实——某个路径的某个整份版本，被看过哪些行。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ObservationFact {
    /// 目的：持久化格式版本；旧转录/损坏数据缺省为 0，门禁不认。
    #[serde(default)]
    pub schema: u32,
    /// 目的：这条观察来自哪一次模型回复（= 该回复第一行的行 id）；压缩按它丢弃失效观察。
    pub reply: u64,
    /// 目的：工作树内的相对路径（斜杠分隔）。
    pub path: String,
    /// 目的：空 = 主副本；否则 = agent 沙箱名。
    pub worktree: String,
    /// 目的：整份内容的 SHA-256 十六进制串；必须与 ReadFact.blob / Change.hash 同算法。
    pub version: String,
    /// 目的：这条事实覆盖了该版本的哪些行。
    pub coverage: Coverage,
    /// 目的：该版本的逻辑行数；区间覆盖是否等于全文时用。
    pub total_lines: u32,
    /// 目的：这条事实由哪种操作产生（诊断与显示）。
    pub source: ObservationSource,
}

impl ObservationFact {
    /// 目的：把这条观察还原成 freshness 用的读取事实：整份读 = 一条，分段读 = 每段一条。
    /// 约束：from/to 只落在该版本确实有内容的行上（空文件、越界段不产生）；blob 与版本同一指纹。
    pub fn read_facts(&self) -> Vec<ReadFact> {
        if self.schema != OBSERVATION_FORMAT || self.source != ObservationSource::Read {
            return Vec::new();
        }
        let total = u64::from(self.total_lines);
        let mut out = Vec::new();
        let mut push = |from: u64, to: u64| {
            if total == 0 || from == 0 || from > to || from > total {
                return;
            }
            out.push(ReadFact {
                worktree: self.worktree.clone(),
                path: self.path.clone(),
                from,
                to: to.min(total),
                blob: self.version.clone(),
            });
        };
        match &self.coverage {
            Coverage::Full => push(1, total),
            Coverage::Ranges(ranges) => {
                for r in ranges {
                    push(u64::from(r.from), u64::from(r.to));
                }
            }
        }
        out
    }
}
