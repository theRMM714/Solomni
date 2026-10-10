//! 目的：守门进程——把围栏装进工具进程，然后才跑模块声明的命令（实现 tools 的围栏策略）。
//! 管：按平台把围栏（可读可写的根、断网、进程树围栏、资源上限）装好；平台实现分文件 linux.rs / macos.rs / windows/ / other.rs。
//! 不管：策略（哪些根可达、放不放网）——由 conductor 派生后经命令行传入。
//! 联动：能力不足时如实上报（capability），降级而非崩溃——绝不静默假装有围栏。

use crate::kernel::api::FenceSpec;
use crate::kernel::domain::fence::{FenceBlocked, FencePart};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as backend;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as backend;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as backend;
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod other;
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
use other as backend;

/// 目的：守门模式参数：主程序带它启动 = 以守门进程身份执行（内部协议，用户不直接用）。
pub const FENCE_FLAG: &str = "--fence-run";
/// 目的：围栏装不上时守门进程的退出码（工具执行据此如实报错，不静默）。
pub const FENCE_FAILED: i32 = 111;

/// 目的：本机能不能强制住这次执行的围栏——**机制层的验证结论**，与"外层授权了没有"无关。
///   三态的理由：把"本机不允许"与"我们的机制写错了"分开。
///   前者是环境结论，如实降级照跑（能力等级已在启动报告里说过）；后者绝不能静默降级——
///   那等于用户以为有围栏、实际什么都没有。探针早就按这两类分别处理，运行期也必须一样。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FenceVerdict {
    /// 机制真装上了，强制生效。
    Enforced,
    /// 本机环境不允许装（内核不支持、私有 ABI 失效、系统拒绝建容器）——降级照跑，不是我们的错。
    EnvUnavailable(String),
    /// 自检已确认机制有效，但我们的规则/步骤装不上 = 我们写错了。未授权时据此**拒绝执行**。
    Broken(String),
}

/// 目的：测试专用的注入开关：让探针能确定性地构造「本机不允许」这一态。
///   为什么需要：`EnvUnavailable` 只在老内核 / 失效的私有 ABI / 环境拒绝建容器时出现，
///   正常 runner 上碰不到——没有这条开关，那一路分支就只被偶然验证过。
///   **只在测试里打开**（探针自己给守门进程带这个环境变量），运行期永不设置它。
pub const SELFCHECK_FAIL_FLAG: &str = "SOLOMNI_FENCE_SELFCHECK_FAIL";

/// 目的：自检该不该按「本机不允许」处理：测试专用注入优先，否则问真实自检。
///   三平台的自检入口共用这一份判断，避免各写一遍导致探针在某平台上失效。
pub(crate) fn selfcheck_forced_unavailable() -> bool {
    std::env::var_os(SELFCHECK_FAIL_FLAG).is_some()
}

/// 目的：本机能不能强制住这次执行的围栏（机制层自检，**不写本机任何权限项**）。
///   未授权时段靠它把「环境不允许」与「我们写错了」分开——后者绝不能被当成降级吞掉。
pub fn verify(spec: &FenceSpec, command: &str) -> FenceVerdict {
    if selfcheck_forced_unavailable() {
        return FenceVerdict::EnvUnavailable(format!(
            "{}（测试注入：按本机不允许处理）",
            SELFCHECK_FAIL_FLAG
        ));
    }
    backend::verify(spec, command)
}

/// 目的：围栏授权释放的适配器（实现 kernel 共享的 FenceHost 端口）：conductor 只说「这个会话的围栏撤掉」。
pub struct FenceHostAdapter {
    /// 产品私有区（.home/）：台账落点，也是产品根的锚（收尾还原要读它）。
    home: PathBuf,
}

impl FenceHostAdapter {
    /// 目的：组合根注入产品私有区；释放时据此按台账还原或精确撤权。
    pub fn new(home: PathBuf) -> Self {
        Self { home }
    }
}

impl crate::kernel::ports::FenceHost for FenceHostAdapter {
    fn release(&self, spec: &FenceSpec) -> Result<(), String> {
        release_fence(spec, &self.home)
    }
}

/// 目的：本机能强制的围栏等级（如实上报给用户与日志）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capability {
    /// 目的：文件系统可达范围是否被真正强制。
    pub fs: bool,
    /// 目的：出站网络是否被真正强制关闭。
    pub net: bool,
    /// 目的：进程树是否连根围住（超时/退出能杀整棵）。
    pub tree: bool,
    /// 目的：如实说明（机制名 + 限制）。
    pub note: String,
}

/// 目的：本机能力（装配期如实告知）。
pub fn capability() -> Capability {
    backend::capability()
}

/// 目的：守门进程的入参：conductor 的围栏策略 + **外层是否已把本机授权做完** + 产品私有区（台账落点）。
///   授权是改本机目录 ACL 的动作（只有 Windows 的容器围栏需要），所以它不进 conductor 的 `FenceSpec`，
///   由适配层随这次执行一起交给守门进程。JSON 是**扁平**的：FenceSpec 的字段同层再加 `prepared` 与 `home`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FenceJob {
    pub spec: FenceSpec,
    pub prepared: bool,
    /// 目的：产品私有区（`.home/`）：守门进程把**它建过的容器 profile** 记进这里的台账，供 `--fence-clean` 精确回收。
    ///   守门进程是唯一真正建 profile 的地方，外层只知道"该建"、不知道"建成了"。
    ///   没有台账的调用方（探针）给 `None`：那类 profile 由 `--fence-clean` 的前缀清扫兜底。
    pub home: Option<PathBuf>,
}

impl FenceJob {
    pub fn to_json(&self) -> String {
        let mut fields = match serde_json::to_value(&self.spec) {
            Ok(serde_json::Value::Object(o)) => o,
            _ => serde_json::Map::new(),
        };
        fields.insert(
            "prepared".to_string(),
            serde_json::Value::Bool(self.prepared),
        );
        fields.insert(
            "home".to_string(),
            serde_json::to_value(&self.home).unwrap_or(serde_json::Value::Null),
        );
        serde_json::Value::Object(fields).to_string()
    }

    pub fn from_json(text: &str) -> Result<FenceJob, String> {
        #[derive(serde::Deserialize)]
        struct Raw {
            #[serde(flatten)]
            spec: FenceSpec,
            /// 缺了这一项 = 调用方没说清有没有授权，如实报错（不默认成"有"）。
            prepared: bool,
            /// 缺省 = 没有台账可落（探针、别的调用方）；这不是"忘记传"，所以允许缺省。
            #[serde(default)]
            home: Option<PathBuf>,
        }
        serde_json::from_str::<Raw>(text)
            .map(|raw| FenceJob {
                spec: raw.spec,
                prepared: raw.prepared,
                home: raw.home,
            })
            .map_err(|e| format!("围栏参数非法：{}", e))
    }
}

/// 目的：组装守门进程的命令行：工具命令作为**数据**传递（不拼进 shell 字符串，杜绝注入）。
pub fn launcher(exe: &Path, job: &FenceJob, command: &str) -> Command {
    let mut cmd = Command::new(exe);
    cmd.arg(FENCE_FLAG)
        .arg(job.to_json())
        .arg("--")
        .arg(command);
    cmd
}

/// 目的：守门进程内：装围栏 → 跑命令 → 返回退出码。失败必须报错（stderr）并用 FENCE_FAILED 退出。
pub fn run_fenced(job: &FenceJob, command: &str) -> i32 {
    backend::run_fenced(&job.spec, job.prepared, job.home.as_deref(), command)
}

/// 目的：围栏里工具失败时给回执的边界说明（可达范围 + 范围外被拒）；不需要说明时返回 None。
/// 约束：只在围栏真强制生效且工具自己非零退出时给；不含任何语言知识，也不猜被拒的是哪条路径。
fn fence_boundary_note(spec: &FenceSpec, enforced: bool, code: i32) -> Option<String> {
    if !enforced || code == 0 {
        return None;
    }
    Some(format!(
        "[围栏] 工具进程只可达 {}（网：{}）；范围外的访问被围栏拒绝。",
        reachable_roots(spec),
        if spec.net { "开" } else { "关" }
    ))
}

/// 目的：可达范围的一行摘要（工作目录 + 读写/只读根，去重；超过三处折叠成一处计数）。
fn reachable_roots(spec: &FenceSpec) -> String {
    let mut all: Vec<String> = Vec::new();
    push_root(&mut all, &spec.cwd);
    for p in spec
        .rw
        .iter()
        .chain(spec.ro.iter())
        .chain(spec.ro_tree.iter())
    {
        push_root(&mut all, p);
    }
    if all.is_empty() {
        return "（无）".to_string();
    }
    if all.len() <= 3 {
        return all.join("、");
    }
    format!("{} 等 {} 处", all[..3].join("、"), all.len())
}

/// 目的：把一个非空的根收进摘要，重复的丢掉。
fn push_root(all: &mut Vec<String>, p: &Path) {
    if p.as_os_str().is_empty() {
        return;
    }
    let s = p.display().to_string();
    if !all.contains(&s) {
        all.push(s);
    }
}

/// 目的：把边界说明打到 stderr（守门进程的 stderr 由外层拼进工具回执）。
fn note_fence_boundary(spec: &FenceSpec, enforced: bool, code: i32) {
    if let Some(line) = fence_boundary_note(spec, enforced, code) {
        eprintln!("{}", line);
    }
}

/// 目的：扫掉本程序建过的整族容器 profile：台账只记"我们知道写过什么"，而 profile 可能来自没有台账的路径
///   （探针、夹具的台账被删、旧版本）。名字前缀是本程序独有的，所以按它扫。返回扫掉的个数。
pub fn sweep_profiles() -> Result<usize, String> {
    #[cfg(windows)]
    {
        windows::sweep_profiles()
    }
    #[cfg(not(windows))]
    {
        // 其它平台没有容器 profile 这一步。
        Ok(0)
    }
}

/// 目的：一次围栏授权的结论：**必要**落点是否全部授上 + 其余授不上落点的事实。
/// 约束：必要落点授不上时调用方**不许降级**（问用户或按 fail-closed 拒绝，见 docs/tools/README.md）；
///   可选落点授不上只进 `notes`，不牵动这次执行。
#[derive(Debug, Clone, Default)]
// 这套机制只在 Windows 的容器围栏里用（unix 没有 prepare_fence 这一步）：unix 侧无使用点，如实放行死代码。
#[allow(dead_code)]
pub struct FencePrep {
    /// 目的：必要落点授不上时的如实结论（`None` = 围栏成立，可以按围栏执行）。
    pub blocked: Option<FenceBlocked>,
    /// 目的：其余授不上落点的事实（可选落点只在这里记一条）。
    pub notes: Vec<String>,
}

impl FencePrep {
    /// 目的：这次授权成不成立（必要落点全授上了）。
    // 这套机制只在 Windows 的容器围栏里用（unix 没有 prepare_fence 这一步）：unix 侧无使用点，如实放行死代码。
    #[allow(dead_code)]
    pub fn ok(&self) -> bool {
        self.blocked.is_none()
    }

    /// 目的：记一个落点授不上的事实：**必要**落点进 `blocked`（第一次为准——那就是要补的那一环），
    ///   可选落点与后续必要落点进 `notes`。
    // 这套机制只在 Windows 的容器围栏里用（unix 没有 prepare_fence 这一步）：unix 侧无使用点，如实放行死代码。
    #[allow(dead_code)]
    pub fn fail(&mut self, part: FencePart, path: std::path::PathBuf, why: String) {
        let fact = FenceBlocked { part, path, why };
        if fact.part.necessary() && self.blocked.is_none() {
            self.blocked = Some(fact);
            return;
        }
        self.notes.push(fact.line());
    }
}

/// 目的：外层进程调用：把围栏要用的授权一次性做好（写目录 ACL），并按落点的必要性如实收尾。
///   跳过条件看**实际 ACE**而不是内存台账，所以权限收窄并撤权后能正确重授。
///   只有 Windows 的容器围栏需要这一步——Linux 的 Landlock 与 macOS 的 seatbelt 在守门进程里自足，
///   所以本函数在非 Windows 平台上**不存在**（而不是"存在但空转"）。
#[cfg(windows)]
pub fn prepare_fence(spec: &FenceSpec, command: &str, home: &std::path::Path) -> FencePrep {
    windows::prepare_fence(spec, command, home)
}

/// 目的：精确回收：按台账撤掉我们写过的权限项、删掉我们建过的容器 profile（`--fence-clean` 用）。
pub fn clean(home: &std::path::Path) -> Result<String, String> {
    #[cfg(windows)]
    {
        windows::clean(home)
    }
    #[cfg(not(windows))]
    {
        let _ = home;
        Ok("本平台的围栏不留权限项，无需清理".to_string())
    }
}

/// 目的：启动期对账——按台账回收"归属明确已死"的陈旧授权；无法判定的报告后跳过（启动与 `--fence-reconcile` 共用）。
/// 约束：只有 Windows 写本机权限项；其它平台没有台账，如实返回空报告。
pub fn reconcile(home: &std::path::Path) -> ReconcileReport {
    #[cfg(windows)]
    {
        windows::reconcile(home)
    }
    #[cfg(not(windows))]
    {
        let _ = home;
        ReconcileReport {
            note: "本平台的围栏不留权限项，没有台账可对账".to_string(),
            ..Default::default()
        }
    }
}

/// 目的：孤儿授权清扫：按容器 SID 族在产品根内撤掉台账之外的残留 ACE（--fence-clean 用）。
///   只有 Windows 写本机 ACL，其它平台没有这一步（而不是"存在但空转"）。
#[cfg(windows)]
pub fn sweep_orphan_aces(root: &std::path::Path) -> Result<usize, String> {
    windows::sweep_orphan_aces(root)
}

/// 目的：台账条目的归属——哪个进程还需要这条授权（pid + 进程创建时刻，防 PID 复用）。
/// 约束：跨平台只承载数据；活性判定与写盘都在 Windows 后端（其它平台没有持久授权，也就没有归属）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Owner {
    /// 目的：归属的进程号（缺省 = 0，判不了，按"无法判定"处理，不主动回收）。
    #[serde(default)]
    pub pid: u32,
    /// 目的：进程创建时刻（Windows FILETIME，自 1601 起的 100ns；0 = 读不到，按"无法判定"处理）。
    #[serde(default)]
    pub start: u64,
    /// 目的：会话租约（会话 id；空 = 无会话/旧格式）。同一进程里同名 agent 的多个会话靠它区分。
    #[serde(default)]
    pub lease: String,
}

/// 目的：一次启动对账的结果（如实交代回收了什么、跳过了什么、哪里失败）。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ReconcileReport {
    /// 目的：一句如实说明（没有台账 / 本平台不留权限项）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    pub reclaimed_grants: usize,
    pub restored_snapshots: usize,
    pub deleted_profiles: usize,
    /// 目的：保留的条目数。
    pub kept: usize,
    /// 目的：没有归属或归属判不了、报告后跳过的条目数。
    pub skipped_unjudgeable: usize,
    /// 目的：本次回收前的台账备份（展示用；没有备份时为空）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<String>,
    /// 目的：没清掉的部分（失败保留供下次重试）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

impl ReconcileReport {
    /// 目的：这次对账有没有"值得说的"（真回收了、跳过了、或失败了）——没有就保持安静，不刷启动日志。
    pub fn has_activity(&self) -> bool {
        self.reclaimed_grants > 0
            || self.restored_snapshots > 0
            || self.deleted_profiles > 0
            || self.skipped_unjudgeable > 0
            || !self.errors.is_empty()
    }

    /// 目的：一行如实摘要（启动打印与手动入口共用）。
    pub fn summary(&self) -> String {
        if !self.errors.is_empty() {
            let skipped = if self.skipped_unjudgeable > 0 {
                format!("；另有 {} 条无法判定已跳过", self.skipped_unjudgeable)
            } else {
                String::new()
            };
            return format!(
                "对账未完成：{}（已还原 {} 处、撤销 {} 条、删 {} 个 profile；台账保留供重试{}）",
                self.errors.join("；"),
                self.restored_snapshots,
                self.reclaimed_grants,
                self.deleted_profiles,
                skipped
            );
        }
        if !self.note.is_empty() {
            return self.note.clone();
        }
        if self.reclaimed_grants == 0 && self.restored_snapshots == 0 && self.deleted_profiles == 0
        {
            return if self.skipped_unjudgeable > 0 {
                format!(
                    "没有可回收的陈旧授权；{} 条无法判定已跳过（用 --fence-clean 处置）",
                    self.skipped_unjudgeable
                )
            } else {
                "没有陈旧授权，无需回收".to_string()
            };
        }
        format!(
            "已回收陈旧授权：还原快照 {} 处、撤销授权 {} 条、删除 profile {} 个；保留 {} 条{}",
            self.restored_snapshots,
            self.reclaimed_grants,
            self.deleted_profiles,
            self.kept,
            if self.skipped_unjudgeable > 0 {
                format!("（另有 {} 条无法判定已跳过）", self.skipped_unjudgeable)
            } else {
                String::new()
            }
        )
    }
}

/// 目的：台账现值里的**一条**（机器可读，与平台无关的形状）：哪一类、谁、哪个路径、什么权限、什么时候记的。
/// 约束：present = 这一条现在还在不在（路径在不在、ACE 还在不在、profile 还在不在）；
///   owners = 还需要这条授权的进程；ace_sids = 该路径上**现在**看得到的显式包 SID 允许 ACE；
///   notes = 读不到 DACL、认不出布局、路径不在这类如实记下的事实。
#[derive(Debug, serde::Serialize)]
pub struct LedgerEntry {
    /// 目的：这一条属于哪一类——snapshot（根内快照）/ grant（授权）/ profile（容器 profile）。
    pub kind: &'static str,
    /// 目的：授给谁（只有 grant 有；其余为空串）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub sid: String,
    /// 目的：哪个路径（profile 条目里是 profile 名，其余是路径）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub path: String,
    /// 目的：权限位（只有 grant 有）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rights: Option<u32>,
    /// 目的：记录时刻（Unix 秒）。
    pub at: u64,
    /// 目的：这一条现在还在不在。
    pub present: bool,
    /// 目的：还需要这条授权的归属（快照没有归属：它的去留由该路径上的授权条目派生）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub owners: Vec<Owner>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ace_sids: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// 目的：台账现值（机器可读的数字面）：条目清单 + 一句如实说明。
#[derive(Debug, serde::Serialize)]
pub struct Ledger {
    /// 目的：一句如实说明（没有台账 / 本平台不留权限项）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    pub entries: Vec<LedgerEntry>,
}

/// 目的：台账现值（清单 + "当前实际 ACE 与台账对不对得上"的差异）。
/// 约束：只有 Windows 写本机权限项，所以台账只在 Windows 上有内容；其它平台如实说"本平台不留权限项"。
pub fn ledger(home: &std::path::Path) -> Ledger {
    #[cfg(windows)]
    {
        windows::catalog(home)
    }
    #[cfg(not(windows))]
    {
        let _ = home;
        Ledger {
            note: "本平台的围栏不留权限项，没有台账".to_string(),
            entries: Vec::new(),
        }
    }
}

/// 目的：按路径**只还原一条**快照（其余条目与整份 DACL 不受影响）。
/// 错误：台账里没有该路径、路径不在了、写回被拒都如实返回；失败时台账不改（供重试）。
pub fn restore_one(home: &std::path::Path, path: &std::path::Path) -> Result<String, String> {
    #[cfg(windows)]
    {
        windows::restore_one(home, path)
    }
    #[cfg(not(windows))]
    {
        let _ = (home, path);
        Err("本平台的围栏不留权限项，没有可还原的快照".to_string())
    }
}

/// 目的：按 **SID + 路径**只撤一条授权（其余条目不受影响）；**不在台账里也照撤**（台账外残留走这条）。
/// 错误：路径不在了、SID 不合法、写撤权后的 DACL 被拒都如实返回；失败时台账不改动。
pub fn revoke_grant(
    home: &std::path::Path,
    sid: &str,
    path: &std::path::Path,
) -> Result<String, String> {
    #[cfg(windows)]
    {
        windows::revoke_grant(home, sid, path)
    }
    #[cfg(not(windows))]
    {
        let _ = (home, sid, path);
        Err("本平台的围栏不留权限项，没有可撤销的授权".to_string())
    }
}

/// 目的：按名**只删一个**容器 profile（连该容器的存储一起删）。
/// 错误：删除被拒时如实返回；失败时台账不改动。
pub fn remove_profile_one(home: &std::path::Path, name: &str) -> Result<String, String> {
    #[cfg(windows)]
    {
        windows::remove_profile_one(home, name)
    }
    #[cfg(not(windows))]
    {
        let _ = (home, name);
        Err("本平台没有容器 profile".to_string())
    }
}

/// 目的：撤销一次会话的围栏授权（会话删除时由核心经 FenceHost 端口请求；其它平台是空操作）。
pub fn release_fence(spec: &FenceSpec, home: &std::path::Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows::release_fence(spec, home)
    }
    #[cfg(not(windows))]
    {
        let _ = (spec, home);
        Ok(())
    }
}

/// 目的：命令里可能出现的外部程序：按 PATH 解析出真实路径（解析不出的跳过，不猜）。
///   它们的**安装目录**必须放行（只读+执行），否则受限进程连解释器都起不来——Windows 的目录 ACL 与 macOS 的 seatbelt 都靠它。
pub(crate) fn interpreter_dirs(command: &str) -> Vec<std::path::PathBuf> {
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let pathext = std::env::var("PATHEXT").ok();
    interpreter_dirs_in(command, &path_var, pathext.as_deref())
}

/// 可执行扩展名：PATHEXT（若在场）**加**一份标准兜底，按小写去重。
/// 兜底不是装饰：PATHEXT 可能缺席（CI 的 runner 起 node 再起产品），
/// 那时只按 PATHEXT 找扩展名会一个解释器都解析不出来——容器里的工具连 python 都找不到。
fn exec_extensions(pathext: Option<&str>) -> Vec<String> {
    let mut exts: Vec<String> = Vec::new();
    let mut push = |ext: &str| {
        let ext = ext.trim().to_ascii_lowercase();
        if !ext.is_empty() && !exts.contains(&ext) {
            exts.push(ext);
        }
    };
    for ext in [".exe", ".cmd", ".bat", ".com"] {
        push(ext);
    }
    if let Some(text) = pathext {
        for ext in text.split(';') {
            push(ext);
        }
    }
    exts
}

/// 去掉 Windows 规范化路径的 `\\?\` 前缀：安全描述符接口不认这个前缀，带上就是白写一条授权。
fn strip_verbatim_prefix(path: &std::path::Path) -> std::path::PathBuf {
    match path.to_string_lossy().strip_prefix(r"\\?\") {
        Some(rest) => std::path::PathBuf::from(rest),
        None => path.to_path_buf(),
    }
}

/// 命令里出现的**程序**：按 PATH 解析出真实路径（解析不出的跳过，不猜）。
/// 约束：绝对路径的 token 一律不算——那可能是数据文件，把它当程序会连带把它放行（见 interpreter_dirs 的用例）。
/// 约束：解释器目录与「命令用到了哪个解释器」都从这一份派生——两处各写一遍迟早会漏掉某一类
///   （PATHEXT 缺席、带引号的 PATH 项、符号链接的真身）。
fn command_programs_in(
    command: &str,
    path_var: &std::ffi::OsStr,
    pathext: Option<&str>,
) -> Vec<std::path::PathBuf> {
    let mut candidates: Vec<String> = Vec::new();
    for raw in command.split([' ', '\t', '&', '|', ';', '\n']) {
        let token = raw.trim_matches(|c| c == '"' || c == '\'' || c == '(' || c == ')');
        if token.is_empty()
            || token.starts_with('-')
            || token.starts_with('/')
            || token.starts_with('%')
        {
            continue;
        }
        if std::path::Path::new(token).is_absolute() {
            continue;
        }
        candidates.push(token.to_string());
    }
    let exts = exec_extensions(pathext);
    let mut hits: Vec<std::path::PathBuf> = Vec::new();
    for name in candidates {
        for entry in std::env::split_paths(path_var) {
            // PATH 项可能带外层引号（手写的 PATH 常见）：带引号去 join 就永远找不到。
            let text = entry.to_string_lossy();
            let unquoted = text.trim_matches('"');
            let dir = if unquoted.len() == text.len() {
                entry
            } else {
                std::path::PathBuf::from(unquoted.to_string())
            };
            let mut tries: Vec<std::path::PathBuf> = vec![dir.join(&name)];
            for e in &exts {
                tries.push(dir.join(format!("{}{}", name, e)));
            }
            if let Some(hit) = tries.into_iter().find(|p| p.is_file() && is_executable(p)) {
                hits.push(hit);
                break;
            }
        }
    }
    hits
}

/// 解析规则由调用方把环境形状喂进来：**不能假设 PATH / PATHEXT 一定在场或一定干净**。
fn interpreter_dirs_in(
    command: &str,
    path_var: &std::ffi::OsStr,
    pathext: Option<&str>,
) -> Vec<std::path::PathBuf> {
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    for hit in command_programs_in(command, path_var, pathext) {
        // 解释器常见布局：<root>/bin/xxx（官方安装与虚拟环境）或 <root>/xxx。
        if let Some(parent) = hit.parent() {
            dirs.push(install_dir(parent));
        }
        // 符号链接要把**真身**的安装目录也放行：macOS 上 python3 常常是链接，动态库在真身旁边——
        // 只放行链接所在目录会让加载器取不到库（进程直接 SIGABRT）。
        if let Ok(real) = std::fs::canonicalize(&hit) {
            let real = strip_verbatim_prefix(&real);
            if real != hit {
                if let Some(parent) = real.parent() {
                    dirs.push(install_dir(parent));
                }
            }
        }
    }
    dirs.sort();
    dirs.dedup();
    dirs.into_iter().filter(|d| d.is_dir()).collect()
}

/// 程序所在目录 → 该程序的「安装目录」：<root>/bin|Scripts 这种布局上溯一层（标准库与动态库在 <root> 里），其余就是所在目录。
fn install_dir(bin: &std::path::Path) -> std::path::PathBuf {
    let leaf = bin
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if leaf == "bin" || leaf == "scripts" {
        if let Some(up) = bin.parent() {
            // 绝不把「文件系统根」当安装目录：/bin 的父目录就是 /，
            // 上溯到根等于把整盘放行（macOS 上会直接让围栏形同虚设）。
            if up.parent().is_some() {
                return up.to_path_buf();
            }
        }
    }
    bin.to_path_buf()
}

/// node 的「跳过 realpath」开关：主模块看 `-main`，依赖与 ESM 看另一个——**两个都要**
/// （单开任何一个，另一半照样 realpath，真机实测如此）。
#[cfg(windows)]
const NODE_REALPATH_FLAGS: &str = "--preserve-symlinks --preserve-symlinks-main";

/// 目的：命令用 node 时要补的环境（`NODE_OPTIONS` 跳过 realpath）；不是 node 就是 `None`。
/// 约束：Windows 的容器围栏才需要它——`fs.realpathSync` 先 lstat 盘卷根、再逐级 lstat 祖先前缀，
///   而这两类落点按设计都不在可达范围（卷根属主是系统、非管理员改不动；祖先链只靠令牌的
///   「按名穿过」特权，管不到显式 lstat），进程在脚本执行前就 EPERM 死。
/// 约束：另两个平台不需要：Landlock 的权限位里没有「读属性」这一项，seatbelt 的规则显式给祖先
///   放行了 file-read-metadata（见 macos.rs 的祖先元数据放行）。
#[cfg(windows)]
fn node_realpath_env(command: &str) -> Option<(OsString, OsString)> {
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let pathext = std::env::var("PATHEXT").ok();
    node_realpath_env_in(command, &path_var, pathext.as_deref())
}

/// 同上，环境形状由调用方喂进来（与 `interpreter_dirs_in` 同一个理由：不能假设 PATH / PATHEXT 干净）。
#[cfg(windows)]
fn node_realpath_env_in(
    command: &str,
    path_var: &std::ffi::OsStr,
    pathext: Option<&str>,
) -> Option<(OsString, OsString)> {
    let is_node = command_programs_in(command, path_var, pathext)
        .iter()
        .any(|p| is_node_program(p));
    if !is_node {
        return None;
    }
    Some((
        OsString::from("NODE_OPTIONS"),
        OsString::from(NODE_REALPATH_FLAGS),
    ))
}

/// 程序是不是 node：看程序名，也看**真身**的程序名——安装器常把 `<root>/nodejs` 做成指向具体版本目录的
/// 符号链接（真机如此），只认链接名会漏掉那一类。
#[cfg(windows)]
fn is_node_program(program: &std::path::Path) -> bool {
    fn named_node(p: &std::path::Path) -> bool {
        p.file_stem()
            .map(|s| s.to_string_lossy().eq_ignore_ascii_case("node"))
            .unwrap_or(false)
    }
    if named_node(program) {
        return true;
    }
    std::fs::canonicalize(program)
        .map(|real| named_node(&strip_verbatim_prefix(&real)))
        .unwrap_or(false)
}

/// 是不是「可执行文件」：Unix 看执行位；Windows 没有这个概念，文件存在即算（PATH 解析已按 PATHEXT 试过扩展名）。
#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(_path: &std::path::Path) -> bool {
    true
}

/// 目的：交给 cmd 解释前，把**程序名**里的正斜杠换成反斜杠。
///   cmd 只把程序名里的 `\` 当路径分隔符：`build/indexer build` 会被它读成「命令 build + 开关 /indexer」，
///   报 `'build' is not recognized`。程序名之后的参数原样保留（`node tools/report.js` 这类命令靠参数里的正斜杠）。
///   程序名 = 第一个空白前的字段；模块作者若用引号包住程序名，只改引号内那一段。
#[cfg(windows)]
pub(crate) fn windows_program_separators(command: &str) -> String {
    let end = match command.strip_prefix('"') {
        Some(rest) => rest.find('"').map(|i| i + 2).unwrap_or(command.len()),
        None => command.find(char::is_whitespace).unwrap_or(command.len()),
    };
    command[..end].replace('/', "\\") + &command[end..]
}

/// 目的：工具进程的启动命令：命令行由**模块作者**写在 module.yaml 里，交系统 shell 解释（与既有语义一致）。
#[cfg(windows)]
pub fn shell_command(command: &str) -> Command {
    let mut c = Command::new("cmd");
    c.arg("/C").arg(windows_program_separators(command));
    c
}

#[cfg(not(windows))]
pub fn shell_command(command: &str) -> Command {
    let mut c = Command::new("sh");
    c.arg("-c").arg(command);
    c
}

/// 目的：环境白名单：子进程只拿到这些（其余一律不继承——密钥与无关凭据不进工具进程）。
///   解释器需要 HOME/TEMP 这类落点：全部指到该 agent 的私有沙箱里（缓存与临时文件落在工作区内）。
pub fn fence_env(spec: &FenceSpec, command: &str) -> Vec<(OsString, OsString)> {
    let keep = [
        "PATH",
        "PATHEXT",
        "SystemRoot",
        "WINDIR",
        "COMSPEC",
        "ComSpec",
        "SYSTEMDRIVE",
        "LANG",
        "LC_ALL",
        "TZ",
    ];
    let mut out: Vec<(OsString, OsString)> = Vec::new();
    for k in keep {
        if let Some(v) = std::env::var_os(k) {
            out.push((OsString::from(k), v));
        }
    }
    // 文本编码统一 UTF-8（Windows 上 Python 默认按系统代码页解 stdin，会把中文参数解坏）。
    out.push((OsString::from("PYTHONIOENCODING"), OsString::from("utf-8")));
    out.push((OsString::from("PYTHONUTF8"), OsString::from("1")));
    // 工作区内的落点：私有沙箱作为 HOME / TEMP（缓存与临时文件不出工作区）。
    let home = spec.private_or_cwd();
    out.push((OsString::from("HOME"), home.clone().into_os_string()));
    // Windows 建 AppContainer 进程要读它：白名单里没有它就 CreateProcessW 直接失败（os error 203），
    // 容器整条路会静默降级成无围栏执行。落点同样指进该 agent 的私有沙箱。
    out.push((
        OsString::from("LOCALAPPDATA"),
        home.clone().into_os_string(),
    ));
    out.push((OsString::from("USERPROFILE"), home.clone().into_os_string()));
    out.push((OsString::from("TEMP"), home.clone().into_os_string()));
    out.push((OsString::from("TMP"), home.clone().into_os_string()));
    // macOS / Linux 认 TMPDIR：不设的话进程会去读系统临时区（那不在可达范围内）。
    out.push((OsString::from("TMPDIR"), home.into_os_string()));
    // 解释器基线：命令用 node 时要跳过 realpath，否则容器里起不来（理由见 node_realpath_env）。
    #[cfg(windows)]
    if let Some((k, v)) = node_realpath_env(command) {
        out.push((k, v));
    }
    // 其余平台不需要它：Landlock 不管 stat、seatbelt 已放行祖先元数据，realpath 正常。
    #[cfg(not(windows))]
    let _ = command;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试专用注入开关：打开它，机制验证必须确定性地报「本机不允许」。
    /// 这是覆盖 EnvUnavailable 那一路的唯一确定性手段——三平台探针都靠它。
    ///
    /// **不能在进程内调未注入的真实后端**：unix 上 Landlock 与 seatbelt 都是**进程级且不可逆**的
    /// ——装了它，测试进程此后连 `target/` 都写不了（macOS CI 上真抓到过：后续 123 个用例全挂在
    /// "建契约测试隔离根：Operation not permitted"）。真实后端的行为由平台探针（子进程里）验收，
    /// 这里只钉注入开关本身的语义。
    #[test]
    fn selfcheck_injection_switch_reports_env_unavailable() {
        let spec = FenceSpec {
            agent: "a".to_string(),
            lease: String::new(),
            private: PathBuf::new(),
            ro_tree: Vec::new(),
            rw: vec![PathBuf::from("demo").join("work")],
            cwd: PathBuf::from("mods").join("m0"),
            ro: Vec::new(),
            net: false,
        };
        // 运行期不该有这个开关（探针自己给守门进程带）。
        assert!(!selfcheck_forced_unavailable(), "运行期不该有这个开关");
        // 注入后：必须在触到后端之前就返回「本机不允许」，且带得出注入标记
        // （探针据此把"环境结论"与"我们写错了"分开）。
        std::env::set_var(SELFCHECK_FAIL_FLAG, "1");
        let injected = verify(&spec, "true");
        std::env::remove_var(SELFCHECK_FAIL_FLAG);
        match injected {
            FenceVerdict::EnvUnavailable(why) => assert!(
                why.contains(SELFCHECK_FAIL_FLAG),
                "理由要带得出注入标记：{}",
                why
            ),
            other => panic!("注入后必须是本机不允许，实际 {:?}", other),
        }
        // 开关关掉之后必须回到真实判定入口（不被上一次注入粘住）。
        assert!(
            !selfcheck_forced_unavailable(),
            "注入是一次性的，不该留下状态"
        );
    }
    /// 安装目录上溯**绝不能停在文件系统根**：/bin 的父目录就是 /，
    /// 一旦返回 / 就等于把整盘放行（macOS 的 seatbelt 会因此形同虚设）。
    #[test]
    fn install_dir_never_climbs_to_the_filesystem_root() {
        let root = if cfg!(windows) {
            PathBuf::from("C:\\")
        } else {
            PathBuf::from("/")
        };
        assert_eq!(
            install_dir(&root.join("bin")),
            root.join("bin"),
            "根下的 bin 不再上溯"
        );
        assert_eq!(install_dir(&root), root, "根就是根");
        let deep = root.join("home").join("u").join(".venv").join("bin");
        assert_eq!(
            install_dir(&deep),
            root.join("home").join("u").join(".venv"),
            "普通布局上溯一层"
        );
        let scripts = root.join("home").join("u").join("env").join("Scripts");
        assert_eq!(
            install_dir(&scripts),
            root.join("home").join("u").join("env"),
            "Scripts 布局同样上溯"
        );
        assert_eq!(
            install_dir(&root.join("usr").join("local").join("bin")),
            root.join("usr").join("local"),
            "usr/local/bin 上溯到 usr/local"
        );
    }

    /// cmd 只认程序名里的反斜杠：`build/indexer build` 会被读成命令 build + 开关 /indexer（模块工具会因此跑不起来）。
    /// 参数里的正斜杠必须原样保留——`node tools/report.js` 正是靠它。
    #[cfg(windows)]
    #[test]
    fn windows_program_separators_rewrites_only_the_program_name() {
        assert_eq!(
            windows_program_separators("build/indexer build"),
            "build\\indexer build"
        );
        assert_eq!(
            windows_program_separators("node tools/report.js"),
            "node tools/report.js"
        );
        assert_eq!(
            windows_program_separators("python tools/scan.py extra"),
            "python tools/scan.py extra"
        );
        assert_eq!(
            windows_program_separators(".tools/mingw64/bin/g++.exe -O2"),
            ".tools\\mingw64\\bin\\g++.exe -O2"
        );
        assert_eq!(
            windows_program_separators("build\\indexer build"),
            "build\\indexer build"
        );
        assert_eq!(windows_program_separators("\"a/b\" rest"), "\"a\\b\" rest");
        assert_eq!(windows_program_separators("plain"), "plain");
    }

    /// 守门进程的入参是**扁平** JSON：FenceSpec 的字段同层再加一个 prepared；
    /// 缺字段一律报错（不默认成"有授权"——那会把容器送进一个读不到东西的环境）。
    #[test]
    fn fence_job_round_trips_and_rejects_incomplete_json() {
        let spec = FenceSpec {
            agent: "a".to_string(),
            lease: String::new(),
            private: PathBuf::new(),
            ro_tree: Vec::new(),
            rw: vec![PathBuf::from("demo").join("work")],
            cwd: PathBuf::from("mods").join("m0"),
            ro: Vec::new(),
            net: false,
        };
        let job = FenceJob {
            spec,
            prepared: true,
            home: Some(PathBuf::from("home")),
        };
        let text = job.to_json();
        assert!(text.contains("\"prepared\":true"), "{}", text);
        assert_eq!(FenceJob::from_json(&text).expect("回读守门进程入参"), job);
        // 没有台账可落是合法形态（探针），所以 home 允许缺省；prepared 缺了才报错。
        let bare = FenceSpec {
            agent: "b".to_string(),
            lease: String::new(),
            private: PathBuf::new(),
            ro_tree: Vec::new(),
            rw: vec![PathBuf::from("demo").join("work")],
            cwd: PathBuf::from("mods").join("m1"),
            ro: Vec::new(),
            net: true,
        };
        let no_home = FenceJob {
            spec: bare,
            prepared: false,
            home: None,
        };
        assert_eq!(
            FenceJob::from_json(&no_home.to_json()).expect("回读"),
            no_home
        );
        assert!(FenceJob::from_json("{}").is_err(), "缺字段必须报错，不猜");
        assert!(FenceJob::from_json("这不是 JSON").is_err());
    }

    /// 解析解释器不能假设环境形状：PATHEXT 缺席（真机 CI 上见过）时靠标准兜底扩展名照样找到 .exe，
    /// 带引号的 PATH 项照样能用——否则容器里的工具连解释器都找不到。
    #[test]
    fn interpreter_dirs_resolves_without_pathext_and_with_quoted_path_entries() {
        let root = crate::tests::scratch("interpreter-dirs");
        let pydir = root.join("pydir");
        std::fs::create_dir_all(&pydir).expect("建解释器目录");
        let exe = pydir.join(if cfg!(windows) {
            "python.exe"
        } else {
            "python"
        });
        std::fs::write(&exe, b"#!/bin/sh\nexit 0\n").expect("放一个假解释器");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perm = std::fs::metadata(&exe).expect("读权限位").permissions();
            perm.set_mode(0o755);
            std::fs::set_permissions(&exe, perm).expect("加执行位");
        }
        let path = pydir.as_os_str();
        assert_eq!(
            interpreter_dirs_in("python tools/x.py", path, None),
            vec![pydir.clone()],
            "PATHEXT 缺席也要解析出解释器目录"
        );
        assert_eq!(
            interpreter_dirs_in("python tools/x.py", path, Some(".EXE;.BAT")),
            vec![pydir.clone()],
            "PATHEXT 在场照走 PATHEXT"
        );
        let quoted = std::ffi::OsString::from(format!("\"{}\"", pydir.display()));
        assert_eq!(
            interpreter_dirs_in("python tools/x.py", &quoted, None),
            vec![pydir.clone()],
            "带引号的 PATH 项也要能解析"
        );
        assert!(
            interpreter_dirs_in(&format!("cat {}", exe.display()), path, None)
                .iter()
                .all(|d| d != &pydir),
            "数据文件路径不算解释器（否则等于给围栏开洞）"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 命令里的解释器要按 PATH 解析出真实路径，并给出它的安装目录；
    /// 数据文件路径不算解释器（否则会把那个文件放行，等于开洞）。
    #[test]
    fn interpreter_dirs_resolves_programs_but_not_data_files() {
        let cmd = if cfg!(windows) {
            "cmd /C echo hi"
        } else {
            "sh -c 'echo hi'"
        };
        let dirs = interpreter_dirs(cmd);
        assert!(!dirs.is_empty(), "系统 shell 应当能被解析出来：{:?}", dirs);
        for d in &dirs {
            assert!(d.is_dir(), "只报真实存在的目录：{:?}", d);
            assert!(d.parent().is_some(), "绝不报文件系统根：{:?}", d);
        }
        // 明确的非程序路径（一个不存在的文件）不该被当成解释器。
        let missing = if cfg!(windows) {
            "C:\\nope\\nope.exe"
        } else {
            "/nope/nope"
        };
        assert!(
            interpreter_dirs(&format!("cat {}", missing))
                .iter()
                .all(|d| !d.ends_with("nope")),
            "数据/缺失路径不该被当成解释器"
        );
    }

    /// 解释器基线：`node <文件>` 的命令要带上"跳过 realpath"的两个开关；别的解释器、命令里的数据文件路径都不带。
    /// 判据与解释器目录同一份 PATH 解析（PATHEXT 兜底、带引号的 PATH 项、按名解析才认）。
    #[cfg(windows)]
    #[test]
    fn node_commands_get_the_realpath_skip_and_nothing_else_does() {
        let root = crate::tests::scratch("interpreter-node");
        let dir = root.join("nodedir");
        std::fs::create_dir_all(&dir).expect("建解释器目录");
        std::fs::write(dir.join("node.exe"), b"stub").expect("放假 node");
        std::fs::write(dir.join("python.exe"), b"stub").expect("放假 python");
        let path = dir.as_os_str();

        let (key, value) =
            node_realpath_env_in("node tools/report.js", path, None).expect("node 命令要带上开关");
        assert_eq!(key, OsString::from("NODE_OPTIONS"));
        let value = value.to_string_lossy().to_string();
        // 两个开关都要：只开一个，另一半照样 realpath（真机上量过）。
        for flag in ["--preserve-symlinks", "--preserve-symlinks-main"] {
            assert!(value.contains(flag), "缺开关 {}：{}", flag, value);
        }
        assert!(
            node_realpath_env_in("node.exe tools/report.js", path, None).is_some(),
            "带扩展名写 node 也要认（标准兜底扩展名）"
        );
        let quoted = std::ffi::OsString::from(format!("\"{}\"", dir.display()));
        assert!(
            node_realpath_env_in("node tools/report.js", &quoted, None).is_some(),
            "带引号的 PATH 项也要能解析"
        );
        assert!(
            node_realpath_env_in("python tools/x.py", path, None).is_none(),
            "别的解释器不带它（免得在不需要的地方改模块解析语义）"
        );
        assert!(
            node_realpath_env_in("cmd /C echo hi", path, None).is_none(),
            "命令里没有 node 就不带"
        );
        assert!(
            node_realpath_env_in("cat C:\\nope\\nope.exe", path, None).is_none(),
            "数据文件路径不算程序（与解释器目录同一口径）"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 非 node 命令在三平台都不带解释器开关：那是 Windows 容器独有的补丁（另两个平台 realpath 正常）。
    #[test]
    fn non_node_commands_carry_no_interpreter_switch() {
        let spec = FenceSpec {
            agent: "a".to_string(),
            lease: String::new(),
            rw: Vec::new(),
            ro: Vec::new(),
            ro_tree: Vec::new(),
            private: PathBuf::new(),
            cwd: PathBuf::from("mods").join("m0"),
            net: false,
        };
        let keys: Vec<String> = fence_env(&spec, "python tools/x.py")
            .into_iter()
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .collect();
        assert!(
            !keys.iter().any(|k| k == "NODE_OPTIONS"),
            "非 node 命令不该带 NODE_OPTIONS：{:?}",
            keys
        );
    }

    /// 围栏边界说明只在围栏内**失败**时给：成功不打扰、没强制生效不谈边界。
    #[test]
    fn fence_boundary_note_only_on_enforced_failure() {
        let spec = FenceSpec {
            agent: "a".to_string(),
            lease: String::new(),
            rw: vec![PathBuf::from("demo").join("sandbox")],
            ro: Vec::new(),
            ro_tree: vec![PathBuf::from("mods").join("m0")],
            private: PathBuf::from("demo").join("sandbox"),
            cwd: PathBuf::from("mods").join("m0"),
            net: false,
        };
        assert!(fence_boundary_note(&spec, true, 0).is_none(), "成功不打扰");
        assert!(
            fence_boundary_note(&spec, false, 1).is_none(),
            "没强制生效就不谈边界"
        );
        let note = fence_boundary_note(&spec, true, 1).expect("围栏内失败要有边界说明");
        assert!(note.contains("范围外的访问被围栏拒绝"), "{}", note);
        assert!(note.contains("（网：关）"), "{}", note);
        assert!(note.contains("mods"), "要报出可达范围：{}", note);
    }

    /// 可达范围摘要：重复的根只报一次；根多时折叠，失败说明不因根多而失控。
    #[test]
    fn reachable_roots_dedupes_and_collapses() {
        let mut spec = FenceSpec {
            agent: "a".to_string(),
            lease: String::new(),
            rw: vec![PathBuf::from("r1")],
            ro: Vec::new(),
            ro_tree: vec![PathBuf::from("r1")],
            private: PathBuf::from("r1"),
            cwd: PathBuf::from("r1"),
            net: false,
        };
        assert_eq!(reachable_roots(&spec), "r1", "重复的根只报一次");
        spec.rw = (1..=6).map(|i| PathBuf::from(format!("r{}", i))).collect();
        let many = reachable_roots(&spec);
        assert!(many.starts_with("r1"), "{}", many);
        assert!(many.ends_with("等 6 处"), "{}", many);
    }

    /// 会话租约进归属身份：同一进程里同名 agent 的两个会话是**两个**归属，释放其中一个不许动另一个。
    #[test]
    fn owner_lease_tells_sessions_apart() {
        let mk = |lease: &str| Owner {
            pid: 1,
            start: 2,
            lease: lease.to_string(),
        };
        let a = mk("s1");
        let b = mk("s2");
        assert_eq!(a, a.clone(), "完全相同的归属用于去重/撤销");
        assert_ne!(a, b, "同进程的两个会话是两个归属");
        assert_eq!(b.lease, "s2");
    }
}
