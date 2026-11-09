//! 目的：存储空间的入站能力面——空间报告、回收、显式清理、作业与策略。
//! 管：Storage trait 与 DTO（SpaceReport / GcReport / CleanRequest / JobState）、分类 id（CAT_*）、未接入的 NoStorage。
//! 不管：怎么扫描（service/detail）、怎么删（service）；工作对象回收仍在 workspace。
//! 联动：Ops 持它；Web / CLI / builds 经这里拿事实与动作。

use serde::{Deserialize, Serialize};

/// 目的：空间分类 id——盘点产出、策略排序与删除三处共用的线格式。
pub const CAT_BUILD: &str = "build-cache";
pub const CAT_DOWNLOAD: &str = "download-cache";
pub const CAT_OVERLAYS: &str = "overlays";
pub const CAT_LOGS: &str = "logs";
pub const CAT_GUESTS: &str = "guest-bases";
pub const CAT_SESSIONS: &str = "sessions";
pub const CAT_OUTPUTS: &str = "module-outputs";

/// 目的：可回收缓存的空间预算与保留策略（settings.yaml 的 storage 段的运行形态）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoragePolicy {
    pub periodic_gc: bool,
    /// 目的：全局自动回收间隔（分钟，至少 15）。
    pub gc_interval_minutes: u64,
    /// 目的：可回收缓存总预算（GiB；0 = 自动）。
    pub cache_cap_gib: u64,
    pub cache_keep_days: u64,
    pub build_volume_gib: u64,
    pub min_free_gib: u64,
    pub log_keep_days: u64,
    /// 目的：按上限实际采用的字节数（cache_cap_gib=0 时按磁盘 10% 夹 10–50 GiB）。
    pub cap_bytes: u64,
}

impl From<&crate::capabilities::registry::api::StorageSettings> for StoragePolicy {
    fn from(s: &crate::capabilities::registry::api::StorageSettings) -> Self {
        StoragePolicy {
            periodic_gc: s.periodic_gc,
            gc_interval_minutes: s.gc_interval_minutes,
            cache_cap_gib: s.cache_cap_gib,
            cache_keep_days: s.cache_keep_days,
            build_volume_gib: s.build_volume_gib,
            min_free_gib: s.min_free_gib,
            log_keep_days: s.log_keep_days,
            cap_bytes: s.cache_cap_gib.saturating_mul(1024 * 1024 * 1024),
        }
    }
}

impl Default for StoragePolicy {
    fn default() -> Self {
        StoragePolicy {
            periodic_gc: true,
            gc_interval_minutes: 15,
            cache_cap_gib: 20,
            cache_keep_days: 30,
            build_volume_gib: 16,
            min_free_gib: 2,
            log_keep_days: 14,
            cap_bytes: 20 * 1024 * 1024 * 1024,
        }
    }
}

/// 目的：一块可回收数据（一块卷、一个缓存文件、一个过期可写层、一份日志）。
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SpaceItem {
    pub id: String,
    pub label: String,
    /// 目的：实际占用（qcow2 取 actual-size；目录按文件长度求和）。
    pub bytes: u64,
    /// 目的：虚拟大小（qcow2 的 virtual-size；不适用为 None）。
    pub virtual_bytes: Option<u64>,
    /// 目的：最近使用（unix 秒；未知为 None）。
    pub last_used: Option<u64>,
    /// 目的：有构建锁 / 活跃 guest / 有会话在用时受保护。
    pub active: bool,
    pub detail: String,
    /// 目的：构建缓存卷里的模块 id（没有元数据时为空）。
    pub modules: Vec<String>,
    /// 目的：这一项是否允许显式清理（虚拟根/会话为 false）。
    pub cleanable: bool,
}

/// 目的：一类可回收数据（构建缓存 / 下载缓存 / 过期可写层 / 日志 / 环境组件 / 用户数据）。
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SpaceCategory {
    pub id: String,
    pub label: String,
    pub bytes: u64,
    pub items: Vec<SpaceItem>,
    /// 目的：是否进自动回收与上限；环境组件/用户数据为 false。
    pub auto_evictable: bool,
    pub note: String,
}

/// 目的：上次回收的摘要（给界面与 doctor 一行事实）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LastGc {
    pub at: u64,
    pub freed_bytes: u64,
    pub reason: String,
    pub skipped: usize,
}

/// 目的：一次空间盘点。
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SpaceReport {
    pub generated_at: u64,
    /// 目的：自动回收口径下的总占用。
    pub managed_bytes: u64,
    pub cap_bytes: u64,
    pub free_bytes: u64,
    pub total_bytes: u64,
    pub over_cap: bool,
    pub categories: Vec<SpaceCategory>,
    pub policy: StoragePolicy,
    pub last_gc: Option<LastGc>,
    pub job: JobState,
}

/// 目的：一条回收动作的事实（预览与执行共用）。
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GcAction {
    pub category: String,
    pub id: String,
    pub label: String,
    pub bytes: u64,
    pub reason: String,
    pub applied: bool,
    pub error: Option<String>,
}

/// 目的：一次回收的结果。
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GcReport {
    pub applied: bool,
    pub before_bytes: u64,
    pub after_bytes: u64,
    pub freed_bytes: u64,
    pub over_cap: bool,
    pub actions: Vec<GcAction>,
    pub skipped: Vec<GcAction>,
}

/// 目的：显式清理的目标类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanKind {
    BuildCache,
    DownloadCache,
    Overlays,
    Logs,
    Compact,
    GuestReset,
}

impl CleanKind {
    /// 目的：线格式（Web 请求与 CLI 共用）→ 类别；未知值如实拒绝。
    pub fn parse(s: &str) -> Result<CleanKind, String> {
        match s {
            "build-cache" => Ok(CleanKind::BuildCache),
            "download-cache" => Ok(CleanKind::DownloadCache),
            "overlays" => Ok(CleanKind::Overlays),
            "logs" => Ok(CleanKind::Logs),
            "compact" => Ok(CleanKind::Compact),
            "guest-reset" => Ok(CleanKind::GuestReset),
            other => Err(format!("未知的清理类别：{}", other)),
        }
    }
}

/// 目的：一次显式清理请求。
#[derive(Debug, Clone)]
pub struct CleanRequest {
    pub kind: CleanKind,
    /// 目的：目标 id：构建缓存 = 虚拟根名（可空 = 全部）；可写层 = 租约名（可空 = 全部过期）；还原 = 虚拟根名。
    pub id: Option<String>,
    /// 目的：构建缓存按模块清理时的模块 id。
    pub module: Option<String>,
    /// 目的：没有它一律不动手（Web/CLI 的二次确认落到这里）。
    pub confirm: bool,
}

impl Default for CleanRequest {
    fn default() -> Self {
        CleanRequest {
            kind: CleanKind::BuildCache,
            id: None,
            module: None,
            confirm: false,
        }
    }
}

/// 目的：显式清理的结果：同步报告，或已启动的作业（虚拟根还原）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CleanOutcome {
    Report(GcReport),
    Started(JobState),
}

/// 目的：空间作业进度（目前只有虚拟根还原；GC 是同步的）。
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct JobState {
    pub running: bool,
    pub seq: u64,
    pub lines: Vec<String>,
    pub error: Option<String>,
}

/// 目的：存储空间的唯一入站面。
pub trait Storage: Send + Sync {
    /// 目的：盘点当前空间；失败如实返回。
    fn report(&self) -> Result<SpaceReport, String>;
    /// 目的：按策略回收一次；apply=false 只预览，不删。
    fn gc(&self, apply: bool) -> Result<GcReport, String>;
    /// 目的：构建 / 起 guest 前检查预算与宿主余量；到点才回收一次，否则空报告。
    /// 参数：protect = 本次要用的虚拟根名（其卷排最后，尽量不先删它）。
    fn ensure_headroom(&self, protect: Option<&str>) -> Result<GcReport, String>;
    /// 目的：显式清理/还原；confirm=false 如实拒绝。
    fn clean(&self, req: &CleanRequest) -> Result<CleanOutcome, String>;
    /// 目的：当前空间作业快照。
    fn job(&self) -> JobState;
    /// 目的：更新策略（设置保存后由呈现层推入；不落盘）。
    fn set_policy(&self, policy: StoragePolicy);
    /// 目的：当前策略。
    fn policy(&self) -> StoragePolicy;
}

/// 目的：未接入存储能力时的空实现（测试装配与默认值）。
pub struct NoStorage;

impl Storage for NoStorage {
    fn report(&self) -> Result<SpaceReport, String> {
        Ok(SpaceReport::default())
    }
    fn gc(&self, _apply: bool) -> Result<GcReport, String> {
        Err("存储能力未接入".to_string())
    }
    fn ensure_headroom(&self, _protect: Option<&str>) -> Result<GcReport, String> {
        Ok(GcReport::default())
    }
    fn clean(&self, _req: &CleanRequest) -> Result<CleanOutcome, String> {
        Err("存储能力未接入".to_string())
    }
    fn job(&self) -> JobState {
        JobState::default()
    }
    fn set_policy(&self, _policy: StoragePolicy) {}
    fn policy(&self) -> StoragePolicy {
        StoragePolicy::default()
    }
}
