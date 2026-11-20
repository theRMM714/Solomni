//! 目的：系统维护与自检能力的入站面——启动相命令要的事实、请求与结果形状。
//! 管：FenceFacts、各类请求、System 用例 trait 与工具链事实的渲染。
//! 不管：怎么回收 / 怎么补齐（在 service 与各能力）；命令语法与回执文案（在呈现层）。
//! 联动：presentation/cli/boot.rs 与 Web 的维护入口经它执行；事实来自 storage / provision / builds / workspace / vm。
//!   作业进度快照 `JobState` 是 kernel 的共享形状，这里只转出给呈现层。

use serde_json::Value;

use crate::capabilities::catalog::packages::api::BaseFacts;
use crate::capabilities::maintenance::builds::api::Built;
use crate::capabilities::maintenance::storage::api::{CleanOutcome, GcReport, Storage};
use crate::capabilities::provision::api::Provision;
use crate::capabilities::workspace::api::WorkGcReport;
pub use crate::kernel::api::JobState;

/// 目的：基础工具链 profile——与清单 toolchain 条目同一组（自检与启动报告共用）。
pub const TOOLCHAIN_PROFILE: [&str; 3] = ["python", "node", "cc"];

/// 目的：本机围栏能力的事实——生产取一次 `capability()`，用例注入合成值免得再跑一次自检。
#[derive(Debug, Clone)]
pub struct FenceFacts {
    pub fs: bool,
    pub net: bool,
    pub tree: bool,
    pub note: String,
}

/// 目的：自检里要直读宿主的那几项事实——能力按端口声明，宿主探针在 detail 侧实现，由能力入口装进来。
pub trait Facts: Send + Sync {
    /// 目的：本机围栏能力（生产路径只取一次）。
    fn fence(&self) -> FenceFacts;
    /// 目的：按给定围栏事实出一份自检报告（字段见 docs/system 的「自检报告」）。
    fn doctor(
        &self,
        root: &std::path::Path,
        provision: &dyn Provision,
        storage: &dyn Storage,
        toolchains: &BaseFacts,
        fence: FenceFacts,
    ) -> Value;
}

/// 目的：一次回收请求（`--gc`）：类别缺省 = 按策略回收，给了类别 = 显式清理。
#[derive(Debug, Clone, Default)]
pub struct GcRequest {
    /// 目的：清理类别（build-cache / download-cache / overlays / logs / compact / guest-reset）。
    pub kind: Option<String>,
    /// 目的：目标 id：构建缓存与还原是虚拟根名，可写层是租约名。
    pub guest: Option<String>,
    /// 目的：构建缓存按模块清理时的模块 id。
    pub module: Option<String>,
    /// 目的：真的动手；缺省只预览。
    pub confirm: bool,
}

/// 目的：一次补齐请求（`--provision --ensure`）。
#[derive(Debug, Clone)]
pub struct EnsureRequest {
    /// 目的：补齐规格（例如 toolchain）；缺省 = 本机全部。
    pub spec: Option<String>,
    pub confirm: bool,
}

/// 目的：一次虚拟根还原请求（`--provision --reset`）。
#[derive(Debug, Clone)]
pub struct ResetRequest {
    pub guest: String,
    pub confirm: bool,
}

/// 目的：一次工作区回收请求（`--work-gc`）。
#[derive(Debug, Clone)]
pub struct WorkGcRequest {
    pub work: String,
    /// 目的：真的删；缺省只预览。
    pub confirm: bool,
}

/// 目的：一次构建请求（`--build`）。
#[derive(Debug, Clone)]
pub struct BuildRequest {
    pub module: String,
    /// 目的：目标虚拟机档名；缺省 = 宿主。
    pub guest: Option<String>,
}

/// 目的：便携维护（`--setup`）的模式——PATH shim 的写/摘/还原、只读状态、围栏机器级预授。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupMode {
    /// 把 `<root>/bin` 追加进用户 PATH（并写好 shim）。
    Path,
    /// 把 `<root>/bin` 从用户 PATH 里摘掉。
    Unset,
    /// 按备份还原用户 PATH（原串与原类型）。
    Undo,
    /// 只读状态（缺省模式）。
    Status,
    /// 围栏机器级一次性预授。
    Fence,
    /// 先改 PATH 再预授。
    All,
}

impl SetupMode {
    /// 目的：把命令行上的一个模式词翻成模式；空词等于 `status`。
    /// 返回：认不出的词给 None（调用方按用法错处理，不猜）。
    pub fn parse(raw: &str) -> Option<SetupMode> {
        Some(match raw.trim() {
            "" | "status" => SetupMode::Status,
            "path" => SetupMode::Path,
            "unset" => SetupMode::Unset,
            "undo" => SetupMode::Undo,
            "fence" => SetupMode::Fence,
            "all" => SetupMode::All,
            _ => return None,
        })
    }

    /// 目的：模式在命令面与 JSON 回执里的写法（与 `parse` 成对）。
    pub fn id(self) -> &'static str {
        match self {
            SetupMode::Path => "path",
            SetupMode::Unset => "unset",
            SetupMode::Undo => "undo",
            SetupMode::Status => "status",
            SetupMode::Fence => "fence",
            SetupMode::All => "all",
        }
    }

    /// 目的：可用模式的一行清单（用法错时给人看，与 `parse` 同一处口径）。
    pub fn list() -> &'static str {
        "path / unset / undo / status / fence / all"
    }
}

/// 目的：便携维护的端口——路径与围栏两件事的实现都在 detail，用例面只说做什么。
/// 约束：进度行经 `on_line` 交出，由呈现层决定前缀与排版；返回的 JSON 是机器回执。
pub trait SetupOps: Send + Sync {
    /// 目的：执行一个模式：`Status` 只读，其余按语义写用户 PATH / 写机器级 ACL。
    fn apply(&self, mode: SetupMode, on_line: &mut dyn FnMut(&str)) -> Result<Value, Fault>;
    /// 目的：还缺哪些机器级落点（只读；空 = 已就绪）。
    fn fence_missing(&self) -> Result<Vec<String>, String>;
    /// 目的：把机器级落点一次授好（首次授权后那一趟与 `Fence` 模式共用）。
    fn fence_ensure(&self, on_line: &mut dyn FnMut(&str)) -> Result<(), String>;
}

/// 目的：一次系统维护调用的失败——文案 + 退出码（用法/门槛不对 = 2，执行失败 = 1）。
/// 约束：呈现层只打印 `message` 并按 `code` 退出；不再自己猜该给几。
#[derive(Debug, Clone)]
pub struct Fault {
    pub message: String,
    pub code: i32,
}

impl Fault {
    /// 目的：用法或确认门槛不对（参数缺、缺 --yes、对象不存在）。
    pub fn usage(message: impl Into<String>) -> Fault {
        Fault {
            message: message.into(),
            code: 2,
        }
    }
    /// 目的：执行失败（能力面报错）。
    pub fn failed(message: impl Into<String>) -> Fault {
        Fault {
            message: message.into(),
            code: 1,
        }
    }
}

impl From<String> for Fault {
    fn from(message: String) -> Fault {
        Fault::failed(message)
    }
}

/// 目的：系统维护与自检的用例面——启动相命令与 Web 维护入口的唯一执行契约。
/// 约束：写操作一律要 `confirm`；没有就如实拒绝，不静默改成预览。
pub trait System: Send + Sync {
    /// 目的：自检事实本体（平台、围栏、外部解释器、工具链、进程台账、虚拟机档前置、存储盘面）。
    fn doctor(&self) -> Value;
    /// 目的：空间盘点（机器可读）。
    fn storage(&self) -> Result<Value, Fault>;
    /// 目的：回收一次：类别缺省按策略（`confirm` = 真删），给了类别走显式清理。
    fn gc(&self, req: GcRequest) -> Result<CleanOutcome, Fault>;
    /// 目的：依赖补齐状态（机器可读）。
    fn provision(&self) -> Result<Value, Fault>;
    /// 目的：按需补齐（写操作）：`spec` 给了就按规格，否则本机全部。
    fn ensure(&self, req: EnsureRequest, on_line: &mut dyn FnMut(&str)) -> Result<(), Fault>;
    /// 目的：还原一个由清单构造的虚拟根，并清掉它的构建缓存（写操作）。
    fn reset(&self, req: ResetRequest, on_line: &mut dyn FnMut(&str)) -> Result<GcReport, Fault>;
    /// 目的：回收一个工作区里不再可达的对象。
    fn work_gc(&self, req: WorkGcRequest) -> Result<WorkGcReport, Fault>;
    /// 目的：在目标环境里构建一个模块声明的产物。
    fn build(&self, req: BuildRequest) -> Result<Built, Fault>;
    /// 目的：清理作业的状态快照（回收起了后台作业时，CLI 靠它等结束）。
    fn job(&self) -> JobState;
    /// 目的：便携维护（`--setup`）：PATH 与围栏机器级预授；`Status` 返回状态 JSON。
    fn setup(&self, mode: SetupMode, on_line: &mut dyn FnMut(&str)) -> Result<Value, Fault>;
}

/// 目的：启动报告的工具链事实（项目内 / 缺失 / 宿主回退是否可用）。
pub fn toolchain_lines(facts: &BaseFacts) -> Vec<String> {
    let installed: std::collections::BTreeSet<&str> =
        facts.active.iter().map(|(id, _)| id.as_str()).collect();
    let mut lines = Vec::new();
    if !facts.active.is_empty() {
        let list: Vec<String> = facts
            .active
            .iter()
            .map(|(id, version)| format!("{} {}", id, version))
            .collect();
        lines.push(format!("项目内：{}", list.join("、")));
    }
    let missing: Vec<&str> = TOOLCHAIN_PROFILE
        .iter()
        .copied()
        .filter(|id| !installed.contains(id))
        .collect();
    if !missing.is_empty() {
        lines.push(format!(
            "项目内未安装：{}；补全：solomni --provision --ensure=toolchain --yes",
            missing.join("、")
        ));
        for id in missing {
            match facts.fallback.get(id) {
                Some(f) if f.fenced => lines.push(format!(
                    "宿主回退 {} → {}（围栏可用）",
                    id,
                    f.path.display()
                )),
                Some(f) => lines.push(format!(
                    "宿主回退 {} → {}（围栏不可授权：{}；工具调用会问是否本次无围栏跑）",
                    id,
                    f.path.display(),
                    f.why
                )),
                None => lines.push(format!("宿主回退 {}：宿主 PATH 也没有", id)),
            }
        }
    }
    lines
}

/// 目的：工具链事实的机器可读快照（自检报告与界面横幅共用同一份）。
pub fn toolchain_json(facts: &BaseFacts) -> Value {
    let installed: std::collections::BTreeSet<&str> =
        facts.active.iter().map(|(id, _)| id.as_str()).collect();
    let missing: Vec<&str> = TOOLCHAIN_PROFILE
        .iter()
        .copied()
        .filter(|id| !installed.contains(id))
        .collect();
    serde_json::json!({
        "profile": TOOLCHAIN_PROFILE,
        "active": facts.active.iter().map(|(id, version)| serde_json::json!({
            "id": id,
            "version": version,
        })).collect::<Vec<_>>(),
        "missing": missing,
        "fallback": facts.fallback.iter().map(|(id, f)| (id.clone(), serde_json::json!({
            "path": f.path.to_string_lossy(),
            "fenced": f.fenced,
            "why": f.why,
        }))).collect::<std::collections::BTreeMap<_, _>>(),
        "lines": toolchain_lines(facts),
    })
}
