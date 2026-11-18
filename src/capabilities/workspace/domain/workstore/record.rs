//! 目的：版本库的记录形态——提交、引用、改动、待恢复事务、内容对象与回收报告。
//! 管：`Commit` / `Ref` / `Change` / `Index` / `Tree` / `Pending` / `ObjectInfo` / `WorkGcReport` / `CommitSummary` 这些落盘与回执共用的数据结构。
//! 不管：三方比较与判定（`merge.rs` / `resolve.rs`）、路径校验（`path.rs`）、状态展示（`status.rs`）。
//! 联动：落盘由 `workspace::detail::fs_workstore` 消费，能力面经 `workspace::api` 重导出（docs/workspace/README.md）。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 目的：每个 agent 的拉取基线：共享区相对路径 → 当时的内容指纹。
pub type Index = BTreeMap<String, String>;
/// 目的：一个提交点的整棵树：共享区相对路径 → 内容指纹。
pub type Tree = BTreeMap<String, String>;

/// 目的：一次改动是新增、修改还是删除。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Add,
    Modify,
    Delete,
}

/// 目的：提交记录里的一条改动。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub path: String,
    pub kind: ChangeKind,
    /// 目的：新内容的指纹；删除为空串。
    #[serde(default)]
    pub hash: String,
}

/// 目的：提交锚：这个提交点是**哪个会话（agent 实例名）的哪一行**触发的。
///    **空 agent = 主会话/用户投喂**。回档按它把共享区物化回某一行那一刻。
/// 约束：会话名必须一起记——同名 agent 在不同会话里的行号各自从 1 数，只按 agent 比会把两条转录串起来。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitAnchor {
    #[serde(default)]
    pub agent: String,
    #[serde(default)]
    pub session: String,
    #[serde(default)]
    pub line: u64,
}

/// 目的：这条提交落在哪条线上——主线只收发布，存档线只收沙箱快照（不进共享区）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitLine {
    /// 目的：主线（共享区的唯一更新途径）。
    #[default]
    Main,
    /// 目的：该 agent 的工作线 `refs/agents/<agent>`（只由 work_commit 推进，不进主线和共享区）。
    Work,
    /// 目的：该 agent 的存档线 `refs/agents/<agent>/saves`。
    Save,
}

/// 目的：标记这条提交是谁、因为什么写下的。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// 目的：工具写完后的自动提交。
    Tool,
    /// 目的：从主副本拉进沙箱（不改提交线）。
    Pull,
    /// 目的：采纳磁盘上的外部改动。
    External,
    /// 目的：用户投喂或界面写入。
    #[default]
    User,
    /// 目的：留档检查点。
    Archive,
    /// 目的：切换到非祖先提交。
    Checkout,
}

/// 目的：一个提交点（提交记录 + 该点的整棵树）。
/// 约束：树按"根 + 增量"落盘——`tree` 只在根提交上是 `Some`，其余提交只记 `changes`，
///    读树时沿 `parent` 回到最近的根再生；两种形态都能读，旧提交（带整棵树）天然是根。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    pub id: u64,
    pub parent: Option<u64>,
    /// 目的：分支分叉点——这条提交线从哪个提交分出来（主线提交 = parent）。
    #[serde(default)]
    pub base: u64,
    /// 目的：作者：agent 实例名，或 `user`（界面投喂）。
    pub author: String,
    /// 目的：提交所属会话（界面投喂为空串）。
    #[serde(default)]
    pub session: String,
    pub time: i64,
    pub message: String,
    pub changes: Vec<Change>,
    /// 目的：整棵树；只在根提交上落盘，增量提交为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<Tree>,
    /// 目的：到最近根提交的步数；根 = 0。
    #[serde(default)]
    pub depth: u32,
    /// 目的：写下这条提交的来源。
    #[serde(default)]
    pub source: Source,
    /// 目的：这条提交属于哪条线——只有主线提交进共享区与共享区历史。
    #[serde(default)]
    pub line: CommitLine,
    /// 目的：这条主线提交 squash 了工作线的哪些提交；work_log 据此判定「已推送」。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<u64>,
    /// 目的：转录行锚（没有锚 = None，按"总在回档点之前"保守处理）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<CommitAnchor>,
}

/// 目的：引用的种类——主线、agent 分支、留档检查点、活动线钉子。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    /// 目的：主线；所有提交都从它可达才算活。
    #[default]
    Main,
    /// 目的：一个 agent 的提交线。
    Agent,
    /// 目的：留档检查点（连续留档各占一个 ref，不互相覆盖）。
    Archive,
    /// 目的：活动线钉子——回档 / 切枝前把当前线钉住。
    Live,
    /// 目的：其它具名线。
    Other,
}

/// 目的：一条命名引用——`refs/main`（主线）、`refs/agents/<agent>`（该 agent 的发布线）、
///   `refs/agents/<agent>/saves`（该 agent 的存档线）、`refs/archive/<id>`、`refs/live/<id>`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ref {
    #[serde(default)]
    pub kind: RefKind,
    /// 目的：这条线当前指向的提交；空 = 线已建但还没有提交。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<u64>,
    /// 目的：每条线上各 agent 的 head（提交线分叉时用）。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub heads: BTreeMap<String, Option<u64>>,
    /// 目的：留档时各 agent 存档线的头（`refs/archive/<id>` 专用）——恢复要把它连同共享区一起展回来。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub saves: BTreeMap<String, Option<u64>>,
}

/// 目的：一个 agent 的基线与 agent 名的配对，供待恢复事务写回。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingIndex {
    /// 目的：agent 实例名。
    pub agent: String,
    /// 目的：这次提交后的新基线。
    pub index: Index,
}

/// 目的：版本库写事务的待恢复日志；一个工作同时最多一条。
/// 约束：重放必须幂等；成功后由恢复方删除 pending。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum Pending {
    /// 目的：一次提交（主副本改动 + 可选基线更新）。
    Commit {
        /// 目的：已经算好的提交记录（box 住，避免 pending 枚举被整棵提交树撑大）。
        commit: Box<Commit>,
        /// 目的：提交后的 agent 基线；用户投喂与存档提交为 None。
        index: Option<PendingIndex>,
        /// 目的：这条提交落在哪条线上（旧日志没有该字段 —— 那时只有主线）。
        #[serde(default)]
        line: CommitLine,
    },
    /// 目的：把主副本物化到某个提交点（None = 清空）。
    Restore {
        /// 目的：目标提交号。
        target: Option<u64>,
    },
    /// 目的：丢弃 keep 之外的提交，并让主副本与列出的沙箱一起落到 keep。
    Discard {
        /// 目的：要保留的提交点。
        keep: Option<u64>,
        /// 目的：要一起回退的 agent 沙箱（空 = 只动主副本，测试用的低层语义）。
        #[serde(default)]
        agents: Vec<String>,
        /// 目的：这些沙箱各自要落到哪条存档提交（None = 空沙箱）。
        #[serde(default)]
        saves: BTreeMap<String, Option<u64>>,
    },
}

/// 目的：一个内容寻址对象的事实（对象回收用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectInfo {
    /// 目的：内容指纹（文件名）。
    pub hash: String,
    /// 目的：对象字节数。
    pub size: u64,
}

/// 目的：一次对象回收的报告；apply=false 时只统计不删除。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkGcReport {
    /// 目的：是否真的执行了删除。
    pub applied: bool,
    /// 目的：对象总数。
    pub objects: usize,
    /// 目的：对象总字节。
    pub bytes: u64,
    /// 目的：可达对象数。
    pub reachable_objects: usize,
    /// 目的：可达对象字节。
    pub reachable_bytes: u64,
    /// 目的：不可达对象数。
    pub unreachable_objects: usize,
    /// 目的：不可达对象字节。
    pub unreachable_bytes: u64,
    /// 目的：本次实际删除的对象数。
    pub removed_objects: usize,
    /// 目的：本次实际删除的字节。
    pub removed_bytes: u64,
    /// 目的：对象目录里不是内容指纹的杂项文件数（临时文件等）。
    pub ignored_files: usize,
    /// 目的：本次实际清掉的杂项文件数。
    pub removed_ignored: usize,
    /// 目的：本次实际删除的提交记录数。
    pub removed_commits: usize,
}

/// 目的：提交点摘要（work_status 用）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommitSummary {
    pub id: u64,
    pub author: String,
    pub time: i64,
    pub message: String,
}

impl From<&Commit> for CommitSummary {
    fn from(c: &Commit) -> CommitSummary {
        CommitSummary {
            id: c.id,
            author: c.author.clone(),
            time: c.time,
            message: c.message.clone(),
        }
    }
}
