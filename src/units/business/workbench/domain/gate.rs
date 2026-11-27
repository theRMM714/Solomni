//! 目的：写入前的门禁口径——一份版本身份、要求观察到什么程度、拒绝原因与端口。
//! 管：`VersionRef` / `Need` / `GateFault` / `GateItem` / `WriteGate`。
//! 不管：账本从哪来、版本从哪取（实现方在 `agentloop` 的 `SessionWriteGate`，快照来自 `versions`）。
//! 联动：`tools` 的内置工具在真正落盘前经本端口判定；拒绝要带稳定错误码，供模型与 UI 分流。

/// 目的：写入门禁用的一份版本身份：只关心"这份内容是不是同一份"，不返回正文。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VersionRef {
    /// 目的：整份内容的 SHA-256（与 `ReadFact.blob` / `Change.hash` 同源）。
    pub hash: String,
    /// 目的：文件字节数；来自对象树的基线版本可能没有大小（门禁只用 hash）。
    pub size: Option<u64>,
}

/// 目的：写入门禁要求观察到的程度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// 目的：观察过该版本即可（edit 单处、patch Update、work_commit）。
    Observed,
    /// 目的：必须完整观察过该版本（write 覆盖、patch Add 覆盖、replace_all）。
    Full,
}

/// 目的：写入门禁拒绝的原因；带稳定错误码，供模型与 UI 区分。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateFault {
    /// 目的：当前上下文里没有这个版本的任何观察。
    NotObserved,
    /// 目的：观察到的是旧版本；当前版本已经变了。
    Stale {
        /// 目的：账本里那次的版本哈希。
        observed: String,
        /// 目的：磁盘上当前的版本哈希。
        current: String,
    },
    /// 目的：观察过，但只覆盖了部分行，而这次要求全文。
    NotFull {
        /// 目的：该版本的逻辑行数。
        total: u32,
    },
    /// 目的：当前内容存在，但没有权限/信息判断。
    Unavailable(String),
}

impl GateFault {
    /// 目的：稳定错误码。
    pub fn code(&self) -> &'static str {
        match self {
            GateFault::NotObserved => "FS_NOT_OBSERVED",
            GateFault::Stale { .. } => "FS_STALE_VERSION",
            GateFault::NotFull { .. } => "FS_NOT_FULLY_READ",
            GateFault::Unavailable(_) => "FS_GATE_UNAVAILABLE",
        }
    }

    /// 目的：给模型看的中文说明；保留现有测试依赖的关键短语。
    pub fn message(&self, path: &str) -> String {
        let body = match self {
            GateFault::NotObserved => format!("要先看过 {} 当前版本才能改；请 read 后再改。", path),
            GateFault::Stale { .. } => format!(
                "{} 在你读过之后又被改动过（内容与当时不同）：请重新 read 一遍再决定怎么写，不要凭记忆覆盖。",
                path
            ),
            GateFault::NotFull { .. } => format!(
                "这次要整份覆盖 {}，但你只读到一部分：整份覆盖会丢掉没读到的内容。请先完整 read 全文，或改用 edit 只改那一处。",
                path
            ),
            GateFault::Unavailable(why) => format!(
                "暂时无法确认 {} 的当前版本（{}）；请稍后重试，不要凭记忆覆盖。",
                path, why
            ),
        };
        format!("[{}] {}", self.code(), body)
    }
}

/// 目的：一条待判定的写入；`current` 是即将被替换/覆盖的磁盘版本。
#[derive(Clone, Copy)]
pub struct GateItem<'a> {
    /// 目的：工作树内的相对路径。
    pub path: &'a str,
    /// 目的：这次写入要求观察到什么程度。
    pub need: Need,
    /// 目的：即将被覆盖的磁盘版本；None = 文件不存在（或调用方没取到）。
    pub current: Option<&'a VersionRef>,
}

/// 目的：写入门禁端口；None = 不做门禁（内部写、直调测试）。
/// 约束：current = 即将被替换/覆盖的磁盘版本；None = 文件不存在。
pub trait WriteGate: Send + Sync {
    /// 目的：判断账本是否足以支撑这次写入；`current` 是即将被覆盖的磁盘版本。
    fn check(&self, path: &str, need: Need, current: Option<&VersionRef>) -> Result<(), GateFault>;

    /// 目的：一次判定多条写入（同一批补丁只取一次工作区快照）；返回第一条失败的序号与原因。
    /// 约束：默认逐条调用 `check`；实现方可以合并快照，但不能改变单条的判定语义。
    fn check_many(&self, items: &[GateItem<'_>]) -> Result<(), (usize, GateFault)> {
        for (i, item) in items.iter().enumerate() {
            if let Err(f) = self.check(item.path, item.need, item.current) {
                return Err((i, f));
            }
        }
        Ok(())
    }
}
