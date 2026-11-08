//! 目的：一次 `read` 的读取事实：哪个工作树、哪个路径、哪几行、当时的整份内容指纹。
//! 管：事实的形状与随工具行落盘。
//!   只记"读的时候"的样子，不记"过期"这个状态——过期是派生的。
//! 不管：过期怎么判（domain/freshness.rs 的算法 + workspace 服务的 work_stale_reads）；
//!   占位符怎么换（collab 服务 driver.rs 的 send_view）。
//! 联动：随 ToolCallView.read 落进转录行；重建时由 conductor 的 replay_reads 装回读台账。

/// 目的：一条读取事实（由内置 `read` 产生，随工具行保存）。
/// 约束：`blob` 是整份内容的指纹（内容寻址对象库的键）；读到的不是完整原文时不产生事实——
///   半份内容无法与后续版本做块对齐，冒然判定只会误导模型。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReadFact {
    /// 目的：读的是哪个工作树（agent 沙箱名）；空 = 主副本。
    pub worktree: String,
    /// 目的：工作树内的相对路径（斜杠分隔）。
    pub path: String,
    /// 目的：读到的第一行（从 1 数起）。
    pub from: u64,
    /// 目的：读到的最后一行（含）。
    pub to: u64,
    /// 目的：读的那一刻整份内容的指纹。
    pub blob: String,
}
