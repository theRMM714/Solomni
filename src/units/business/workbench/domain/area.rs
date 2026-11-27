//! 目的：一次寻址落在哪个区——共享区 / 私有沙箱 / 成员模块目录。
//! 管：`Area` 三个变体；它是读写判定与回执文案的落点口径。
//! 不管：这些根在哪（在 `Bench`）、谁能写谁（在 `Bench` 的方法里）。
//! 联动：`Bench::resolve` 产出它；`tools` 的读写落点与 `versions` 的工作副本都按它分流。

/// 目的：寻址落点（用于回执文案与越界提示）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Area {
    /// 本工作共享区。
    Shared,
    /// 本 agent 私有沙箱。
    Private,
    /// 本 agent 的成员模块目录。
    Module(String),
}
