//! 目的：依赖补齐能力的实现——清单解析、状态、下载缓存、解包落盘与一次后台作业。
//! 管：内嵌 dependencies.yaml（清单）与 links.yaml（台账）的解析、按本机筛选、下载 / 解包 / 校验、作业状态与取消。
//! 不管：虚拟机档的能力语义（在 vm）；界面怎么展示（在 presentation）。
//! 联动：呈现层经 api::Provision 用它；组合根构造它；下载走 kernel::detail::outbound，解包走 kernel::detail::unpack。

use super::api::{DepStatus, GuestBase, JobState, Provision};
use crate::capabilities::vm::api::Vm;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// 清单与台账在构建期嵌进二进制：分发的二进制不依赖仓库里的文件，读到的仍是同一份真相。
const DEPS_YAML: &str = include_str!("../../../dependencies.yaml");
const LINKS_YAML: &str = include_str!("../../../links.yaml");

#[derive(Debug, Clone, Deserialize)]
struct DepsFile {
    deps: Vec<Dep>,
}

/// 目的：缺省安装方式——解归档。
fn default_kind() -> String {
    "archive".to_string()
}

/// 目的：一条依赖——它是什么、装到哪、怎么判在场、从哪来、怎么装。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Dep {
    pub id: String,
    pub os: String,
    pub arch: String,
    pub install: String,
    pub probe: String,
    pub source: String,
    /// 目的：安装方式：archive（默认，解归档）/ appimage / file（单文件直下）/ guest-rootfs（多步构造）/
    ///   toolchain（基础工具链：装进 .tools/toolchains 并登记 active.json）。
    #[serde(default = "default_kind")]
    pub kind: String,
    /// 目的：归属的虚拟根名（只有 guest 类条目有）；一个虚拟根可以由多条条目共同装。
    #[serde(default)]
    pub guest: Option<String>,
    /// 目的：kind: toolchain 的载荷；缺了就是清单写错（安装时如实拒绝，不猜版本或布局）。
    #[serde(default)]
    pub toolchain: Option<ToolchainSpec>,
    /// 目的：Linux 虚拟根构造用的发行版族（只有 guest-rootfs 有）；没有声明 = 没有构造配方。
    #[serde(default)]
    pub family: Option<String>,
    /// 目的：解包时剥掉归档里的前 N 段目录（便携包常带一层包装目录）；缺省 0。
    #[serde(default)]
    pub strip: usize,
    #[serde(default)]
    pub size_mb: u64,
    /// 目的：这条依赖的归档 / 单文件 sha256（64 位十六进制）——**与它守护的那条连接写在同一条**：
    ///   自托管 release 资产写在这里，链接类源写在 `links.yaml` 的 `<键>.sha256`。
    ///   空 = 没 pin（`latest` 这种会随上游滚动的源只能这样），如实不校验，不假装验过。
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub note: String,
}

/// 目的：基础工具链条目的载荷（kind: toolchain）——装哪个工具、什么版本、哪些 bin 目录进 active.json。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ToolchainSpec {
    /// 目的：active.json 的工具 id（python / node / cc）。
    pub tool: String,
    /// 目的：安装布局与 active.json 里的版本（Zig 是 Zig 的版本，MinGW 是 GCC 的版本）。
    pub version: String,
    /// 目的：宿主目标名（与 install 路径同一套写法）；安装时与当前宿主比对，不符即拒绝。
    #[serde(default)]
    pub target: Option<String>,
    /// 目的：相对安装目录的 bin 目录（按顺序写进 active.json；启动期把它们前置到 PATH）。
    #[serde(default)]
    pub bins: Vec<String>,
    /// 目的：装完是否写 cc / gcc / g++ / clang 包装——Zig 包用它把 zig cc 装成常规编译器名。
    #[serde(default)]
    pub wrappers: bool,
}

/// 目的：解析内嵌的依赖清单。
pub fn deps() -> Result<Vec<Dep>, String> {
    let parsed: DepsFile =
        yaml_serde::from_str(DEPS_YAML).map_err(|e| format!("dependencies.yaml 非法：{}", e))?;
    Ok(parsed.deps)
}

/// 目的：解析内嵌的下载源台账（扁平的 键: https://…）。
pub fn links() -> Result<BTreeMap<String, String>, String> {
    yaml_serde::from_str(LINKS_YAML).map_err(|e| format!("links.yaml 非法：{}", e))
}

/// 目的：把 source 解析成可下载的 URL——`release:<tag>/<资产名>` 走自托管包，其余按 links.yaml 的键。
pub fn source_url(source: &str, links: &BTreeMap<String, String>) -> Result<String, String> {
    if let Some(asset) = source.strip_prefix("release:") {
        // 优先用调用方给的台账（用例注入本地源）；生产传的就是内嵌那份，结果一致。
        let base = links
            .get("release.base")
            .cloned()
            .or_else(|| link("release.base"))
            .ok_or_else(|| "links.yaml 缺 release.base".to_string())?;
        return Ok(format!("{}/{}", base.trim_end_matches('/'), asset));
    }
    links
        .get(source)
        .cloned()
        .ok_or_else(|| format!("links.yaml 里没有这个键：{}", source))
}

/// 目的：按台账键取 URL——Rust 侧与 Node 侧读同一份台账；这里也是台账消费点（T0 扫 `link("键")`）。
fn link(key: &str) -> Option<String> {
    links().ok().and_then(|m| m.get(key).cloned())
}

/// 目的：按组件把清单里的相对路径落到产品根下（不用字符串拼分隔符）。
pub(super) fn under(root: &Path, rel: &str) -> PathBuf {
    let mut out = root.to_path_buf();
    for seg in rel.split('/').filter(|s| !s.is_empty()) {
        out.push(seg);
    }
    out
}

/// 目的：一个虚拟根画像里我们要的几项（id / os / arch / runtimes）。
#[derive(Debug, Default, PartialEq, Eq)]
struct GuestProfile {
    id: Option<String>,
    os: Option<String>,
    arch: Option<String>,
    /// 目的：画像里声明的运行能力（内联列表 `runtimes: [python, node, cc, gcc]`；缺画像 = 空）。
    runtimes: Vec<String>,
}

/// 目的：读虚拟根画像里的 `id` / `os` / `arch` / `runtimes`（`guest.yaml`；不在场 = 空画像，调用方给缺省）。
/// 约束：只读几个扁平标量与一个内联列表，不引 YAML 依赖（与 links.yaml / 缺口账同一手法）。
fn read_guest_profile(base: &Path) -> GuestProfile {
    let Ok(text) = fs::read_to_string(base.join("guest.yaml")) else {
        return GuestProfile::default();
    };
    let mut p = GuestProfile::default();
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let v = v.trim().trim_matches('"');
        if v.is_empty() {
            continue;
        }
        match k.trim() {
            "id" => p.id = Some(v.to_string()),
            "os" => p.os = Some(v.to_string()),
            "arch" => p.arch = Some(v.to_string()),
            "runtimes" => {
                p.runtimes = v
                    .trim_matches(['[', ']'])
                    .split(',')
                    .map(|s| s.trim().trim_matches('"'))
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect();
            }
            _ => {}
        }
    }
    p
}

/// 目的：本机（os/arch）要用的依赖清单（含 `any`）。
pub fn for_host(deps: &[Dep], os: &str, arch: &str) -> Vec<Dep> {
    deps.iter()
        .filter(|d| (d.os == os || d.os == "any") && (d.arch == arch || d.arch == "any"))
        .cloned()
        .collect()
}

/// 目的：宿主目标名——与清单 `toolchain.target`、安装布局同一套写法（windows-x86_64 等）。
pub(crate) fn host_target() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// 目的：这条依赖是不是基础工具链（运行前提：每次启动都要读 active.json）。
fn is_toolchain(dep: &Dep) -> bool {
    dep.kind == "toolchain"
}

/// 目的：这条依赖的 probe 是否已在场（文件或目录；与 status 的判据同一把尺子）。
fn probe_present(root: &Path, dep: &Dep) -> bool {
    let p = under(root, &dep.probe);
    p.is_file() || p.is_dir()
}

/// 目的：`--ensure` 取值的两种含义——依赖 id 列表（原语义），或按清单 kind 的组别名（只补缺项）。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum EnsureSpec {
    Ids(Vec<String>),
    Kinds(Vec<String>),
}

/// 目的：解析 `--ensure=<取值>`：全为清单里的 kind 时按组别名解释；否则原样当 id 列表。
/// 约束：kind 与 id 混写如实拒绝（两种语义装的不是一回事，猜一种会装错东西）；
///   未知 id 交给 run 的「没有匹配」拒绝。
pub(crate) fn split_ensure_spec(all: &[Dep], spec: &str) -> Result<EnsureSpec, String> {
    let tokens: Vec<String> = spec
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let mut kinds: Vec<&str> = all.iter().map(|d| d.kind.as_str()).collect();
    kinds.sort_unstable();
    kinds.dedup();
    let is_kind = |t: &str| kinds.contains(&t);
    if !tokens.is_empty() && tokens.iter().all(|t| is_kind(t)) {
        return Ok(EnsureSpec::Kinds(tokens));
    }
    if tokens.iter().any(|t| is_kind(t)) {
        return Err(format!(
            "--ensure 不能混写 kind（{}）与依赖 id：{}",
            kinds.join(" / "),
            spec
        ));
    }
    Ok(EnsureSpec::Ids(tokens))
}

/// 目的：按 kind 组取「还没在场」的条目 id——组别名只补缺项，已在位的不重装（260 MB 的包不重解）。
pub(crate) fn pending_of_kinds(all: &[Dep], root: &Path, kinds: &[String]) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for dep in all {
        if kinds.iter().any(|k| k == &dep.kind)
            && !probe_present(root, dep)
            && !ids.contains(&dep.id)
        {
            ids.push(dep.id.clone());
        }
    }
    ids
}

/// 目的：把两段相对路径拼起来（统一用 `/` 书写，与清单同一套写法）；空段忽略，不产生 `//`。
fn join_rel(base: &str, rel: &str) -> String {
    let mut out = base.trim_end_matches('/').to_string();
    for seg in rel.split('/').filter(|s| !s.is_empty()) {
        out.push('/');
        out.push_str(seg);
    }
    out
}

/// 目的：把**已在场**的基础工具链登记进 `.tools/toolchains/active.json`（启动期只读这一份，见 entry::toolchains）。
/// 约束：以磁盘为准而不是以「这次装了什么」为准——先装 python 后装 node 时，两次都要登记到同一份；
///   有 bin 目录不在场的工具如实不登记（启动期据此报缺哪个，不假装装上）；写盘走临时文件再改名。
pub(crate) fn sync_toolchain_active(root: &Path, all: &[Dep]) -> Result<(), String> {
    let target = host_target();
    let mut profile: Vec<String> = Vec::new();
    let mut tools: Vec<serde_json::Value> = Vec::new();
    for dep in all.iter().filter(|d| is_toolchain(d)) {
        let spec = dep
            .toolchain
            .as_ref()
            .ok_or_else(|| format!("{}：kind: toolchain 缺 toolchain 段", dep.id))?;
        if spec.tool.trim().is_empty() || spec.version.trim().is_empty() || spec.bins.is_empty() {
            return Err(format!(
                "{}：toolchain 段的 tool / version / bins 都不能空",
                dep.id
            ));
        }
        if let Some(want) = &spec.target {
            if want != &target {
                return Err(format!(
                    "{}：工具链目标不符（清单 {}，当前 {}）",
                    dep.id, want, target
                ));
            }
        }
        if !profile.contains(&spec.tool) {
            profile.push(spec.tool.clone());
        }
        let bins: Vec<String> = spec
            .bins
            .iter()
            .map(|b| join_rel(&dep.install, b))
            .collect();
        let present = bins.iter().all(|b| under(root, b).is_dir());
        if present {
            tools.push(serde_json::json!({
                "id": spec.tool,
                "version": spec.version,
                "bin": bins,
                "env": {},
            }));
        }
    }
    let file = root.join(".tools").join("toolchains").join("active.json");
    let dir = file
        .parent()
        .ok_or_else(|| "active.json 没有父目录".to_string())?;
    fs::create_dir_all(dir).map_err(|e| format!("建 {} 失败：{}", dir.display(), e))?;
    let doc = serde_json::json!({ "target": target, "profile": profile, "tools": tools });
    let text = serde_json::to_string_pretty(&doc)
        .map_err(|e| format!("序列化 active.json 失败：{}", e))?;
    let tmp = file.with_extension("json.tmp");
    fs::write(&tmp, text + "\n").map_err(|e| format!("写 {} 失败：{}", tmp.display(), e))?;
    fs::rename(&tmp, &file).map_err(|e| format!("提交 {} 失败：{}", file.display(), e))?;
    Ok(())
}

/// 目的：依赖补齐的实现——持产品根、作业状态、取消旗与（可选的）vm 能力面；下载 / 解包在下面同一文件。
pub struct ProvisionService {
    root: PathBuf,
    job: Arc<Mutex<JobState>>,
    cancel: Arc<AtomicBool>,
    /// Windows guest 的运行期编排要起 guest；组合根或 --provision 入口注入；只查状态时可为 None。
    vm: Option<Arc<dyn Vm + Send + Sync>>,
}

impl ProvisionService {
    /// 目的：以产品根建一个实现（只查状态、或只装不启动 guest 的条目用；guest-runtime 会被如实拒绝）。
    pub fn new(root: &Path) -> ProvisionService {
        ProvisionService {
            root: root.to_path_buf(),
            job: Arc::new(Mutex::new(JobState::default())),
            cancel: Arc::new(AtomicBool::new(false)),
            vm: None,
        }
    }

    /// 目的：带 vm 能力面的实现（组合根 / --provision 入口用）；guest-runtime 编排需要它。
    pub fn with_vm(root: &Path, vm: Arc<dyn Vm + Send + Sync>) -> ProvisionService {
        ProvisionService {
            vm: Some(vm),
            ..ProvisionService::new(root)
        }
    }

    /// 目的：同步补齐（CLI 用）：逐条下载 / 解包 / 校验，进度逐行回调。
    pub fn ensure(&self, ids: &[String], on_line: &mut dyn FnMut(&str)) -> Result<(), String> {
        run_ensure(&self.root, ids, &self.cancel, on_line, self.vm.as_deref())
    }

    /// 目的：`--ensure=<取值>` 的补齐入口：kind 组别名只补缺项（如 `toolchain`），
    ///   id 列表按原语义点名安装（空列表 = 清单全部）。
    /// 约束：组别名一条都不缺时如实说「都在场」且不碰网络；在位项不重装。
    pub fn ensure_spec(&self, spec: &str, on_line: &mut dyn FnMut(&str)) -> Result<(), String> {
        let all = for_host(&deps()?, std::env::consts::OS, std::env::consts::ARCH);
        match split_ensure_spec(&all, spec)? {
            EnsureSpec::Ids(ids) => self.ensure(&ids, on_line),
            EnsureSpec::Kinds(kinds) => {
                let pending = pending_of_kinds(&all, &self.root, &kinds);
                if pending.is_empty() {
                    on_line(&format!("{}：没有缺项，无需补齐", kinds.join(" / ")));
                    return Ok(());
                }
                on_line(&format!(
                    "{}：补 {} 条（{}）",
                    kinds.join(" / "),
                    pending.len(),
                    pending.join(" / ")
                ));
                run(
                    &self.root,
                    &all,
                    &links()?,
                    &pending,
                    &self.cancel,
                    on_line,
                    self.vm.as_deref(),
                )
            }
        }
    }
}

impl Provision for ProvisionService {
    fn status(&self) -> Result<Vec<DepStatus>, String> {
        let links = links()?;
        let mut out = Vec::new();
        for dep in for_host(&deps()?, std::env::consts::OS, std::env::consts::ARCH) {
            let probe = under(&self.root, &dep.probe);
            let url = source_url(&dep.source, &links).ok();
            out.push(DepStatus {
                id: dep.id,
                guest: dep.guest.clone(),
                present: probe.is_file() || probe.is_dir(),
                install: dep.install,
                probe: dep.probe,
                source: dep.source,
                url,
                size_mb: dep.size_mb,
                note: dep.note,
            });
        }
        Ok(out)
    }

    fn guest_bases(&self) -> Result<Vec<GuestBase>, String> {
        self.guest_bases_of(&for_host(
            &deps()?,
            std::env::consts::OS,
            std::env::consts::ARCH,
        ))
    }

    /// 约束：账本是"我们亲手起的长驻子进程"这一份事实源；`alive` 逐条问一次系统（死条目如实留着，等对账清）。
    fn processes(&self) -> Result<Vec<crate::capabilities::provision::api::ProcInfo>, String> {
        let book = crate::kernel::detail::procs::ProcessBook::open(&self.root.join(".home"));
        Ok(book
            .list()
            .into_iter()
            .map(|e| crate::capabilities::provision::api::ProcInfo {
                alive: crate::kernel::detail::procs::alive(e.pid),
                pid: e.pid,
                purpose: e.purpose,
                owner: e.owner,
                since: e.since,
                orphan_risk: e.guard_note.is_some(),
                guard_note: e.guard_note,
            })
            .collect())
    }

    fn job(&self) -> JobState {
        self.job.lock().map(|j| j.clone()).unwrap_or_default()
    }

    fn start(&self, ids: Vec<String>) -> Result<(), String> {
        {
            let mut j = self.job.lock().map_err(|_| "作业状态锁坏了".to_string())?;
            if j.running {
                return Err("已有补齐作业在跑".to_string());
            }
            *j = JobState {
                running: true,
                seq: 0,
                lines: Vec::new(),
                error: None,
            };
        }
        self.cancel.store(false, Ordering::Relaxed);
        let root = self.root.clone();
        let job = Arc::clone(&self.job);
        let cancel = Arc::clone(&self.cancel);
        let vm = self.vm.clone();
        std::thread::spawn(move || {
            let sink = Arc::clone(&job);
            let mut on_line = move |line: &str| push_line(&sink, line);
            let r = run_ensure(&root, &ids, &cancel, &mut on_line, vm.as_deref());
            if let Ok(mut j) = job.lock() {
                j.running = false;
                if let Err(e) = r {
                    j.error = Some(e);
                }
            }
        });
        Ok(())
    }

    fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    fn reset_guest(&self, guest: &str, on_line: &mut dyn FnMut(&str)) -> Result<(), String> {
        let all = for_host(&deps()?, std::env::consts::OS, std::env::consts::ARCH);
        let picked: Vec<&Dep> = all
            .iter()
            .filter(|d| d.guest.as_deref() == Some(guest))
            .collect();
        if picked.is_empty() {
            return Err(format!(
                "虚拟根「{}」不是产品清单构造的（清单里没有它的 guest 条目），不能自动还原",
                guest
            ));
        }
        // 有产品起着的 guest 就不动：删除目录会踩到正在读基础镜像的 QEMU。
        let book = crate::kernel::detail::procs::ProcessBook::open(&self.root.join(".home"));
        if book
            .list()
            .iter()
            .any(|e| e.purpose == "guest" && crate::kernel::detail::procs::alive(e.pid))
        {
            return Err("还有 guest 在跑：先停止所有虚拟机会话，再还原虚拟根".to_string());
        }
        let dir = crate::capabilities::vm::api::resolve_guest(&self.root, guest)?;
        let lock = self
            .root
            .join(".vm")
            .join("build-cache")
            .join(format!("{}.lock", guest));
        if lock.exists() {
            return Err("这个虚拟根有构建在跑：等构建结束再还原".to_string());
        }
        if dir.is_dir() {
            on_line(&format!("删除旧虚拟根 {}", guest));
            fs::remove_dir_all(&dir)
                .map_err(|e| format!("删除旧虚拟根失败（{}）：{}", dir.display(), e))?;
        }
        let ids: Vec<String> = picked.iter().map(|d| d.id.clone()).collect();
        on_line(&format!("按清单重建 {}（{} 条依赖）", guest, ids.len()));
        self.ensure(&ids, on_line)
    }
}

impl ProvisionService {
    /// 目的：在给定清单上归并虚拟根（清单由调用方给，便于用例注入；`guest_bases` 只是它的"本机清单"版）。
    pub(super) fn guest_bases_of(&self, deps: &[Dep]) -> Result<Vec<GuestBase>, String> {
        let mut order: Vec<String> = Vec::new();
        let mut by_guest: BTreeMap<String, Vec<Dep>> = BTreeMap::new();
        for dep in deps.iter().cloned() {
            let Some(name) = dep.guest.clone() else {
                continue;
            };
            if !by_guest.contains_key(&name) {
                order.push(name.clone());
            }
            by_guest.entry(name).or_default().push(dep);
        }
        let mut out = Vec::new();
        for name in order {
            let members = &by_guest[&name];
            let missing: Vec<String> = members
                .iter()
                .filter(|d| {
                    let p = under(&self.root, &d.probe);
                    !p.is_file() && !p.is_dir()
                })
                .map(|d| d.id.clone())
                .collect();
            let install = members
                .first()
                .map(|d| d.install.clone())
                .unwrap_or_default();
            let base = under(&self.root, &install);
            let profile = read_guest_profile(&base);
            out.push(GuestBase {
                guest: name.clone(),
                label: profile.id.unwrap_or_else(|| name.clone()),
                os: profile.os.unwrap_or_else(|| "linux".to_string()),
                arch: profile
                    .arch
                    .unwrap_or_else(|| std::env::consts::ARCH.to_string()),
                // 注：guest.yaml 的 arch 用 amd64 / arm64 写法，这里原样带出，界面只当标签用。
                runtimes: profile.runtimes,
                ready: missing.is_empty(),
                missing,
                install,
            });
        }
        Ok(out)
    }
}

/// 目的：把一行进度写进作业状态——`seq` 单调递增（前端按它增量取），行窗口只留最近 200 行。
fn push_line(job: &Mutex<JobState>, line: &str) {
    if let Ok(mut j) = job.lock() {
        j.seq += 1;
        j.lines.push(line.to_string());
        if j.lines.len() > 200 {
            j.lines.remove(0);
        }
    }
}

/// 目的：补齐的执行核心——选中依赖、逐条下载(带缓存) + 解包 + 校验；作业与同步入口共用。
fn run_ensure(
    root: &Path,
    ids: &[String],
    cancel: &AtomicBool,
    on_line: &mut dyn FnMut(&str),
    vm: Option<&(dyn Vm + Send + Sync)>,
) -> Result<(), String> {
    let all = deps()?;
    let links = links()?;
    let all = for_host(&all, std::env::consts::OS, std::env::consts::ARCH);
    run(root, &all, &links, ids, cancel, on_line, vm)
}

/// 目的：在给定的依赖清单上执行补齐（清单/台账由调用方给，便于用例注入）。
fn run(
    root: &Path,
    all: &[Dep],
    links: &BTreeMap<String, String>,
    ids: &[String],
    cancel: &AtomicBool,
    on_line: &mut dyn FnMut(&str),
    vm: Option<&(dyn Vm + Send + Sync)>,
) -> Result<(), String> {
    let picked: Vec<&Dep> = if ids.is_empty() {
        all.iter().collect()
    } else {
        all.iter()
            .filter(|d| ids.iter().any(|x| x == &d.id))
            .collect()
    };
    if picked.is_empty() {
        return Err("没有匹配的依赖 id（用 --provision 看清单）".to_string());
    }
    // 显式点名 vs 全量补齐：运行期编排的失败在两种情形下处置不同（见下面 guest-runtime 分支）。
    let explicit = !ids.is_empty();
    let agent = crate::kernel::detail::outbound::agent(15, 7200);
    let cache = root.join(".vm").join("cache");
    fs::create_dir_all(&cache).map_err(|e| format!("建缓存目录失败：{}", e))?;
    for dep in picked {
        if cancel.load(Ordering::Relaxed) {
            on_line("已取消");
            return Ok(());
        }
        // 基础工具链是每次启动都要用的运行前提：全量补齐时已在场就不重装
        // （260 MB 的 MinGW 重解一遍没有意义）；其余 kind 的「在位即跳过」由调用方筛 pending。
        if is_toolchain(dep) && !explicit && probe_present(root, dep) {
            on_line(&format!("{}：已就位（跳过）", dep.id));
            continue;
        }
        let url = source_url(&dep.source, links)?;
        let asset = asset_name(&url);
        let cached = cache.join(&asset);
        // 运行期编排自己不下东西：它要的镜像必须已由 guest-disk 装好；缺了编排会如实拒绝。
        if dep.kind != "guest-runtime" {
            if cached.is_file() {
                on_line(&format!("{}：用缓存 {}", dep.id, asset));
            } else {
                on_line(&format!("{}：下载 {}", dep.id, url));
                download(&agent, &url, &cached, on_line)?;
            }
            // 完整性闸：缓存里那份和刚下好的一样都要验（缓存可能是上次中断留下的）。
            let pinned = pinned_sha256(dep, links);
            verify_cached(&dep.id, pinned.as_deref(), &cached, &url)?;
        }
        let dest = under(root, &dep.install);
        fs::create_dir_all(&dest).map_err(|e| format!("建安装目录 {} 失败：{}", dep.install, e))?;
        let installed =
            match dep.kind.as_str() {
                "appimage" => install_appimage(&cached, &dest, on_line)
                    .map_err(|e| format!("{}：{}", dep.id, e)),
                "file" => install_file(&cached, &dest, &dep.probe)
                    .map_err(|e| format!("{}：{}", dep.id, e)),
                "guest-disk" => install_guest_disk(&cached, &dest, &dep.probe, on_line)
                    .map_err(|e| format!("{}：{}", dep.id, e)),
                "guest-rootfs" => install_guest_rootfs(root, dep, &cached, &agent, on_line)
                    .map_err(|e| format!("{}：{}", dep.id, e)),
                "toolchain" => install_toolchain(&cached, &dest, dep, on_line)
                    .map_err(|e| format!("{}：{}", dep.id, e)),
                "guest-runtime" => {
                    let r = super::guest::install_guest_runtime(root, dep, vm, explicit, on_line);
                    match (r, explicit) {
                        (Ok(()), _) => Ok(()),
                        // 全量补齐（没点名）时：起不了 guest 是**机器事实**（缺 KVM / WHPX 这类），
                        // 不该拖垮其余依赖——如实说清并继续；这一条的探针仍然没满足，就绪状态照实报。
                        (Err(e), false) => {
                            on_line(&format!("{}：运行期编排跳过（{}）", dep.id, e));
                            continue;
                        }
                        // 显式点名这一条 = 你要的就是它：失败即失败。
                        (Err(e), true) => Err(format!("{}：{}", dep.id, e)),
                    }
                }
                _ => crate::kernel::detail::unpack::extract_into(&cached, &dest, dep.strip)
                    .map_err(|e| format!("{}：解包失败：{}", dep.id, e)),
            };
        installed?;
        let probe = under(root, &dep.probe);
        if !(probe.is_file() || probe.is_dir()) {
            return Err(format!("{}：装完仍找不到 {}", dep.id, dep.probe));
        }
        // 归档里的 probe 就是入口程序：zip 可能没记 unix 权限位，解出来会是 0644——补可执行位。
        if dep.kind == "archive" {
            ensure_executable(&probe);
        }
        on_line(&format!("{}：就绪（{}）", dep.id, dep.install));
    }
    // 装完就把基础工具链登记进 active.json：启动期只读这一份（entry::toolchains）；
    // 单独点名装一条时也要刷新，所以登记以磁盘为准（见 sync_toolchain_active）。
    if all.iter().any(is_toolchain) {
        sync_toolchain_active(root, all)?;
    }
    Ok(())
}

/// 目的：从 URL 取末段做缓存文件名（去掉查询串与锚点）。
fn asset_name(url: &str) -> String {
    url.split(['?', '#'])
        .next()
        .unwrap_or(url)
        .rsplit('/')
        .next()
        .unwrap_or("asset")
        .to_string()
}

/// 目的：把 URL 下到目标文件——选源（前缀镜像测速）后带续传下载；半截文件留作下次续传的断点。
/// 代理：ureq 的默认配置从环境拾取（HTTPS_PROXY / HTTP_PROXY / NO_PROXY），这里不另写一套。
fn download(
    agent: &ureq::Agent,
    url: &str,
    dest: &Path,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let picked = pick_source(agent, &source_candidates(url), on_line)?;
    download_with_resume(agent, &picked, dest, on_line)
}

/// 目的：下载候选——上游 URL 在前；配了 SOLOMNI_GH_MIRROR 时再加一条前缀镜像（只对 GitHub 资产）。
fn source_candidates(upstream: &str) -> Vec<String> {
    let mirror = std::env::var("SOLOMNI_GH_MIRROR").unwrap_or_default();
    let mut out = vec![upstream.to_string()];
    if let Some(m) = mirror_of(upstream, &mirror) {
        out.push(m);
    }
    out
}

/// 目的：前缀镜像的写法——`<镜像根>/github.com/<owner>/<repo>/...`；不是 GitHub 资产或没配镜像就不加。
fn mirror_of(upstream: &str, mirror: &str) -> Option<String> {
    let root = mirror.trim().trim_end_matches('/');
    let rest = upstream.strip_prefix("https://github.com/")?;
    if root.is_empty() {
        return None;
    }
    Some(format!("{}/github.com/{}", root, rest))
}

/// 目的：选源——只有一个候选就直接用（没有选择时测速是白花的时间）；多个时各探 2 MB，取最快且可达的。
fn pick_source(
    agent: &ureq::Agent,
    urls: &[String],
    on_line: &mut dyn FnMut(&str),
) -> Result<String, String> {
    if urls.len() <= 1 {
        return urls
            .first()
            .cloned()
            .ok_or_else(|| "没有下载源".to_string());
    }
    on_line(&format!("测速选源（{} 个候选，各探 2 MB）...", urls.len()));
    let mut samples: Vec<(String, u16, f64)> = Vec::new();
    for u in urls {
        let (code, kbps) = probe_speed(agent, u);
        on_line(&format!("  {:.0} KB/s  [http {}]  {}", kbps, code, u));
        samples.push((u.clone(), code, kbps));
    }
    choose_fastest(&samples)
}

/// 目的：从探测结果里选源——2xx 且有数据的最快者；都不行时把状态码含义说清（不静默换源硬试）。
fn choose_fastest(samples: &[(String, u16, f64)]) -> Result<String, String> {
    let best = samples
        .iter()
        .filter(|(_, code, kbps)| (200..300).contains(code) && *kbps > 0.0)
        .max_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
    if let Some((url, _, _)) = best {
        return Ok(url.clone());
    }
    let codes: Vec<String> = samples.iter().map(|(_, c, _)| c.to_string()).collect();
    Err(format!(
        "没有可用的下载源（HTTP {}）：000 = 网络层失败（配 HTTPS_PROXY，或浏览器下载后放进缓存），403 = 被限流，404 = 该源没有这个资产",
        codes.join(" / ")
    ))
}

/// 目的：一次 2 MB 的范围 GET，回 (状态码, KB/s)；网络层失败记 000（与 curl 的观感一致）。
/// 约束：非 2xx 再试一次——受限网络上瞬时重置很常见，只探一次会把可达源误杀。
fn probe_speed(agent: &ureq::Agent, url: &str) -> (u16, f64) {
    let first = probe_speed_once(agent, url);
    if (200..300).contains(&first.0) {
        return first;
    }
    probe_speed_once(agent, url)
}

fn probe_speed_once(agent: &ureq::Agent, url: &str) -> (u16, f64) {
    let started = Instant::now();
    let resp = match agent.get(url).header("Range", "bytes=0-2097151").call() {
        Ok(r) => r,
        Err(_) => return (0, 0.0),
    };
    let code = resp.status().as_u16();
    if !(200..300).contains(&code) {
        return (code, 0.0);
    }
    let mut reader = resp.into_body().into_reader().take(2 * 1024 * 1024);
    let bytes = io::copy(&mut reader, &mut io::sink()).unwrap_or(0);
    let secs = started.elapsed().as_secs_f64().max(0.001);
    (code, bytes as f64 / 1024.0 / secs)
}

/// 目的：分片路径（`.part`）——下载先写它再改名，半截文件不会被当成缓存。
fn part_path(dest: &Path) -> PathBuf {
    dest.with_extension("part")
}

/// 目的：分片的来源标记——续传必须打向产出这份分片的那条源（换源续等于把两份字节接起来）。
fn part_source_marker(part: &Path) -> PathBuf {
    let mut s = part.as_os_str().to_os_string();
    s.push(".src");
    PathBuf::from(s)
}

/// 目的：带续传的下载——`.part` 在场且来源标记与本次 URL 一致就从断点续（Range）；
///   来源不明 / 换了源就从头下；服务端忽略 Range（回 200）时从零重写。
/// 约束：失败时保留分片与来源标记（续传证据），成功才改名并删标记。
fn download_with_resume(
    agent: &ureq::Agent,
    url: &str,
    dest: &Path,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let part = part_path(dest);
    let marker = part_source_marker(&part);
    let same_source = fs::read_to_string(&marker)
        .map(|s| s.trim() == url)
        .unwrap_or(false);
    let offset = if part.is_file() {
        if same_source {
            fs::metadata(&part).map(|m| m.len()).unwrap_or(0)
        } else {
            let _ = fs::remove_file(&part);
            0
        }
    } else {
        0
    };
    if offset == 0 {
        let _ = fs::remove_file(&marker);
    }
    fs::write(&marker, url).map_err(|e| format!("写来源标记 {} 失败：{}", marker.display(), e))?;
    let mut req = agent.get(url);
    if offset > 0 {
        req = req.header("Range", format!("bytes={}-", offset));
    }
    let resp = match req.call() {
        Ok(r) => r,
        Err(e) => return Err(format!("下载失败（{}）：{}（分片保留，重跑续传）", url, e)),
    };
    let code = resp.status().as_u16();
    if !(200..300).contains(&code) {
        return Err(format!(
            "下载 {} 返回 {}（分片保留，重跑续传；要换源先删分片与来源标记 {}）",
            url,
            code,
            part.display()
        ));
    }
    let append = offset > 0 && code == 206;
    let mut opts = fs::OpenOptions::new();
    opts.create(true).write(true);
    if append {
        opts.append(true);
    } else {
        opts.truncate(true);
    }
    let mut f = opts
        .open(&part)
        .map_err(|e| format!("建 {} 失败：{}", part.display(), e))?;
    let mut reader = resp.into_body().into_reader();
    let mut added = 0u64;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let got = reader
            .read(&mut buf)
            .map_err(|e| format!("读下载流失败（{}）：{}", url, e))?;
        if got == 0 {
            break;
        }
        io::Write::write_all(&mut f, &buf[..got])
            .map_err(|e| format!("写 {} 失败：{}", part.display(), e))?;
        added += got as u64;
    }
    drop(f);
    fs::rename(&part, dest).map_err(|e| format!("改名 {} 失败：{}", part.display(), e))?;
    let _ = fs::remove_file(&marker);
    if append {
        on_line(&format!(
            "续传完成：断点 {} 字节 + 新增 {} 字节",
            offset, added
        ));
    } else {
        on_line(&format!("下载完成：{} 字节", added));
    }
    Ok(())
}

/// 目的：一个文件的 sha256（小写十六进制）——依赖补齐的完整性判据。
/// 约束：复用 workspace 的 `content_hash`（项目里唯一那份零依赖 SHA-256，见其模块注释：
///   "不引入加密库（项目依赖刻意保持最小）"）。整份读进内存是一次性的补齐全动作，最多几百 MB——
///   宁可这样，也不引第二套算法或加密库。
pub(crate) fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("读不了 {}：{}", path.display(), e))?;
    Ok(crate::capabilities::workspace::api::content_hash(&bytes))
}

/// 目的：这条依赖 pin 的 sha256——**校验值与它守护的那条连接写在同一条**：
///   自托管 release 资产在清单条目里（`sha256:`），链接类源在 `links.yaml` 的 `<source>.sha256`。
/// 返回：小写十六进制；没 pin 给 None（`latest` 这种会滚动的源只能这样，如实不校验）。
fn pinned_sha256(dep: &Dep, links: &BTreeMap<String, String>) -> Option<String> {
    let own = dep.sha256.trim();
    if !own.is_empty() {
        return Some(own.to_ascii_lowercase());
    }
    links
        .get(&format!("{}.sha256", dep.source))
        .map(|v| v.trim().to_ascii_lowercase())
}

/// 目的：缓存里那份文件与 pin 的 sha256 一致吗——**下载完要验，复用旧缓存也要验**。
/// 参数：id = 依赖 id（报错点名）；pinned = pin 的校验值（None = 没 pin）；cached = 缓存文件；url = 下载源。
/// 约束：不符时**不删缓存**——删了既丢证据、又会在 pin 过期时变成"每次重下 165 MB"的死循环，改由人看结论决定。
fn verify_cached(id: &str, pinned: Option<&str>, cached: &Path, url: &str) -> Result<(), String> {
    let Some(want) = pinned else {
        return Ok(());
    };
    let want = want.trim().to_ascii_lowercase();
    let got = sha256_file(cached)?;
    if got == want {
        return Ok(());
    }
    Err(format!(
        "{}：缓存文件的 sha256 与台账不符\n  台账 {}\n  实际 {}\n  来源 {}\n  上游换过资产就更新台账（来源见该条目的 sha256 注释 / links.yaml 的 <键>.sha256 注释）；否则删掉 {} 重下",
        id,
        want,
        got,
        url,
        cached.display()
    ))
}

/// 目的：基础工具链条目——解归档到安装目录；Zig 包再写 cc / gcc / g++ / clang 包装。
fn install_toolchain(
    cached: &Path,
    dest: &Path,
    dep: &Dep,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let spec = dep
        .toolchain
        .as_ref()
        .ok_or_else(|| "kind: toolchain 缺 toolchain 段".to_string())?;
    crate::kernel::detail::unpack::extract_into(cached, dest, dep.strip)?;
    if spec.wrappers {
        write_zig_wrappers(dest)?;
        on_line("已写 cc / gcc / g++ / clang 包装（Zig 提供）");
    }
    Ok(())
}

/// 目的：在安装目录里找 Zig 可执行文件（包根目录名带平台与版本，不写死）。
fn find_zig(root: &Path) -> Option<PathBuf> {
    fn walk(dir: &Path, depth: usize) -> Option<PathBuf> {
        if depth > 4 {
            return None;
        }
        let mut subdirs = Vec::new();
        for e in fs::read_dir(dir).ok()? {
            let e = e.ok()?;
            let p = e.path();
            if e.file_type().ok()?.is_dir() {
                subdirs.push(p);
            } else {
                let name = e.file_name().to_string_lossy().into_owned();
                if name == "zig" || name == "zig.exe" {
                    return Some(p);
                }
            }
        }
        subdirs.into_iter().find_map(|d| walk(&d, depth + 1))
    }
    walk(root, 0)
}

/// 目的：把 `zig cc` / `zig c++` 包成常规编译器名（模块构建按 `gcc` / `g++` 这类能力名声明）。
/// 约束：包装用相对路径（`$DIR/../…`），产品根整体搬移后仍可用；unix 上给执行位（其它平台清单不用 Zig）。
fn write_zig_wrappers(install: &Path) -> Result<(), String> {
    let zig = find_zig(install)
        .ok_or_else(|| format!("包里找不到 zig 可执行文件：{}", install.display()))?;
    let rel = zig
        .strip_prefix(install)
        .map_err(|_| format!("zig 不在安装目录里：{}", zig.display()))?;
    let rel = rel.to_string_lossy().replace('\\', "/");
    let bin = install.join("bin");
    fs::create_dir_all(&bin).map_err(|e| format!("建 {} 失败：{}", bin.display(), e))?;
    // Zig 默认把全局缓存写到 ~/.cache/zig（项目外）；钉进安装目录，项目根整体搬移后仍成立。
    let zig_body = format!(
        r#"#!/bin/sh
DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
export ZIG_GLOBAL_CACHE_DIR="${{ZIG_GLOBAL_CACHE_DIR:-$DIR/../cache/global}}"
export ZIG_LOCAL_CACHE_DIR="${{ZIG_LOCAL_CACHE_DIR:-$DIR/../cache/local}}"
exec "$DIR/../{}" "$@"
"#,
        rel
    );
    write_exe(&bin.join("zig"), &zig_body)?;
    for (name, sub) in [
        ("cc", "cc"),
        ("gcc", "cc"),
        ("clang", "cc"),
        ("c++", "c++"),
        ("g++", "c++"),
        ("clang++", "c++"),
    ] {
        // cc-rs 把 clang 形编译器当跨平台 clang，传 Rust 三元组（x86_64-unknown-linux-gnu）；
        // Zig 不认 `unknown-`，不翻译时 ring 等 C 依赖直接构建失败。
        let body = format!(
            r#"#!/bin/sh
DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
export ZIG_GLOBAL_CACHE_DIR="${{ZIG_GLOBAL_CACHE_DIR:-$DIR/../cache/global}}"
export ZIG_LOCAL_CACHE_DIR="${{ZIG_LOCAL_CACHE_DIR:-$DIR/../cache/local}}"
n=$#
i=0
while [ "$i" -lt "$n" ]; do
  a=$1
  shift
  case "$a" in
    --target=*unknown-*) a=$(printf '%s\n' "$a" | sed 's/^--target=\(.*\)-unknown-/--target=\1-/') ;;
    --target=x86_64-apple-darwin) a=--target=x86_64-macos ;;
    --target=aarch64-apple-darwin) a=--target=aarch64-macos ;;
  esac
  set -- "$@" "$a"
  i=$((i + 1))
done
exec "$DIR/zig" {} "$@"
"#,
            sub
        );
        write_exe(&bin.join(name), &body)?;
    }
    Ok(())
}

/// 目的：写一个可执行包装脚本；unix 上补执行位（Windows 只写文件，清单不用 Zig）。
fn write_exe(path: &Path, body: &str) -> Result<(), String> {
    fs::write(path, body).map_err(|e| format!("写 {} 失败：{}", path.display(), e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("给 {} 执行位失败：{}", path.display(), e))?;
    }
    Ok(())
}

/// 目的：单文件依赖——把缓存里的文件按 probe 的文件名放进安装目录（如 guest 的 qcow2）。
fn install_file(cached: &Path, dest: &Path, probe: &str) -> Result<(), String> {
    let name = Path::new(probe)
        .file_name()
        .ok_or_else(|| format!("probe 没有文件名：{}", probe))?;
    let to = dest.join(name);
    fs::copy(cached, &to).map_err(|e| format!("复制到 {} 失败：{}", to.display(), e))?;
    Ok(())
}

/// 目的：AppImage 便携包——跑它自带的 `--appimage-extract`（不依赖 FUSE），把解出的 AppDir 内容搬进安装目录。
/// 约束：`bin/` 里的 applet 按 argv[0] 解析 `../shared/bin/<名>`，所以**保持原布局**、不做包装脚本。
fn install_appimage(
    archive: &Path,
    dest: &Path,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(archive)
            .map_err(|e| format!("看不了 {}：{}", archive.display(), e))?
            .permissions()
            .mode();
        fs::set_permissions(archive, fs::Permissions::from_mode(mode | 0o111))
            .map_err(|e| format!("加执行位失败：{}", e))?;
    }
    // 解到安装目录里（同一文件系统，随后原地搬移，避免跨设备 rename）。
    let _ = fs::remove_dir_all(dest.join("AppDir"));
    let _ = fs::remove_file(dest.join("squashfs-root"));
    on_line("解包 AppImage（--appimage-extract，不依赖 FUSE）");
    let out = std::process::Command::new(archive)
        .arg("--appimage-extract")
        .current_dir(dest)
        .output()
        .map_err(|e| format!("跑不了 AppImage 解包（{}）：{}", archive.display(), e))?;
    if !out.status.success() {
        let msg: String = String::from_utf8_lossy(&out.stderr)
            .chars()
            .take(200)
            .collect();
        return Err(format!("AppImage 解包失败：{}", msg));
    }
    // pkgforge 的运行时解到 AppDir/（另留一个 squashfs-root 软链）；两种布局都认。
    let root = dest.join("AppDir");
    let root = if root.is_dir() {
        root
    } else {
        dest.join("squashfs-root")
    };
    if !root.is_dir() {
        return Err("AppImage 解包后没找到 AppDir / squashfs-root".to_string());
    }
    for entry in fs::read_dir(&root).map_err(|e| format!("读 {} 失败：{}", root.display(), e))?
    {
        let entry = entry.map_err(|e| e.to_string())?;
        let to = dest.join(entry.file_name());
        if to.exists() {
            continue; // 重复安装时幂等：已在场的不覆盖
        }
        fs::rename(entry.path(), &to)
            .map_err(|e| format!("搬 {} 失败：{}", entry.file_name().to_string_lossy(), e))?;
    }
    let _ = fs::remove_dir_all(dest.join("AppDir"));
    let _ = fs::remove_file(dest.join("squashfs-root"));
    Ok(())
}

/// 目的：Windows 宿主的 guest 磁盘——qcow2 单文件落盘，并写一份 guest.yaml 画像（命令通道 QGA）。
fn install_guest_disk(
    cached: &Path,
    dest: &Path,
    probe: &str,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), String> {
    // 磁盘是**可变状态**（guest 里已经装过 agent / 运行包），不能拿缓存里的原始镜像覆盖它；
    // 只有不在场时才落盘。运行期编排（guest-runtime）负责把它编到可用。
    let name = Path::new(probe)
        .file_name()
        .ok_or_else(|| format!("probe 没有文件名：{}", probe))?;
    if dest.join(name).is_file() {
        on_line("guest 镜像已在场，不覆盖（运行期状态在里面）");
    } else {
        install_file(cached, dest, probe)?;
    }
    let yaml = dest.join("guest.yaml");
    if !yaml.is_file() {
        fs::write(
            &yaml,
            "# 由产品生成：命令通道 QGA；共享客户端由运行期编排装好 cifs-utils 后声明（见 guest-runtime）。\n\nid: debian-13-genericcloud-amd64\nos: linux\narch: amd64\nagent: qga\nclients: []\nruntimes: []\n",
        )
        .map_err(|e| format!("写 guest.yaml 失败：{}", e))?;
        on_line("写 guest.yaml（QGA 通道画像）");
    }
    Ok(())
}

/// 目的：建软链（只 Unix 有；Windows 上如实报不支持）。
#[cfg(unix)]
fn symlink(target: &str, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn symlink(_target: &str, _link: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "本平台不支持软链",
    ))
}

/// 目的：跑一个外部工具并判成功；失败给一句人话。
fn run_tool(cmd: &mut std::process::Command, name: &str) -> Result<(), String> {
    let st = cmd
        .status()
        .map_err(|e| format!("跑不了 {}（缺这个系统工具？）：{}", name, e))?;
    if !st.success() {
        return Err(format!("{} 失败", name));
    }
    Ok(())
}

/// 目的：从 GPT 里取 Linux root 分区（type GUID 优先，退第一个）的 (start, size)，单位 512 字节扇区。
fn root_partition(disk: &Path) -> Result<(u64, u64), String> {
    let out = std::process::Command::new("sfdisk")
        .arg("-J")
        .arg(disk)
        .output()
        .map_err(|e| format!("跑不了 sfdisk（需要 util-linux）：{}", e))?;
    if !out.status.success() {
        return Err("sfdisk 读分区表失败".to_string());
    }
    let table: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("sfdisk 输出不是 JSON：{}", e))?;
    let parts = table
        .get("partitiontable")
        .and_then(|t| t.get("partitions"))
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default();
    let root = parts
        .iter()
        .find(|p| {
            p.get("type").and_then(|t| t.as_str()) == Some("4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709")
        })
        .or_else(|| parts.first())
        .ok_or_else(|| "cloud 镜像里没有分区".to_string())?;
    let start = root
        .get("start")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "分区缺 start".to_string())?;
    let size = root
        .get("size")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "分区缺 size".to_string())?;
    Ok((start, size))
}

/// 目的：取一个目录页的纯文本（列 Debian 包名用）。
fn get_text(agent: &ureq::Agent, url: &str) -> Result<String, String> {
    let resp = agent
        .get(url)
        .call()
        .map_err(|e| format!("取 {} 失败：{}", url, e))?;
    if !resp.status().is_success() {
        return Err(format!("取 {} 返回 {}", url, resp.status().as_u16()));
    }
    let mut body = resp.into_body();
    body.read_to_string()
        .map_err(|e| format!("读 {} 响应失败：{}", url, e))
}

/// 目的：把版本号里的数字段零填充，得到一个可字典序比较的键（数字感知排序）。
fn version_key(s: &str) -> String {
    let mut out = String::new();
    let mut num = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            num.push(c);
        } else {
            if !num.is_empty() {
                out.push_str(&format!("{:0>12}", num));
                num.clear();
            }
            out.push(c);
        }
    }
    if !num.is_empty() {
        out.push_str(&format!("{:0>12}", num));
    }
    out
}

/// 目的：从目录页里挑出候选 .deb 名（前缀 + 必须包含两段判据），按版本号取最新。
fn pick_deb(text: &str, prefix: &str, must_contain: &str) -> Option<String> {
    let mut names: Vec<String> = Vec::new();
    for tok in text.split(|c: char| {
        !(c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '_' | '.' | '~' | ':'))
    }) {
        if tok.starts_with(prefix)
            && tok.ends_with(".deb")
            && tok.contains(must_contain)
            && !names.iter().any(|n| n == tok)
        {
            names.push(tok.to_string());
        }
    }
    names.into_iter().max_by_key(|n| version_key(n))
}

/// 目的：把 .deb 的 data.tar.* 解进目标（先 ar 取成员，不跑 maintainer 脚本）。
fn extract_deb(deb: &Path, dest: &Path, cache: &Path) -> Result<(), String> {
    let tmp = cache.join("deb-x");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).map_err(|e| format!("建 {} 失败：{}", tmp.display(), e))?;
    let st = std::process::Command::new("ar")
        .arg("x")
        .arg(deb)
        .current_dir(&tmp)
        .status()
        .map_err(|e| format!("跑不了 ar（需要 binutils）：{}", e))?;
    if !st.success() {
        return Err("ar 解 .deb 失败".to_string());
    }
    let data = fs::read_dir(&tmp)
        .map_err(|e| format!("读 {} 失败：{}", tmp.display(), e))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().starts_with("data.tar"))
                .unwrap_or(false)
        })
        .ok_or_else(|| ".deb 里没有 data.tar.*".to_string())?;
    let r = crate::kernel::detail::unpack::extract(&data, dest);
    let _ = fs::remove_dir_all(&tmp);
    r
}

/// 目的：基础根里**能拿来起 9p 根**的已装内核 ABI——排除 `-rt-`（实时内核，不是我们要的）与 `-cloud-`（没编 9p）。
/// 约束：`boot/vmlinuz-*` 只是候选；还要它的模块树里真有 9pnet_virtio，才算数。
fn usable_guest_abi(base: &Path) -> Option<String> {
    let boot = base.join("boot");
    let mut abis: Vec<String> = fs::read_dir(&boot)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.strip_prefix("vmlinuz-").map(|s| s.to_string())
        })
        .collect();
    abis.sort();
    abis.into_iter()
        .find(|abi| !abi.contains("-rt-") && !abi.contains("-cloud-") && abi_has_9p(base, abi))
}

/// 目的：装入 Debian 通用内核（带 9p 的那些模块），返回内核 ABI。
/// 约束：cloud 镜像自带的 cloud 内核把 `CONFIG_NET_9P` 关了（`# CONFIG_NET_9P is not set`）——它**起不了 9p 根**，
///   所以只有"已装的 ABI 确实带 9pnet_virtio 模块"才算数，否则换与 rootfs 同 Debian 大版本的通用内核（deb 自带模块）。
fn ensure_guest_kernel(
    cache: &Path,
    base: &Path,
    agent: &ureq::Agent,
    on_line: &mut dyn FnMut(&str),
) -> Result<String, String> {
    if let Some(abi) = usable_guest_abi(base) {
        return Ok(abi);
    }
    let url =
        link("guest.debian.linux").ok_or_else(|| "links.yaml 缺 guest.debian.linux".to_string())?;
    let listing = get_text(agent, &url)?;
    let major = debian_major(base);
    let name = pick_kernel_deb(&listing, major.as_deref())
        .ok_or_else(|| "内核源里没找到 amd64 通用内核包".to_string())?;
    let deb = cache.join(&name);
    if !deb.is_file() {
        download(agent, &format!("{}{}", url, name), &deb, on_line)?;
    }
    on_line(&format!("装入通用内核（带 9p 模块）：{}", name));
    extract_deb(&deb, base, cache)?;
    let abi = abi_of_deb(&name).ok_or_else(|| format!("认不出内核包的 ABI：{}", name))?;
    // 换内核要连两个稳定软链一起换（否则 vmlinuz 还指着没有 9p 的 cloud 内核）。
    link_guest_kernel(base, &abi)?;
    Ok(abi)
}

/// 目的：这个 ABI 在基础根里有没有 9p 客户端模块（cloud 内核没有——它是模块，且 cloud 内核压根没编）。
fn abi_has_9p(base: &Path, abi: &str) -> bool {
    let dir = base
        .join("lib")
        .join("modules")
        .join(abi)
        .join("kernel")
        .join("net")
        .join("9p");
    fs::read_dir(dir)
        .map(|it| {
            it.filter_map(|e| e.ok()).any(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("9pnet_virtio.ko")
            })
        })
        .unwrap_or(false)
}

/// 目的：基础根是 Debian 第几版（`/etc/debian_version` 的主版本号）；读不到给 None（那就别挑版本）。
fn debian_major(base: &Path) -> Option<String> {
    let text = fs::read_to_string(base.join("etc").join("debian_version")).ok()?;
    let digits: String = text
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        None
    } else {
        Some(digits)
    }
}

/// 目的：从内核包文件名取 ABI（`linux-image-<abi>-unsigned_<版本>_<arch>.deb`）。
fn abi_of_deb(name: &str) -> Option<String> {
    let rest = name.strip_prefix("linux-image-")?;
    let rest = rest.split('_').next()?;
    rest.strip_suffix("-unsigned").map(|s| s.to_string())
}
/// 目的：从内核源清单里挑一个**普通**通用内核包名——`-rt-`（实时）与 `-cloud-`（没编 9p）都不能要。
/// 参数：major = rootfs 的 Debian 大版本号（有就优先同代内核，避免拿 deb14 的内核配 deb13 的 rootfs）。
fn pick_kernel_deb(listing: &str, major: Option<&str>) -> Option<String> {
    let pick =
        |must: &str| pick_deb(listing, "linux-image-", must).filter(|n| n.ends_with("_amd64.deb"));
    let exact = major.map(|m| format!("+deb{}-amd64-unsigned_", m));
    exact.as_deref().and_then(pick).or_else(|| {
        pick("-amd64-unsigned_").filter(|n| !n.contains("-rt-") && !n.contains("-cloud-"))
    })
}
/// 目的：装入 qemu-guest-agent 并让它随 virtio-serial 端口出现而启动。
fn ensure_guest_qga(
    cache: &Path,
    base: &Path,
    agent: &ureq::Agent,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), String> {
    if !base.join("usr").join("sbin").join("qemu-ga").is_file() {
        let url = link("guest.debian.pool")
            .ok_or_else(|| "links.yaml 缺 guest.debian.pool".to_string())?;
        let listing = get_text(agent, &url)?;
        let name = pick_deb(&listing, "qemu-guest-agent_", "amd64.deb")
            .ok_or_else(|| "qemu 源里没找到 guest agent 包".to_string())?;
        let deb = cache.join(&name);
        if !deb.is_file() {
            download(agent, &format!("{}{}", url, name), &deb, on_line)?;
        }
        on_line(&format!("装入 qemu-guest-agent：{}", name));
        extract_deb(&deb, base, cache)?;
    }
    // 可执行位：解归档没还原权限位的安装靠它自愈——agent 起不来就是这里的锅。
    ensure_executable(&base.join("usr").join("sbin").join("qemu-ga"));
    let wants = base
        .join("etc")
        .join("systemd")
        .join("system")
        .join("multi-user.target.wants");
    fs::create_dir_all(&wants).map_err(|e| format!("建 {} 失败：{}", wants.display(), e))?;
    let unit = wants.join("qemu-guest-agent.service");
    // 按**链接本身**在不在判断：它的目标是绝对路径，`exists()` 会在宿主上找，永远找不到。
    if unit.symlink_metadata().is_err() {
        symlink("/usr/lib/systemd/system/qemu-guest-agent.service", &unit)
            .map_err(|e| format!("建 qemu-guest-agent.service 软链失败：{}", e))?;
    }
    Ok(())
}

/// 目的：给已落盘的可执行文件补齐可执行位（解归档丢过权限位的安装靠它自愈）；不是文件就什么都不做。
fn ensure_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let Ok(meta) = fs::metadata(path) else {
            return;
        };
        if !meta.is_file() {
            return;
        }
        let mode = meta.permissions().mode() & 0o7777;
        if mode & 0o111 != 0 {
            return;
        }
        // 照 Unix 惯例补齐：rwxr-xr-x（可执行位全给，读位保留原有）。
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode | 0o555));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}
/// 目的：清掉 cloud 镜像留下的、在 9p 根下会掉进 emergency 的东西。
fn sanitize_guest_base(base: &Path) -> Result<(), String> {
    fs::write(
        base.join("etc").join("fstab"),
        "# guest root 由内核命令行给出（磁盘镜像 /dev/vda），这里不写 fstab 条目。
",
    )
    .map_err(|e| format!("写 fstab 失败：{}", e))?;
    let mask = base
        .join("etc")
        .join("systemd")
        .join("system")
        .join("systemd-remount-fs.service");
    if mask.symlink_metadata().is_err() {
        fs::create_dir_all(mask.parent().expect("有父目录")).map_err(|e| e.to_string())?;
        symlink("/dev/null", &mask).map_err(|e| format!("mask remount 失败：{}", e))?;
    }
    Ok(())
}

/// 目的：一个 Linux 虚拟根发行版族的构造配方——initrd 的差异只在这里，执行器（unshare + chroot + 固定 PATH）共用。
/// 约束：内核 / QGA / 系统清理 / 运行期编排仍按 Debian 写死；第二族落地时按同一份清单字段扩展。
#[derive(Debug)]
struct GuestFamily {
    /// 目的：initrd 配置（虚拟根内相对路径）。
    initrd_conf: &'static str,
    /// 目的：把模块清单写成该族配置文件的全文。
    initrd_conf_text: fn(&[String]) -> String,
    /// 目的：在 chroot 里跑的构建脚本（已含固定 PATH）。
    initrd_script: fn(&str) -> String,
    /// 目的：initrd 产物（虚拟根内相对路径）。
    initrd_artifact: fn(&str) -> String,
}

/// 目的：initrd 要补的模块——9p 回退档要它们；模块名各族一致，只有写法由配方给。
const INITRD_MODULES: &[&str] = &[
    "9p",
    "9pnet",
    "9pnet_virtio",
    "virtio",
    "virtio_pci",
    "virtio_ring",
];

/// 目的：按清单的 family 取构造配方；当前只有 Debian。
fn guest_family(name: &str) -> Option<&'static GuestFamily> {
    match name {
        "debian" => Some(&DEBIAN),
        _ => None,
    }
}

/// 目的：这条 guest-rootfs 用哪份配方——缺声明与不认识都如实拒绝，并指出预构造镜像的出路。
fn family_of(dep: &Dep) -> Result<&'static GuestFamily, String> {
    match dep.family.as_deref() {
        Some(name) => guest_family(name).ok_or_else(|| {
            format!(
                "{}：没有发行版族 {} 的构造配方（当前只有 debian）；可用预构造镜像 + guest.yaml",
                dep.id, name
            )
        }),
        None => Err(format!(
            "{}：清单缺 family（这条 guest-rootfs 用哪份构造配方）；可用预构造镜像 + guest.yaml",
            dep.id
        )),
    }
}

/// 目的：Debian / Ubuntu 的 initrd 配方（initramfs-tools）——一行一个模块。
static DEBIAN: GuestFamily = GuestFamily {
    initrd_conf: "etc/initramfs-tools/modules",
    initrd_conf_text: initramfs_tools_conf,
    initrd_script: initramfs_tools_script,
    initrd_artifact: initramfs_tools_artifact,
};

/// 目的：initramfs-tools 的模块文件全文。
fn initramfs_tools_conf(modules: &[String]) -> String {
    let mut text = modules.join("\n");
    text.push('\n');
    text
}

/// 目的：initramfs-tools 的构建脚本——先固定 guest PATH，宿主 PATH 缺 /usr/sbin 也能找到 depmod / mkinitramfs。
fn initramfs_tools_script(abi: &str) -> String {
    format!(
        "#!/bin/bash\nset -e\nexport PATH=/usr/sbin:/usr/bin:/sbin:/bin\ndepmod {0}\nmkinitramfs -o /boot/initrd.img-{0} {0}\n",
        abi
    )
}

/// 目的：initramfs-tools 的产物（虚拟根内相对路径）。
fn initramfs_tools_artifact(abi: &str) -> String {
    format!("boot/initrd.img-{}", abi)
}

/// 目的：把已有的 initrd 模块清单与要补的模块合并——去空行、保留原顺序、不重复。
fn merge_initrd_modules(existing: &str, wanted: &[&str]) -> Vec<String> {
    let mut lines: Vec<String> = existing
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    for m in wanted {
        if !lines.iter().any(|l| l == m) {
            lines.push((*m).to_string());
        }
    }
    lines
}

/// 目的：拼出 initrd 构建失败的事实——退出码 + 输出尾部；不替内核能力下结论。
fn initrd_failure(code: Option<i32>, stderr: &[u8], stdout: &[u8]) -> String {
    let tail = output_tail(stderr, stdout);
    let status = match code {
        Some(code) => format!("退出码 {}", code),
        None => "被信号终止".to_string(),
    };
    if tail.is_empty() {
        format!("生成 initrd 失败（{}，无输出）", status)
    } else {
        format!("生成 initrd 失败（{}）：{}", status, tail)
    }
}

/// 目的：取子进程输出的尾部（stderr 在前、stdout 在后，最多 8 行，每行最多 300 字）。
fn output_tail(stderr: &[u8], stdout: &[u8]) -> String {
    let mut lines: Vec<String> = Vec::new();
    for bytes in [stderr, stdout] {
        for line in String::from_utf8_lossy(bytes).lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            lines.push(line.chars().take(300).collect());
        }
    }
    let start = lines.len().saturating_sub(8);
    lines[start..].join("\n")
}

/// 目的：在 guest 里生成带 9p 模块的 initrd（chroot 走非特权用户命名空间，不需要 root）。
/// 参数：family = 这条虚拟根的发行版族配方（配置路径 / 模块写法 / 构建脚本 / 产物）。
/// 约束：脚本在 chroot 里固定 PATH；失败带退出码与输出尾部，不把非零退出归因到命名空间。
fn ensure_guest_initrd(
    base: &Path,
    abi: &str,
    family: &GuestFamily,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let conf = base.join(family.initrd_conf);
    fs::create_dir_all(conf.parent().expect("有父目录")).map_err(|e| e.to_string())?;
    let modules = merge_initrd_modules(
        &fs::read_to_string(&conf).unwrap_or_default(),
        INITRD_MODULES,
    );
    fs::write(&conf, (family.initrd_conf_text)(&modules))
        .map_err(|e| format!("写 {} 失败：{}", conf.display(), e))?;
    let script = base.join("tmp").join("solomni-initrd.sh");
    fs::create_dir_all(script.parent().expect("有父目录")).map_err(|e| e.to_string())?;
    fs::write(&script, (family.initrd_script)(abi))
        .map_err(|e| format!("写 initrd 脚本失败：{}", e))?;
    on_line("在 chroot（用户命名空间，无需 root）里生成带 9p 的 initrd");
    // /proc 绑定失败不致命（Debian 的 mkinitramfs 不需要它），但要如实留一行，不静默吞掉。
    let inner = format!(
        "if mount --bind /proc {proc} 2>/dev/null; then :; else echo 'solomni: /proc 绑定失败（继续）' >&2; fi; chroot {root} /bin/bash /tmp/solomni-initrd.sh",
        proc = base.join("proc").display(),
        root = base.display()
    );
    let out = std::process::Command::new("unshare")
        .args(["-r", "-m", "sh", "-c", &inner])
        .output()
        .map_err(|e| format!("起不了 unshare（需要 util-linux）：{}", e))?;
    let _ = fs::remove_file(&script);
    if !out.status.success() {
        return Err(initrd_failure(out.status.code(), &out.stderr, &out.stdout));
    }
    // 成功时把 chroot 的告警带出来（限 5 行，别刷屏）：/proc 绑定失败这类事实不能只在日志里。
    for line in String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(5)
    {
        on_line(&format!("initrd：{}", line.trim()));
    }
    let artifact = base.join((family.initrd_artifact)(abi));
    if !artifact.is_file() {
        return Err(format!(
            "initrd 没生成出来：{}（chroot 脚本退出 0，但没有产物）",
            artifact.display()
        ));
    }
    Ok(())
}

/// 目的：vmlinuz / initrd.img 这两个稳定软链指向当前 ABI（指错了就换指——换内核时会用到）。
fn link_guest_kernel(base: &Path, abi: &str) -> Result<(), String> {
    for (link_name, target) in [
        ("vmlinuz", format!("boot/vmlinuz-{}", abi)),
        ("initrd.img", format!("boot/initrd.img-{}", abi)),
    ] {
        let p = base.join(link_name);
        if let Ok(cur) = fs::read_link(&p) {
            if cur == Path::new(&target) {
                continue;
            }
        }
        // 名字被占着就先清掉（软链 / 普通文件都算）；再无条件建——不看 `exists()`（它跟着目标走，会骗人）。
        if p.symlink_metadata().is_ok() {
            fs::remove_file(&p).map_err(|e| format!("换 {} 软链失败：{}", link_name, e))?;
        }
        symlink(&target, &p).map_err(|e| format!("建 {} 软链失败：{}", link_name, e))?;
    }
    Ok(())
}

/// 目的：Linux guest 基础根的多步构造——cloud 镜像 → rootfs 目录 + 通用内核 + 9p initrd + qemu-guest-agent。
/// 约束：只写项目内；调系统工具（sfdisk / dd / debugfs / ar / unshare），不依赖任何 Node 脚本。
/// 联动：源在 links.yaml 的 guest.*；镜像 tar 已由调用方下到缓存。
fn install_guest_rootfs(
    root: &Path,
    dep: &Dep,
    image_tar: &Path,
    agent: &ureq::Agent,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let family = family_of(dep)?;
    let base = under(root, &dep.install);
    let cache = root.join(".vm").join("cache");
    fs::create_dir_all(&base).map_err(|e| format!("建 {} 失败：{}", base.display(), e))?;
    let disk = cache.join("disk.raw");
    if !disk.is_file() {
        on_line("解开 cloud 镜像 tar（disk.raw）");
        let x = cache.join("image-x");
        let _ = fs::remove_dir_all(&x);
        fs::create_dir_all(&x).map_err(|e| format!("建 {} 失败：{}", x.display(), e))?;
        // 2 GB 的 disk.raw 在 tar 里是 **GNU sparse**：用系统 tar 展开（纯 Rust 解包暂不支持稀疏项）。
        run_tool(
            std::process::Command::new("tar")
                .arg("-xf")
                .arg(image_tar)
                .arg("-C")
                .arg(&x),
            "tar",
        )?;
        let found = fs::read_dir(&x)
            .map_err(|e| format!("读 {} 失败：{}", x.display(), e))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| p.file_name().map(|n| n == "disk.raw").unwrap_or(false))
            .ok_or_else(|| "cloud 镜像 tar 里没有 disk.raw".to_string())?;
        fs::rename(&found, &disk).map_err(|e| format!("搬 disk.raw 失败：{}", e))?;
        let _ = fs::remove_dir_all(&x);
    } else {
        on_line("用已有 disk.raw（缓存）");
    }
    let rootfs = cache.join("rootfs.ext4");
    if !rootfs.is_file() {
        on_line("从 GPT 取 root 分区（sfdisk + dd）");
        let (start, size) = root_partition(&disk)?;
        run_tool(
            std::process::Command::new("dd")
                .arg(format!("if={}", disk.display()))
                .arg(format!("of={}", rootfs.display()))
                .arg("bs=512")
                .arg(format!("skip={}", start))
                .arg(format!("count={}", size))
                .arg("status=none"),
            "dd",
        )?;
    }
    // 重新构造前先清掉上一次的镜像产物：留着它，rdump 出来的目录里就多一份旧 rootfs.ext4，
    // 打新镜像时会被整个包进去（体积翻倍、内容还是过期的）。
    let stale_image = base.join("rootfs.ext4");
    if stale_image.is_file() {
        fs::remove_file(&stale_image).map_err(|e| format!("清旧镜像失败：{}", e))?;
    }
    if !base.join("etc").join("os-release").is_file() {
        on_line(&format!("把 rootfs 解到 {}（debugfs rdump）", dep.install));
        extract_guest_rootfs(&rootfs, &base)?;
    }
    // 这时目录树与源镜像一一对应：先把"打包会丢的属性"读出来（之后会装内核 / agent，树不再对应）。
    let attrs = guest_attrs_to_restore(&rootfs, &base)?;
    let abi = ensure_guest_kernel(&cache, &base, agent, on_line)?;
    ensure_guest_qga(&cache, &base, agent, on_line)?;
    sanitize_guest_base(&base)?;
    ensure_guest_initrd(&base, &abi, family, on_line)?;
    link_guest_kernel(&base, &abi)?;
    #[cfg(unix)]
    {
        let py3 = base.join("usr").join("bin").join("python3");
        let py = base.join("usr").join("bin").join("python");
        if py3.is_file() && !py.exists() {
            let _ = std::os::unix::fs::symlink("python3", &py);
        }
    }
    // 画像：虚拟根对世界的自我介绍（列表显示名与 os 从这里读）；运行能力由运行期编排另写。
    let yaml = base.join("guest.yaml");
    if !yaml.is_file() {
        fs::write(
            &yaml,
            "# 由产品生成：命令通道 QGA；共享客户端与运行能力在运行期编排（guest-runtime）里确认后另写。\n\nid: debian-13-genericcloud-amd64\nos: linux\narch: amd64\nagent: qga\nclients: [9p, virtiofs]\nruntimes: []\n",
        )
        .map_err(|e| format!("写 guest.yaml 失败：{}", e))?;
        on_line("写 guest.yaml（QGA 通道画像）");
    }
    // 运行产物是**磁盘镜像**（会话靠每租约可写层并发，谁也不写基础镜像）；目录只在这条构造链里当中间产物。
    pack_guest_base(&base, &cache, on_line)?;
    // rdump 与 mke2fs 都拿不到源镜像里的特权位 / 属主、属组：按刚读出的清单在**镜像上**回填并核对。
    if !attrs.is_empty() {
        on_line(&format!(
            "按源镜像回填 {} 条属性（特权位 / 属主）",
            attrs.len()
        ));
    }
    restore_guest_image_attrs(&base.join("rootfs.ext4"), &attrs)?;
    // agent 已经装进镜像：运行期编排看到这个标记就跳过 cloud-init 首启。
    fs::write(
        base.join(".qga-ready"),
        "guest-rootfs 已装 qemu-guest-agent（打包进磁盘镜像）\n",
    )
    .map_err(|e| format!("写 .qga-ready 失败：{}", e))?;
    Ok(())
}

/// 目的：把构造好的 rootfs 目录打成**磁盘镜像**（`rootfs.ext4`，raw），然后删掉目录中间产物。
/// 参数：base = 虚拟根目录（内核 / initrd / guest.yaml 与镜像并排）；cache = 打包临时落点；on_line = 进度行。
/// 约束：只在 Linux 宿主上跑——`unshare -r` 拿假 root，镜像里的属主才会是 0（否则会写成调用者 uid）；
///   调用前目录已定稿（内核 / agent / initrd / 画像都写完）。
fn pack_guest_base(base: &Path, cache: &Path, on_line: &mut dyn FnMut(&str)) -> Result<(), String> {
    // 内核 / initrd 现在是软链（`vmlinuz` → `boot/vmlinuz-<abi>`）：落成实体文件，否则删目录后就成了悬空链。
    for name in ["vmlinuz", "initrd.img"] {
        let link = base.join(name);
        if fs::symlink_metadata(&link).is_err()
            || !fs::symlink_metadata(&link)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
        {
            continue;
        }
        let tmp = base.join(format!("{}.solid", name));
        fs::copy(&link, &tmp).map_err(|e| format!("把 {name} 落成实体文件失败：{e}"))?;
        fs::remove_file(&link).map_err(|e| format!("换掉 {name} 软链失败：{e}"))?;
        fs::rename(&tmp, &link).map_err(|e| format!("放回 {name} 失败：{e}"))?;
    }
    let used = dir_bytes(base)?;
    // 留足余量：guest 里 apt 要下索引（压缩的也有几十 MB）、装 C/C++（约 300 MB）、解 Node（约 200 MB）、写日志——
    // 1.6 GiB 的旧镜像在维护启动装基础工具链时会真的空间不足（Write error 28）。下限 1 GB、至少 1.5 GB 余量。
    let slack = std::cmp::max(1536 << 20, used / 2);
    let need = std::cmp::max(1u64 << 30, used + slack);
    let blocks = need.div_ceil(4096);
    let tmp_img = cache.join("pack.ext4");
    let _ = fs::remove_file(&tmp_img);
    on_line(&format!(
        "把 rootfs 打成磁盘镜像（{} MiB，目录占用 {} MiB）",
        need >> 20,
        used >> 20
    ));
    // `-d` 从目录灌内容；假 root 里跑，镜像属主才是 root。**必须显式 `-b 4096`**：位置参数按块大小解释，
    // 不给时 mke2fs 会按体积选 1024 字节块（并套用 small 的 4096 字节/inode），镜像缩成约四分之一而装不下。
    run_tool(
        std::process::Command::new("unshare")
            .arg("-r")
            .arg("mke2fs")
            .args(["-q", "-b", "4096", "-t", "ext4", "-F", "-d"])
            .arg(base)
            .arg(&tmp_img)
            .arg(blocks.to_string()),
        "unshare + mke2fs（要 e2fsprogs 且内核允许非特权用户命名空间）",
    )?;
    let image = base.join("rootfs.ext4");
    fs::rename(&tmp_img, &image).map_err(|e| format!("放 rootfs.ext4 失败：{e}"))?;
    // 中间产物删掉：虚拟根只剩 磁盘镜像 + 内核 + initrd + 画像（没有第二份真相）。
    purge_except(
        base,
        &["rootfs.ext4", "vmlinuz", "initrd.img", "guest.yaml"],
    )?;
    on_line("磁盘镜像就位，目录中间产物已清掉");
    Ok(())
}

/// 目的：一条"打包会丢的属性"——镜像内绝对路径 + 完整 mode（含类型位）+ uid / gid。
#[derive(Debug, Clone, PartialEq, Eq)]
struct GuestAttr {
    path: String,
    mode: u32,
    uid: u32,
    gid: u32,
}

/// 目的：把源 rootfs 分区解成目录树（debugfs rdump）。
/// 约束：普通用户身份解——内核会剥掉特权位、rdump 也改不动属主；这两样由
///   `guest_attrs_to_restore` / `restore_guest_image_attrs` 在打包后按源镜像回填。
fn extract_guest_rootfs(image: &Path, dest: &Path) -> Result<(), String> {
    run_tool(
        std::process::Command::new("debugfs")
            .arg("-R")
            .arg(format!("rdump / {}", dest.display()))
            .arg(image),
        "debugfs",
    )
}

/// 目的：源镜像里"打包会丢"的属性——特权位（setuid / setgid / sticky）、非 root 属主 / 属组。
/// 参数：tree 是刚解出的目录树（这时它与源镜像一一对应；要在装内核 / agent 之前调）。
/// 约束：清单来自目录树，属性值读源镜像；本镜像实测约 170 条（21 个特权位 + 140 个非 0 uid + 156 个非 0 gid）。
fn guest_attrs_to_restore(image: &Path, tree: &Path) -> Result<Vec<GuestAttr>, String> {
    let dirs = image_dirs(tree)?;
    let all = image_attrs(image, &dirs)?;
    Ok(all
        .into_iter()
        .filter(|a| a.mode & 0o7000 != 0 || a.uid != 0 || a.gid != 0)
        .collect())
}

/// 目的：把源镜像里那些"打包会丢"的属性回填到最终镜像，再读一遍核对（不一致就硬失败）。
/// 约束：只改需要改的字段（mode 只在有特权位时写、uid / gid 只在非 0 时写）；`sif` 单条失败时
///   debugfs 的退出码未必非零，所以必须自己核对——不把不合规的镜像交给用户。
fn restore_guest_image_attrs(image: &Path, attrs: &[GuestAttr]) -> Result<(), String> {
    if attrs.is_empty() {
        return Ok(());
    }
    let mut cmds = String::new();
    for a in attrs {
        let quoted = debugfs_quote(&a.path)?;
        if a.mode & 0o7000 != 0 {
            cmds.push_str(&format!("sif {} mode 0{:o}\n", quoted, a.mode));
        }
        if a.uid != 0 {
            cmds.push_str(&format!("sif {} uid {}\n", quoted, a.uid));
        }
        if a.gid != 0 {
            cmds.push_str(&format!("sif {} gid {}\n", quoted, a.gid));
        }
    }
    run_debugfs(image, &cmds, true)?;
    let mut parents: Vec<String> = attrs.iter().map(|a| parent_of(&a.path)).collect();
    parents.sort();
    parents.dedup();
    let now = image_attrs(image, &parents)?;
    for want in attrs {
        let Some(got) = now.iter().find(|n| n.path == want.path) else {
            return Err(format!(
                "回填后镜像里找不到 {}（打包链路漏了它）",
                want.path
            ));
        };
        if got.mode != want.mode || got.uid != want.uid || got.gid != want.gid {
            return Err(format!(
                "回填没生效：{} 源是 mode {:o} {}/{}，镜像是 mode {:o} {}/{}",
                want.path, want.mode, want.uid, want.gid, got.mode, got.uid, got.gid
            ));
        }
    }
    Ok(())
}

/// 目的：读一个 ext4 镜像里若干目录的直接条目（`ls -l -p`，一个 debugfs 会话读完）。
/// 参数：dirs = 镜像内绝对路径（`/` 分隔）；实测 2377 个目录一个会话 0.2 秒。
/// 约束：只列给定目录、不递归；路径写进命令表前用双引号包起来（含引号 / 反斜杠 / 换行的直接拒绝）。
fn image_attrs(image: &Path, dirs: &[String]) -> Result<Vec<GuestAttr>, String> {
    if dirs.is_empty() {
        return Ok(Vec::new());
    }
    let mut cmds = String::new();
    for d in dirs {
        cmds.push_str("ls -l -p ");
        cmds.push_str(&debugfs_quote(d)?);
        cmds.push('\n');
    }
    let out = run_debugfs(image, &cmds, false)?;
    let mut attrs = Vec::new();
    let mut dir = String::new();
    for line in out.lines() {
        if let Some(echo) = line.strip_prefix("debugfs: ls -l -p ") {
            dir = unquote(echo.trim())
                .ok_or_else(|| format!("看不懂 debugfs 的命令回显（不猜路径）：{}", line))?;
            continue;
        }
        if dir.is_empty() || !line.starts_with('/') {
            continue;
        }
        // -p 格式：/inode/mode/uid/gid/name/size/ ——名字里的空格原样保留，/ 不可能出现在名字里。
        let parts: Vec<&str> = line.splitn(7, '/').collect();
        if parts.len() < 7 {
            continue;
        }
        let mode = u32::from_str_radix(parts[2], 8)
            .map_err(|_| format!("看不懂 debugfs 的 mode：{}", line))?;
        let uid = parts[3]
            .parse::<u32>()
            .map_err(|_| format!("看不懂 debugfs 的 uid：{}", line))?;
        let gid = parts[4]
            .parse::<u32>()
            .map_err(|_| format!("看不懂 debugfs 的 gid：{}", line))?;
        let name = parts[5];
        if name == "." || name == ".." {
            continue;
        }
        attrs.push(GuestAttr {
            path: format!("{}/{}", dir.trim_end_matches('/'), name),
            mode,
            uid,
            gid,
        });
    }
    Ok(attrs)
}

/// 目的：目录树里的全部目录（镜像内绝对路径，`/` 分隔）；不跟软链（避免环）。
fn image_dirs(tree: &Path) -> Result<Vec<String>, String> {
    let mut out = vec!["/".to_string()];
    let mut stack = vec![(tree.to_path_buf(), String::new())];
    while let Some((abs, rel)) = stack.pop() {
        let entries =
            fs::read_dir(&abs).map_err(|e| format!("读 {} 失败：{}", abs.display(), e))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("读 {} 失败：{}", abs.display(), e))?;
            let kind = entry
                .file_type()
                .map_err(|e| format!("读 {} 的类型失败：{}", entry.path().display(), e))?;
            if !kind.is_dir() {
                continue;
            }
            let name = entry.file_name().into_string().map_err(|_| {
                format!(
                    "目录名不是 UTF-8（写不进 debugfs 命令表）：{}",
                    entry.path().display()
                )
            })?;
            let child = if rel.is_empty() {
                name
            } else {
                format!("{}/{}", rel, name)
            };
            out.push(format!("/{}", child));
            stack.push((entry.path(), child));
        }
    }
    Ok(out)
}

/// 目的：跑一个 debugfs 会话（`-f` 命令表），返回 stdout；`write` 决定要不要 `-w`。
fn run_debugfs(image: &Path, cmds: &str, write: bool) -> Result<String, String> {
    let cmd_file = image.with_extension(format!("debugfs-{}.cmd", std::process::id()));
    fs::write(&cmd_file, cmds).map_err(|e| format!("写 debugfs 命令表失败：{}", e))?;
    let mut cmd = std::process::Command::new("debugfs");
    if write {
        cmd.arg("-w");
    }
    let out = cmd.arg("-f").arg(&cmd_file).arg(image).output();
    let _ = fs::remove_file(&cmd_file);
    let out = out.map_err(|e| format!("跑不了 debugfs（要 e2fsprogs）：{}", e))?;
    if !out.status.success() {
        return Err(format!(
            "debugfs 失败：{}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// 目的：把镜像内路径变成 debugfs 命令表里一个双引号参数。
/// 约束：debugfs 命令表按行读、参数用双引号包；会破坏命令的字符直接拒绝（不猜、不静默改名）。
fn debugfs_quote(path: &str) -> Result<String, String> {
    if path.chars().any(|c| matches!(c, '"' | '\\' | '\n' | '\r')) {
        return Err(format!("路径里有 debugfs 命令表放不下的字符：{}", path));
    }
    Ok(format!("\"{}\"", path))
}

/// 目的：去掉 debugfs 回显里的双引号（我们生成的命令不带转义）。
fn unquote(s: &str) -> Option<String> {
    let inner = s.strip_prefix('"')?.strip_suffix('"')?;
    Some(inner.to_string())
}

/// 目的：镜像内路径的父目录（根下的条目归 `/`）。
fn parent_of(path: &str) -> String {
    match path.rsplit_once('/') {
        Some(("", _)) | None => "/".to_string(),
        Some((p, _)) => p.to_string(),
    }
}

/// 目的：目录里所有普通文件的大小之和（不含软链本身）；算镜像要多大。
fn dir_bytes(dir: &Path) -> Result<u64, String> {
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = fs::read_dir(&d).map_err(|e| format!("读 {} 失败：{e}", d.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            let meta = fs::symlink_metadata(&path)
                .map_err(|e| format!("看 {} 失败：{e}", path.display()))?;
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                total += file_footprint(&meta);
            }
        }
    }
    Ok(total)
}

/// 目的：一个普通文件实际要占的字节数——Unix 上按块算（st_blocks * 512，含块对齐与稀疏），
///   比 len() 更接近 ext4 真正要分配的空间。
/// 约束：非 Unix 拿不到块数，退回文件长度；打包本身只在 Linux 上真跑。
fn file_footprint(meta: &fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.blocks() * 512
    }
    #[cfg(not(unix))]
    {
        meta.len()
    }
}

/// 目的：只留下 `keep` 里那几个名字，其余全删（虚拟根目录的收口）。
fn purge_except(base: &Path, keep: &[&str]) -> Result<(), String> {
    let entries = fs::read_dir(base).map_err(|e| format!("读 {} 失败：{e}", base.display()))?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if keep.contains(&name.as_str()) {
            continue;
        }
        let path = entry.path();
        let meta =
            fs::symlink_metadata(&path).map_err(|e| format!("看 {} 失败：{e}", path.display()))?;
        if meta.is_dir() {
            fs::remove_dir_all(&path).map_err(|e| format!("删 {} 失败：{e}", path.display()))?;
        } else {
            fs::remove_file(&path).map_err(|e| format!("删 {} 失败：{e}", path.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 目的：造一条任意 kind 的清单条目（组别名/缺项判定的用例夹具）。
    fn dep_of(id: &str, kind: &str, install: &str, probe: &str) -> Dep {
        Dep {
            id: id.to_string(),
            os: "any".to_string(),
            arch: "any".to_string(),
            install: install.to_string(),
            probe: probe.to_string(),
            source: "release:x".to_string(),
            kind: kind.to_string(),
            guest: None,
            toolchain: None,
            family: None,
            strip: 0,
            size_mb: 0,
            sha256: String::new(),
            note: String::new(),
        }
    }

    /// 目的：`--ensure` 取值的两种解释——全为 kind 时按组别名，否则按 id 列表；混写如实拒绝。
    #[test]
    fn ensure_spec_splits_kinds_from_ids_and_refuses_mixed() {
        let all = [
            dep_of("toolchain.python.x", "toolchain", ".t/py", ".t/py/bin"),
            dep_of("vm.qemu.x", "archive", ".t/q", ".t/q/bin"),
        ];
        assert_eq!(
            split_ensure_spec(&all, "toolchain").expect("组别名"),
            EnsureSpec::Kinds(vec!["toolchain".to_string()])
        );
        assert_eq!(
            split_ensure_spec(&all, "vm.qemu.x,vm.guest.x").expect("id 列表"),
            EnsureSpec::Ids(vec!["vm.qemu.x".to_string(), "vm.guest.x".to_string()])
        );
        let err = split_ensure_spec(&all, "toolchain,vm.qemu.x").expect_err("混写要拒绝");
        assert!(err.contains("不能混写"), "{}", err);
    }

    /// 目的：组别名只挑「不在场」的条目——在位的不重装（260 MB 的包不重解）。
    #[test]
    fn pending_of_a_kind_lists_only_missing_entries() {
        let root = crate::tests::scratch("provision-kind-pending");
        let present = dep_of("toolchain.python.x", "toolchain", ".t/py", ".t/py/bin");
        let missing = dep_of("toolchain.node.x", "toolchain", ".t/node", ".t/node/bin");
        let other = dep_of("vm.qemu.x", "archive", ".t/q", ".t/q/bin");
        fs::create_dir_all(under(&root, ".t/py/bin")).expect("铺在位工具");
        let all = [present, missing, other];
        assert_eq!(
            pending_of_kinds(&all, &root, &["toolchain".to_string()]),
            vec!["toolchain.node.x".to_string()]
        );
        assert_eq!(
            pending_of_kinds(&all, &root, &["archive".to_string()]),
            vec!["vm.qemu.x".to_string()]
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：`--ensure=toolchain` 在本机工具链都在场时是一条**不联网**的幂等命令（启动提示就靠它）。
    /// 约束：按当前宿主清单铺出 probe（缺当前平台条目就如实跳过）；不下载、不写安装目录。
    #[test]
    fn ensure_spec_group_is_a_noop_when_every_toolchain_is_present() {
        let root = crate::tests::scratch("provision-ensure-group");
        let all = for_host(
            &deps().expect("清单"),
            std::env::consts::OS,
            std::env::consts::ARCH,
        );
        let toolchains: Vec<&Dep> = all.iter().filter(|d| d.kind == "toolchain").collect();
        if toolchains.is_empty() {
            eprintln!("[探针] 当前平台没有工具链条目，跳过");
            return;
        }
        for dep in &toolchains {
            fs::create_dir_all(under(&root, &dep.probe)).expect("铺 probe");
        }
        let svc = ProvisionService::new(&root);
        let mut lines: Vec<String> = Vec::new();
        svc.ensure_spec("toolchain", &mut |l| lines.push(l.to_string()))
            .expect("都在场时不该碰网络");
        assert!(lines.iter().any(|l| l.contains("没有缺项")), "{:?}", lines);
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：sha256 计算与已知向量一致（完整性闸的第一环：算法本身不能错）。
    #[test]
    fn sha256_matches_a_known_vector() {
        let dir = crate::tests::scratch("provision-sha256");
        let f = dir.join("abc.bin");
        fs::write(&f, b"abc").expect("写样本");
        assert_eq!(
            sha256_file(&f).expect("算 sha256"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// 目的：缓存文件的校验按台账来——pin 对得上才放行；不符时**报出台账值、实际值与来源**（不删文件、不循环）；
    ///   没 pin 就如实不校验（`latest` 源 pin 上去必然腐烂）。
    #[test]
    fn cached_file_is_checked_against_the_pinned_hash() {
        let dir = crate::tests::scratch("provision-verify");
        let f = dir.join("asset.bin");
        fs::write(&f, b"abc").expect("写样本");
        let real = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let url = "https://example.test/asset.bin";
        // 大小写不敏感：台账里手抄的大写也要能对上。
        verify_cached("vm.demo", Some(&real.to_uppercase()), &f, url).expect("pin 对得上要放行");
        // 没 pin：不校验（否则 latest 源没法用）。
        verify_cached("vm.demo", None, &f, url).expect("没 pin 就不校验");
        // 不符：报台账值 / 实际值 / 来源，并且**不许删缓存**（删了就是重下死循环）。
        let bad = verify_cached("vm.demo", Some(&"0".repeat(64)), &f, url).expect_err("不符要拒绝");
        assert!(bad.contains(&"0".repeat(64)), "要报台账值：{}", bad);
        assert!(bad.contains(real), "要报实际值：{}", bad);
        assert!(bad.contains(url), "要报来源：{}", bad);
        assert!(
            f.is_file(),
            "不许静默删缓存（pin 过期时那会变成重下死循环）"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// 目的：pin 的来源二选一——清单条目的 `sha256` 优先，链接类源回落到 `links.yaml` 的 `<source>.sha256`。
    #[test]
    fn pin_comes_from_the_manifest_entry_or_the_ledger_pair() {
        let from_ledger = "D5DB".repeat(16);
        let links = BTreeMap::from([(
            "smbd.windows.sha256".to_string(),
            format!("  {}  ", from_ledger),
        )]);
        let mut dep = Dep {
            id: "x".to_string(),
            os: "windows".to_string(),
            arch: "x86_64".to_string(),
            install: ".vm/x".to_string(),
            probe: ".vm/x/x.exe".to_string(),
            source: "smbd.windows".to_string(),
            kind: "file".to_string(),
            guest: None,
            family: None,
            toolchain: None,
            strip: 0,
            size_mb: 1,
            sha256: String::new(),
            note: String::new(),
        };
        assert_eq!(
            pinned_sha256(&dep, &links).as_deref(),
            Some(from_ledger.to_ascii_lowercase().as_str()),
            "链接类源回落到台账里成对的那条（去掉首尾空白、转小写）"
        );
        let own = "E96F".repeat(16);
        dep.sha256 = own.clone();
        assert_eq!(
            pinned_sha256(&dep, &links).as_deref(),
            Some(own.to_ascii_lowercase().as_str()),
            "清单条目自己的 pin 优先"
        );
        dep.source = "guest.debian.image".to_string();
        dep.sha256 = String::new();
        assert_eq!(pinned_sha256(&dep, &links), None, "都没 pin = 不校验");
    }

    /// 目的：目录页里按版本挑最新 .deb（数字感知，不受字典序影响）。
    #[test]
    fn picks_newest_deb_numerically() {
        let listing = r#"<a href="linux-image-6.9.0-amd64-unsigned_6.9.0-1_amd64.deb">x</a>
<a href="linux-image-6.12.111+deb13-amd64-unsigned_6.12.111-1_amd64.deb">y</a>"#;
        let got = pick_deb(listing, "linux-image-", "-amd64-unsigned_").expect("有候选");
        assert_eq!(
            got,
            "linux-image-6.12.111+deb13-amd64-unsigned_6.12.111-1_amd64.deb"
        );
    }

    /// 目的：Debian 配方带齐 initrd 的配置 / 脚本 / 产物；脚本先固定 guest PATH，
    ///   宿主 PATH 缺 /usr/sbin 时也能找到 depmod / mkinitramfs。
    #[test]
    fn debian_family_carries_the_initrd_recipe() {
        let family = guest_family("debian").expect("debian 要有配方");
        assert_eq!(family.initrd_conf, "etc/initramfs-tools/modules");
        let conf = (family.initrd_conf_text)(&["9p".to_string(), "virtio".to_string()]);
        assert_eq!(conf, "9p\nvirtio\n");
        let script = (family.initrd_script)("6.12.0");
        assert!(
            script.contains("export PATH=/usr/sbin:/usr/bin:/sbin:/bin"),
            "{script}"
        );
        assert!(script.contains("depmod 6.12.0"), "{script}");
        assert!(
            script.contains("mkinitramfs -o /boot/initrd.img-6.12.0 6.12.0"),
            "{script}"
        );
        assert_eq!((family.initrd_artifact)("6.12.0"), "boot/initrd.img-6.12.0");
    }

    /// 目的：模块清单按原顺序合并、去空行、不重复；已有的条目不被改写。
    #[test]
    fn initrd_modules_are_merged_without_duplicates() {
        let got = merge_initrd_modules(" foo \n\n9p\n", &["9p", "virtio"]);
        assert_eq!(got, vec!["foo", "9p", "virtio"]);
    }

    /// 目的：清单没声明或声明了不认识的族都要如实拒绝，并指出预构造镜像的出路；不猜、不回落 Debian。
    #[test]
    fn unknown_family_is_refused_with_a_prebuilt_image_way_out() {
        let mut dep = Dep {
            id: "vm.guest.linux".to_string(),
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            install: ".vm/guest/debian".to_string(),
            probe: ".vm/guest/debian/rootfs.ext4".to_string(),
            source: "guest.debian.image".to_string(),
            kind: "guest-rootfs".to_string(),
            guest: Some("debian".to_string()),
            family: Some("alpine".to_string()),
            toolchain: None,
            strip: 0,
            size_mb: 0,
            sha256: String::new(),
            note: String::new(),
        };
        let err = family_of(&dep).expect_err("没有 alpine 配方要拒绝");
        assert!(err.contains("alpine"), "{}", err);
        assert!(err.contains("预构造镜像"), "{}", err);
        dep.family = None;
        let err = family_of(&dep).expect_err("缺 family 也要拒绝");
        assert!(err.contains("family"), "{}", err);
        assert!(err.contains("预构造镜像"), "{}", err);
    }

    /// 目的：失败信息带退出码与子进程输出尾部，不改写成因；没有输出时也如实说清。
    #[test]
    fn initrd_failure_reports_the_real_output() {
        let err = initrd_failure(
            Some(127),
            b"/bin/bash: line 3: depmod: command not found\n",
            b"",
        );
        assert!(err.contains("退出码 127"), "{}", err);
        assert!(err.contains("depmod: command not found"), "{}", err);
        assert!(
            !err.contains("命名空间"),
            "不许把 PATH 问题推给内核：{}",
            err
        );
        let quiet = initrd_failure(None, b"", b"");
        assert!(quiet.contains("被信号终止"), "{}", quiet);
        assert!(quiet.contains("无输出"), "{}", quiet);
    }

    /// 目的：清单里的 Debian 虚拟根必须写明 family——构造配方只按显式声明取，不靠 id 或目录名猜。
    #[test]
    fn manifest_declares_the_debian_family() {
        let all = deps().expect("清单可解析");
        let dep = all
            .iter()
            .find(|d| d.id == "vm.guest.linux")
            .expect("有 Linux 虚拟根条目");
        assert_eq!(dep.family.as_deref(), Some("debian"));
    }

    /// 目的：单文件安装按 probe 的文件名落进安装目录。
    /// 目的：内核 ABI 必须带 9p 模块才算数——cloud 内核没有（`CONFIG_NET_9P` 没编），不能拿它当 9p 根的内核。
    #[test]
    fn only_a_kernel_with_9p_modules_counts() {
        let root = crate::tests::scratch("provision-kernel-9p");
        let base = root.join("base");
        let make = |abi: &str, with_9p: bool| {
            let dir = base
                .join("lib")
                .join("modules")
                .join(abi)
                .join("kernel")
                .join("net")
                .join("9p");
            fs::create_dir_all(&dir).expect("建模块目录");
            if with_9p {
                fs::write(dir.join("9pnet_virtio.ko.xz"), b"x").expect("写模块");
            }
        };
        make("7.0.0+deb13-cloud-amd64", false);
        make("6.12.111+deb13-amd64", true);
        assert!(
            !abi_has_9p(&base, "7.0.0+deb13-cloud-amd64"),
            "没有 9p 模块要判否"
        );
        assert!(
            abi_has_9p(&base, "6.12.111+deb13-amd64"),
            "带 9p 模块要判是"
        );
        // 已装的 ABI 里挑"能起 9p 根"的那个：`-rt-` 与 `-cloud-` 都不要。
        fs::create_dir_all(base.join("boot")).expect("建 boot");
        make("6.12.111+deb13-rt-amd64", true);
        fs::write(
            base.join("boot").join("vmlinuz-6.12.111+deb13-cloud-amd64"),
            b"k",
        )
        .expect("写内核");
        fs::write(
            base.join("boot").join("vmlinuz-6.12.111+deb13-rt-amd64"),
            b"k",
        )
        .expect("写内核");
        assert_eq!(usable_guest_abi(&base), None, "只有 cloud / rt 时不算可用");
        fs::write(base.join("boot").join("vmlinuz-6.12.111+deb13-amd64"), b"k").expect("写内核");
        assert_eq!(
            usable_guest_abi(&base).as_deref(),
            Some("6.12.111+deb13-amd64")
        );
        // 换内核要把两个稳定软链一起换指（否则 vmlinuz 还指着没 9p 的那个）；软链是 unix 概念。
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                "boot/vmlinuz-6.12.111+deb13-cloud-amd64",
                base.join("vmlinuz"),
            )
            .expect("先指 cloud");
            link_guest_kernel(&base, "6.12.111+deb13-amd64").expect("换指");
            assert_eq!(
                fs::read_link(base.join("vmlinuz")).expect("读链"),
                Path::new("boot/vmlinuz-6.12.111+deb13-amd64")
            );
        }
        // 没有软链的平台：换指如实报错，不假装成功。
        #[cfg(not(unix))]
        assert!(
            link_guest_kernel(&base, "6.12.111+deb13-amd64").is_err(),
            "不支持软链的平台要如实报错"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：内核包名要能取出 ABI，包的选择要排除 `-rt-` / `-cloud-`（它们起不了 / 不是我们要的 9p 根）。
    #[test]
    fn kernel_pick_avoids_rt_and_cloud_flavors() {
        let listing = r#"<a href="linux-image-6.12.111+deb13-amd64-unsigned_6.12.111-1_amd64.deb">a</a>
<a href="linux-image-6.12.111+deb13-rt-amd64-unsigned_6.12.111-1_amd64.deb">b</a>
<a href="linux-image-6.12.111+deb13-cloud-amd64-unsigned_6.12.111-1_amd64.deb">c</a>
<a href="linux-image-7.2.11+deb14-amd64-unsigned_7.2.11-1_amd64.deb">d</a>"#;
        assert_eq!(
            pick_kernel_deb(listing, Some("13")).as_deref(),
            Some("linux-image-6.12.111+deb13-amd64-unsigned_6.12.111-1_amd64.deb"),
            "同代内核里挑普通那个"
        );
        let only_bad = r#"<a href="linux-image-6.12.111+deb13-rt-amd64-unsigned_6.12.111-1_amd64.deb">b</a>
<a href="linux-image-6.12.111+deb13-cloud-amd64-unsigned_6.12.111-1_amd64.deb">c</a>"#;
        assert_eq!(pick_kernel_deb(only_bad, Some("13")), None);
        assert_eq!(
            pick_kernel_deb(only_bad, None),
            None,
            "兜底也要排除 rt / cloud"
        );
        assert_eq!(
            pick_kernel_deb(listing, None).as_deref(),
            Some("linux-image-7.2.11+deb14-amd64-unsigned_7.2.11-1_amd64.deb"),
            "没有版本偏好时挑最新的普通内核"
        );
        assert_eq!(
            abi_of_deb("linux-image-6.12.111+deb13-amd64-unsigned_6.12.111-1_amd64.deb").as_deref(),
            Some("6.12.111+deb13-amd64")
        );
        assert_eq!(abi_of_deb("nope.deb"), None);
        // rootfs 的 Debian 大版本：读 `/etc/debian_version`。
        let root = crate::tests::scratch("provision-debian-major");
        assert_eq!(debian_major(&root), None, "没有 debian_version 就不挑版本");
        fs::create_dir_all(root.join("etc")).expect("建 etc");
        fs::write(root.join("etc").join("debian_version"), "13.1\n").expect("写版本");
        assert_eq!(debian_major(&root).as_deref(), Some("13"));
        let _ = fs::remove_dir_all(&root);
    }
    /// 目的：归档里的入口程序装完必须能跑——归档不带 unix 权限位时（0644）要补上可执行位。
    #[cfg(unix)]
    #[test]
    fn archive_probe_gets_an_executable_bit() {
        use std::os::unix::fs::PermissionsExt;
        let root = crate::tests::scratch("provision-archive-exec");
        let cache = root.join(".vm").join("cache");
        fs::create_dir_all(&cache).expect("建缓存");
        let src = root.join("src");
        fs::create_dir_all(&src).expect("建源目录");
        let tool = src.join("tool");
        fs::write(&tool, b"#!/bin/sh\n").expect("写入口程序");
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o644)).expect("归档里就是 0644");
        {
            let f = fs::File::create(cache.join("t.tar")).expect("建 tar");
            let mut b = tar::Builder::new(f);
            b.append_dir_all(".", &src).expect("打包");
            b.finish().expect("收尾");
        }
        let dep = Dep {
            id: "x.archive".to_string(),
            os: "any".to_string(),
            arch: "any".to_string(),
            kind: "archive".to_string(),
            install: ".vm/tool".to_string(),
            probe: ".vm/tool/tool".to_string(),
            guest: None,
            family: None,
            toolchain: None,
            source: "release:t.tar".to_string(),
            strip: 0,
            size_mb: 1,
            sha256: String::new(),
            note: String::new(),
        };
        let links = BTreeMap::from([(
            "release.base".to_string(),
            "https://example.invalid/".to_string(),
        )]);
        run(
            &root,
            &[dep],
            &links,
            &[],
            &AtomicBool::new(false),
            &mut |_| {},
            None,
        )
        .expect("补齐");
        let mode = fs::metadata(root.join(".vm").join("tool").join("tool"))
            .expect("读入口程序")
            .permissions()
            .mode();
        assert!(
            mode & 0o111 != 0,
            "入口程序要能跑（拿到 {:o}）",
            mode & 0o777
        );
        let _ = fs::remove_dir_all(&root);
    }
    #[test]
    fn install_file_uses_probe_name() {
        let root = crate::tests::scratch("provision-file");
        let cache = root.join(".vm").join("cache");
        fs::create_dir_all(&cache).expect("建缓存");
        fs::write(cache.join("a.bin"), b"qcow2").expect("写缓存");
        let dep = Dep {
            id: "g".to_string(),
            os: "any".to_string(),
            arch: "any".to_string(),
            kind: "file".to_string(),
            install: ".vm/guest/debian".to_string(),
            probe: ".vm/guest/debian/rootfs.qcow2".to_string(),
            guest: Some("debian".to_string()),
            family: None,
            toolchain: None,
            source: "release:a.bin".to_string(),
            strip: 0,
            size_mb: 0,
            sha256: String::new(),
            note: String::new(),
        };
        let mut links = BTreeMap::new();
        links.insert(
            "release.base".to_string(),
            "https://example.invalid".to_string(),
        );
        run(
            &root,
            &[dep],
            &links,
            &[],
            &AtomicBool::new(false),
            &mut |_| {},
            None,
        )
        .expect("单文件安装");
        assert!(root
            .join(".vm")
            .join("guest")
            .join("debian")
            .join("rootfs.qcow2")
            .is_file());
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：Windows guest 磁盘——qcow2 落盘并写 guest.yaml 画像。
    #[test]
    fn guest_disk_install_writes_guest_yaml() {
        let root = crate::tests::scratch("provision-disk");
        let cache = root.join(".vm").join("cache");
        fs::create_dir_all(&cache).expect("建缓存");
        fs::write(cache.join("d.qcow2"), b"qcow2").expect("写缓存");
        let dep = Dep {
            id: "g".to_string(),
            os: "windows".to_string(),
            arch: "x86_64".to_string(),
            kind: "guest-disk".to_string(),
            install: ".vm/guest/debian".to_string(),
            probe: ".vm/guest/debian/rootfs.qcow2".to_string(),
            guest: Some("debian".to_string()),
            family: None,
            toolchain: None,
            source: "release:d.qcow2".to_string(),
            strip: 0,
            size_mb: 0,
            sha256: String::new(),
            note: String::new(),
        };
        let mut links = BTreeMap::new();
        links.insert(
            "release.base".to_string(),
            "https://example.invalid".to_string(),
        );
        run(
            &root,
            &[dep],
            &links,
            &[],
            &AtomicBool::new(false),
            &mut |_| {},
            None,
        )
        .expect("guest-disk 安装");
        let base = root.join(".vm").join("guest").join("debian");
        assert!(base.join("rootfs.qcow2").is_file());
        let y = fs::read_to_string(base.join("guest.yaml")).expect("guest.yaml");
        assert!(y.contains("agent: qga"), "{}", y);
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：进度序号单调递增；行窗口只留最近 200 行（前端按 seq 增量取）。
    #[test]
    fn push_line_keeps_seq_and_window() {
        let j = Mutex::new(JobState::default());
        for i in 0..205 {
            push_line(&j, &format!("line {}", i));
        }
        let s = j.lock().expect("锁");
        assert_eq!(s.seq, 205);
        assert_eq!(s.lines.len(), 200);
        assert_eq!(s.lines.first().map(String::as_str), Some("line 5"));
        assert_eq!(s.lines.last().map(String::as_str), Some("line 204"));
    }

    /// 目的：内嵌清单与台账必须自洽——每条依赖的下载源都解析得成 https。
    #[test]
    fn embedded_manifest_and_links_resolve() {
        let deps = deps().expect("清单可解析");
        assert!(
            deps.iter().any(|d| d.id == "vm.qemu.linux"),
            "Linux QEMU 要在清单里"
        );
        let links = links().expect("台账可解析");
        for d in &deps {
            let url = source_url(&d.source, &links)
                .unwrap_or_else(|e| panic!("{} 的下载源解析不了：{}", d.id, e));
            assert!(
                url.starts_with("https://"),
                "{} 的源不是 https：{}",
                d.id,
                url
            );
        }
        let q = deps.iter().find(|d| d.id == "vm.qemu.linux").unwrap();
        assert!(q.source.starts_with("release:"), "自托管包用 release: 前缀");
        let v = deps
            .iter()
            .find(|d| d.id == "vm.virtiofsd.linux")
            .expect("virtiofsd 在清单里");
        assert_eq!(
            v.strip, 3,
            "zip 里 target/<三斜杠>/release/<bin> 四段，剥 3 段"
        );
    }

    /// 目的：自托管 release 源带版本 tag——`release:<tag>/<资产>` 拼成可下载 URL。
    #[test]
    fn release_source_carries_the_release_tag() {
        let all = links().expect("台账可解析");
        let base = all.get("release.base").expect("台账有 release.base");
        assert_eq!(
            source_url("release:virtiofsd/1.14.0/a.bin", &all).expect("要拼得出 URL"),
            format!("{}/virtiofsd/1.14.0/a.bin", base.trim_end_matches('/')),
            "tag 要原样保留在 URL 里"
        );
    }

    /// 目的：清单按本机 os/arch 筛（含 any）；平台不匹配的不出现。
    #[test]
    fn host_filter_selects_only_this_platform() {
        let all = deps().expect("清单可解析");
        let linux = for_host(&all, "linux", "x86_64");
        let ids: Vec<&str> = linux.iter().map(|d| d.id.as_str()).collect();
        assert!(ids.contains(&"vm.qemu.linux") && ids.contains(&"vm.virtiofsd.linux"));
        assert!(!ids.contains(&"vm.qemu.windows"), "windows 条目不该出现");
        assert!(ids.contains(&"vm.guest.linux"), "linux guest 在 linux 上");
        assert!(!ids.contains(&"vm.guest.windows"), "windows guest 不该出现");
    }

    /// 目的：虚拟根清单按 guest 归并——多条条目共用一个根，缺哪条如实列出来。
    #[test]
    fn guest_bases_merge_manifest_entries_by_guest() {
        let root = crate::tests::scratch("provision-guests");
        let base = root.join(".vm").join("guest").join("debian");
        fs::create_dir_all(&base).expect("建虚拟根");
        fs::write(base.join("vmlinuz"), b"x").expect("写内核");
        fs::write(
            base.join("guest.yaml"),
            "id: debian-test\nos: linux\narch: amd64\nagent: qga\nclients: []\nruntimes: [python, node]\n",
        )
        .expect("写画像");
        let dep = |id: &str, probe: &str, guest: &str| Dep {
            id: id.to_string(),
            os: "any".to_string(),
            arch: "any".to_string(),
            kind: "file".to_string(),
            install: ".vm/guest/debian".to_string(),
            probe: probe.to_string(),
            guest: Some(guest.to_string()),
            family: None,
            toolchain: None,
            source: "release:a".to_string(),
            strip: 0,
            size_mb: 0,
            sha256: String::new(),
            note: String::new(),
        };
        let svc = ProvisionService::new(&root);
        // 两条条目同属一个虚拟根：都在场 = 就绪；缺一条 = 如实列出缺哪条。
        let both = svc
            .guest_bases_of(&[
                dep("g.img", ".vm/guest/debian/vmlinuz", "debian"),
                dep("g.rt", ".vm/guest/debian/.runtimes-ready", "debian"),
            ])
            .expect("清单");
        assert_eq!(both.len(), 1, "同一个虚拟根归并成一条：{:?}", both);
        assert_eq!(both[0].guest, "debian");
        assert_eq!(both[0].label, "debian-test", "显示名取画像 id");
        assert_eq!(both[0].os, "linux");
        assert_eq!(
            both[0].runtimes,
            vec!["python".to_string(), "node".to_string()],
            "画像声明的运行能力要带出来（界面据此说明它自带什么）"
        );
        assert!(!both[0].ready, "少一条就不算装齐");
        assert_eq!(both[0].missing, vec!["g.rt".to_string()]);
        assert_eq!(both[0].install, ".vm/guest/debian");
        let one = svc
            .guest_bases_of(&[dep("g.img", ".vm/guest/debian/vmlinuz", "debian")])
            .expect("清单");
        assert!(one[0].ready && one[0].missing.is_empty(), "{:?}", one[0]);
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：全量补齐时运行期编排起不了 guest 只如实跳过（不拖垮其余依赖）；显式点名它仍然硬失败。
    #[test]
    fn guest_runtime_failure_is_soft_for_the_whole_set_but_hard_when_named() {
        let root = crate::tests::scratch("provision-soft-runtime");
        let cache = root.join(".vm").join("cache");
        fs::create_dir_all(&cache).expect("建缓存");
        fs::write(cache.join("a.bin"), b"x").expect("写缓存");
        let mk = |id: &str, kind: &str, install: &str, probe: &str, source: &str| Dep {
            id: id.to_string(),
            os: "any".to_string(),
            arch: "any".to_string(),
            kind: kind.to_string(),
            install: install.to_string(),
            probe: probe.to_string(),
            guest: Some("debian".to_string()),
            family: None,
            toolchain: None,
            source: source.to_string(),
            strip: 0,
            size_mb: 0,
            sha256: String::new(),
            note: String::new(),
        };
        let file_dep = mk("x.file", "file", ".vm/x", ".vm/x/a.bin", "release:a.bin");
        // 虚拟根还没装 + 没注入 vm 能力面：这一条必然失败，正好用来验软失败。
        let rt_dep = mk(
            "vm.guest-runtime.linux",
            "guest-runtime",
            ".vm/guest/debian",
            ".vm/guest/debian/.runtimes-ready",
            "release:a.bin",
        );
        let all = [file_dep, rt_dep];
        let links = BTreeMap::from([(
            "release.base".to_string(),
            "https://example.invalid".to_string(),
        )]);
        let mut lines: Vec<String> = Vec::new();
        let mut sink = |l: &str| lines.push(l.to_string());
        run(
            &root,
            &all,
            &links,
            &[],
            &AtomicBool::new(false),
            &mut sink,
            None,
        )
        .expect("全量补齐不该因为起不了 guest 而失败");
        assert!(
            lines.iter().any(|l| l.contains("运行期编排跳过")),
            "没点名时编排失败要如实跳过：{:?}",
            lines
        );
        assert!(
            root.join(".vm").join("x").join("a.bin").is_file(),
            "其余依赖照样装好"
        );
        // 显式点名这一条 = 你要的就是它：失败即失败。
        let err = run(
            &root,
            &all,
            &links,
            &["vm.guest-runtime.linux".to_string()],
            &AtomicBool::new(false),
            &mut |_| {},
            None,
        )
        .expect_err("显式点名要硬失败");
        assert!(err.contains("vm.guest-runtime.linux"), "{}", err);
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：状态读真实文件系统——probe 在场才判在位。
    #[test]
    fn status_reads_real_filesystem() {
        let root = crate::tests::scratch("provision-status");
        let svc = ProvisionService::new(&root);
        let items = svc.status().expect("状态");
        assert!(
            items.iter().all(|s| !s.present),
            "空根下都算缺：{:?}",
            items
        );
        // 按当前平台挑一条，把 probe 造出来（macOS 无清单条目，跳过）。
        let host = for_host(
            &deps().expect("清单"),
            std::env::consts::OS,
            std::env::consts::ARCH,
        );
        if let Some(dep) = host.into_iter().next() {
            let probe = under(&root, &dep.probe);
            fs::create_dir_all(probe.parent().expect("probe 有父目录")).expect("建目录");
            fs::write(&probe, b"x").expect("写 probe");
            let items = svc.status().expect("状态");
            let hit = items.iter().find(|s| s.id == dep.id).expect("条目在");
            assert!(hit.present, "probe 在场就该判在位：{}", dep.probe);
        }
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：补齐内核——缓存命中时只解包不下载，装完按 probe 校验（不依赖网络）。
    #[test]
    fn install_core_uses_cache_and_checks_probe() {
        let root = crate::tests::scratch("provision-install");
        let cache = root.join(".vm").join("cache");
        fs::create_dir_all(&cache).expect("建缓存");
        let tar_path = cache.join("a.tar");
        {
            let f = fs::File::create(&tar_path).expect("建 tar");
            let mut b = tar::Builder::new(f);
            let mut h = tar::Header::new_gnu();
            h.set_size(5);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, "f.txt", &b"hello"[..])
                .expect("写文件");
            b.finish().expect("收尾");
        }
        let dep = Dep {
            id: "x".to_string(),
            os: "any".to_string(),
            arch: "any".to_string(),
            kind: "archive".to_string(),
            install: ".vm/x".to_string(),
            probe: ".vm/x/f.txt".to_string(),
            guest: None,
            family: None,
            toolchain: None,
            source: "release:a.tar".to_string(),
            strip: 0,
            size_mb: 0,
            sha256: String::new(),
            note: String::new(),
        };
        let mut links = BTreeMap::new();
        links.insert(
            "release.base".to_string(),
            "https://example.invalid".to_string(),
        );
        let mut lines: Vec<String> = Vec::new();
        run(
            &root,
            &[dep],
            &links,
            &[],
            &AtomicBool::new(false),
            &mut |l| lines.push(l.to_string()),
            None,
        )
        .expect("从缓存装");
        assert!(
            root.join(".vm").join("x").join("f.txt").is_file(),
            "装完 probe 要在：{:?}",
            lines
        );
        assert!(lines.iter().any(|l| l.contains("用缓存")), "{:?}", lines);
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：给不存在的 id 要如实拒绝，且不碰网络。
    #[test]
    fn unknown_dep_id_is_refused() {
        let root = crate::tests::scratch("provision-missing");
        let p = ProvisionService::new(&root);
        let err = p
            .ensure(&["不存在".to_string()], &mut |_| {})
            .expect_err("不存在的 id 要拒");
        assert!(err.contains("没有匹配"), "{}", err);
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：缓存文件名取 URL 末段，去掉查询串。
    #[test]
    fn asset_name_takes_last_path_segment() {
        assert_eq!(asset_name("https://x/a/b.tar.zst?q=1"), "b.tar.zst");
        assert_eq!(asset_name("https://x/y.zip#frag"), "y.zip");
    }

    /// 目的：打出来的 guest 磁盘镜像必须是 4 KiB 块——位置参数按块大小解释，不给 -b 时 mke2fs 会
    ///   按小体积选 1 KiB 块的 small 布局，镜像缩成约四分之一而装不下量出来的内容（Linux 真机上炸过）。
    /// 约束：只在 Linux、且 unshare -r 与 mke2fs（e2fsprogs）可用时真跑；否则如实跳过（不假装验过）。
    #[test]
    fn packed_guest_image_uses_4k_blocks() {
        if !cfg!(target_os = "linux") {
            eprintln!("[探针] 打包镜像的块大小只在 Linux 上成立：本平台跳过");
            return;
        }
        use std::io::Read;
        use std::process::Stdio;
        let tools = std::process::Command::new("unshare")
            .args(["-r", "mke2fs", "-V"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !tools {
            eprintln!(
                "[探针] 没有可用的 unshare -r / mke2fs（要 e2fsprogs 与非特权用户命名空间）：跳过"
            );
            return;
        }
        let root = crate::tests::scratch("provision-pack-4k");
        let base = root.join("base");
        let cache = root.join("cache");
        fs::create_dir_all(&base).expect("建 base");
        fs::create_dir_all(&cache).expect("建 cache");
        fs::write(base.join("guest.yaml"), "id: t\nos: linux\narch: amd64\n").expect("写画像");
        fs::write(base.join("hello.txt"), b"hello").expect("写文件");
        pack_guest_base(&base, &cache, &mut |_| {}).expect("打包要成功");
        // ext4 超级块从镜像 1024 偏移开始：magic 在 +0x38，log_block_size 在 +0x18；块大小 = 1024 << log。
        let mut head = [0u8; 2048];
        fs::File::open(base.join("rootfs.ext4"))
            .expect("开镜像")
            .read_exact(&mut head)
            .expect("读镜像头");
        let sb = &head[1024..2048];
        let magic = u16::from_le_bytes([sb[0x38], sb[0x39]]);
        let log_block = u32::from_le_bytes([sb[0x18], sb[0x19], sb[0x1a], sb[0x1b]]);
        assert_eq!(magic, 0xEF53, "得是 ext4 超级块");
        assert_eq!(
            1024u64 << log_block,
            4096,
            "块大小必须是 4 KiB（1 KiB 的真机上装不下）"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：Linux 上真跑 ext4 工具链的前置（unshare -r + mke2fs + debugfs）；缺一个就不是"验过"。
    #[cfg(target_os = "linux")]
    fn guest_image_tools() -> bool {
        use std::process::Stdio;
        std::process::Command::new("unshare")
            .args(["-r", "mke2fs", "-V"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
            && std::process::Command::new("debugfs")
                .arg("-V")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
    }

    /// 目的：造一个带指定 mode 的普通文件（宿主属主；镜像里的属主由 mke2fs 的假 root 决定）。
    #[cfg(target_os = "linux")]
    fn write_mode(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        fs::write(path, b"#!/bin/sh\n").expect("写文件");
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("设权限");
    }

    /// 目的：读宿主目录树上某个条目的权限位（只比低 12 位；解包后这里应当**丢了**特权位）。
    #[cfg(target_os = "linux")]
    fn mode_of(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).expect("读属性").permissions().mode() & 0o7777
    }

    /// 目的：rdump / mke2fs 都会丢**特权位与属主、属组**：按源镜像回填之后，sudo 的 setuid、
    ///   ssh-agent 的 setgid + 组、/tmp 的粘滞位、/var/mail 的 setgid + 组、/etc/shadow 的 shadow 组
    ///   都要和源镜像一致。
    /// 约束：只在本机有 unshare -r / mke2fs / debugfs 时真跑；否则如实跳过（不假装验过）。
    #[cfg(target_os = "linux")]
    #[test]
    fn guest_rootfs_restores_privilege_bits_and_ownership() {
        use std::os::unix::fs::PermissionsExt;
        if !guest_image_tools() {
            eprintln!(
                "[探针] 没有可用的 unshare -r / mke2fs / debugfs（要 e2fsprogs 与非特权用户命名空间）：跳过"
            );
            return;
        }
        let root = crate::tests::scratch("provision-guest-attrs");
        let src = root.join("src");
        let tree = root.join("tree");
        let cache = root.join("cache");
        fs::create_dir_all(&cache).expect("建缓存目录");
        for d in [
            src.join("usr").join("bin"),
            src.join("etc"),
            src.join("tmp"),
            src.join("var").join("mail"),
        ] {
            fs::create_dir_all(&d).expect("建源目录");
        }
        write_mode(&src.join("usr").join("bin").join("sudo"), 0o4755);
        write_mode(&src.join("usr").join("bin").join("ssh-agent"), 0o2755);
        write_mode(&src.join("etc").join("shadow"), 0o640);
        fs::set_permissions(src.join("tmp"), fs::Permissions::from_mode(0o1777)).expect("设 /tmp");
        fs::set_permissions(
            src.join("var").join("mail"),
            fs::Permissions::from_mode(0o2775),
        )
        .expect("设 /var/mail");
        // 源镜像：mke2fs 的假 root 把属主写成 0；组用 sif 写成 cloud 镜像里的事实（宿主上 chown 不了）。
        let source = root.join("source.ext4");
        let st = std::process::Command::new("unshare")
            .args(["-r", "mke2fs", "-q", "-b", "4096", "-t", "ext4", "-F", "-d"])
            .arg(&src)
            .arg(&source)
            .arg("4096")
            .status()
            .expect("跑 mke2fs");
        assert!(st.success(), "造源镜像");
        let mut cmds = String::new();
        for (path, gid) in [
            ("/usr/bin/ssh-agent", 103u32),
            ("/etc/shadow", 42),
            ("/var/mail", 8),
        ] {
            cmds.push_str(&format!(
                "sif {} gid {}\n",
                debugfs_quote(path).expect("路径可写进命令表"),
                gid
            ));
        }
        run_debugfs(&source, &cmds, true).expect("给源镜像写属组");
        // 解包 → 清单 → 打包 → 回填。
        fs::create_dir_all(&tree).expect("建解包目录");
        extract_guest_rootfs(&source, &tree).expect("解包");
        assert_eq!(
            mode_of(&tree.join("usr").join("bin").join("sudo")),
            0o755,
            "rdump 本来就会丢 setuid——这正是回填要补的"
        );
        let attrs = guest_attrs_to_restore(&source, &tree).expect("读源属性");
        for want in [
            "/usr/bin/sudo",
            "/usr/bin/ssh-agent",
            "/etc/shadow",
            "/tmp",
            "/var/mail",
        ] {
            assert!(
                attrs.iter().any(|a| a.path == want),
                "清单里要有 {}：{:?}",
                want,
                attrs.iter().map(|a| a.path.clone()).collect::<Vec<_>>()
            );
        }
        fs::write(tree.join("guest.yaml"), "id: t\nos: linux\narch: amd64\n").expect("写画像");
        pack_guest_base(&tree, &cache, &mut |_| {}).expect("打包");
        let image = tree.join("rootfs.ext4");
        restore_guest_image_attrs(&image, &attrs).expect("回填并核对");
        let parents: Vec<String> = ["/", "/usr/bin", "/etc", "/var"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let got = image_attrs(&image, &parents).expect("读最终镜像");
        for want in [
            "/usr/bin/sudo",
            "/usr/bin/ssh-agent",
            "/etc/shadow",
            "/tmp",
            "/var/mail",
        ] {
            let a = attrs.iter().find(|a| a.path == want).expect("清单条目");
            let g = got
                .iter()
                .find(|g| g.path == want)
                .unwrap_or_else(|| panic!("最终镜像里没有 {}", want));
            assert_eq!(
                (g.mode, g.uid, g.gid),
                (a.mode, a.uid, a.gid),
                "{} 要回填成源镜像的属性",
                want
            );
        }
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：真机验收（opt-in）：拿项目内缓存的 Debian rootfs 分区跑同一条构造链，最终镜像里
    ///   **源镜像所有特权位 / 非 root 属主的条目**都与源一致（不碰 `.vm/guest/debian`，只写 target/test-scratch）。
    /// 约束：只在 SOLOMNI_VM_LIVE=1、本机有工具链、且 `.vm/cache/rootfs.ext4` 在场时真跑；否则 [探针] 如实跳过。
    #[cfg(target_os = "linux")]
    #[test]
    fn guest_rootfs_pipeline_keeps_source_attrs_on_the_real_image() {
        if !guest_image_tools() {
            eprintln!(
                "[探针] 没有可用的 unshare -r / mke2fs / debugfs（要 e2fsprogs 与非特权用户命名空间）：跳过"
            );
            return;
        }
        if std::env::var("SOLOMNI_VM_LIVE").as_deref() != Ok("1") {
            eprintln!(
                "[探针] 未开启虚拟根真机验收：设 SOLOMNI_VM_LIVE=1 后重跑（用 .vm/cache/rootfs.ext4）"
            );
            return;
        }
        let source = crate::tests::root_dir()
            .join(".vm")
            .join("cache")
            .join("rootfs.ext4");
        if !source.is_file() {
            eprintln!(
                "[探针] 本机没有源 rootfs 分区（先 solomni --provision --ensure=vm.guest-rootfs.linux --yes）：跳过"
            );
            return;
        }
        let root = crate::tests::scratch("provision-guest-attrs-live");
        let tree = root.join("tree");
        let cache = root.join("cache");
        fs::create_dir_all(&tree).expect("建解包目录");
        fs::create_dir_all(&cache).expect("建缓存目录");
        extract_guest_rootfs(&source, &tree).expect("解包真源镜像");
        let attrs = guest_attrs_to_restore(&source, &tree).expect("读真源镜像属性");
        let sudo = attrs
            .iter()
            .find(|a| a.path == "/usr/bin/sudo")
            .expect("真源镜像里要有 setuid 的 sudo");
        assert!(
            sudo.mode & 0o4000 != 0,
            "sudo 的 setuid 要读出来：{:o}",
            sudo.mode
        );
        assert!(
            attrs.iter().any(|a| a.gid != 0),
            "清单里要包含非 0 属组（例如 shadow）"
        );
        fs::write(tree.join("guest.yaml"), "id: t\nos: linux\narch: amd64\n").expect("写画像");
        pack_guest_base(&tree, &cache, &mut |_| {}).expect("打包");
        let image = tree.join("rootfs.ext4");
        restore_guest_image_attrs(&image, &attrs).expect("回填并核对");
        // 抽查 sudo 的最终 mode（回填已经在内部核对过全部条目）。
        let got = image_attrs(&image, &["/usr/bin".to_string()]).expect("读最终镜像");
        let sudo = got
            .iter()
            .find(|g| g.path == "/usr/bin/sudo")
            .expect("最终镜像里要有 sudo");
        assert_eq!(sudo.mode & 0o7777, 0o4755, "最终镜像里的 sudo 要带 setuid");
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：kind: toolchain 走同一条补齐内核——从缓存解归档到安装目录，装完把工具登记进 active.json
    ///   （启动期 entry::toolchains 只读这一份）；全程不碰网络。
    #[test]
    fn toolchain_install_extracts_and_registers_the_tool() {
        let root = crate::tests::scratch("provision-toolchain-install");
        let target = host_target();
        let cache = root.join(".vm").join("cache");
        fs::create_dir_all(&cache).expect("建缓存");
        let install = format!(".tools/toolchains/{}/python/9.9.9", target);
        let tar_path = cache.join("py.tar");
        {
            let f = fs::File::create(&tar_path).expect("建 tar");
            let mut b = tar::Builder::new(f);
            let mut h = tar::Header::new_gnu();
            h.set_size(2);
            h.set_mode(0o755);
            h.set_cksum();
            b.append_data(&mut h, "bin/python3", &b"py"[..])
                .expect("写文件");
            b.finish().expect("收尾");
        }
        let dep = Dep {
            id: "toolchain.python.test".to_string(),
            os: "any".to_string(),
            arch: "any".to_string(),
            install: install.clone(),
            probe: format!("{}/bin", install),
            source: "release:py.tar".to_string(),
            kind: "toolchain".to_string(),
            guest: None,
            toolchain: Some(ToolchainSpec {
                tool: "python".to_string(),
                version: "9.9.9".to_string(),
                target: Some(target.clone()),
                bins: vec!["bin".to_string()],
                wrappers: false,
            }),
            family: None,
            strip: 0,
            size_mb: 0,
            sha256: String::new(),
            note: String::new(),
        };
        let mut links = BTreeMap::new();
        links.insert(
            "release.base".to_string(),
            "https://example.invalid".to_string(),
        );
        let mut lines: Vec<String> = Vec::new();
        run(
            &root,
            &[dep],
            &links,
            &[],
            &AtomicBool::new(false),
            &mut |l| lines.push(l.to_string()),
            None,
        )
        .expect("从缓存装工具链");
        assert!(under(&root, &install).join("bin").is_dir(), "{:?}", lines);
        let active = root.join(".tools").join("toolchains").join("active.json");
        let raw = fs::read_to_string(&active).expect("要写 active.json");
        let doc: serde_json::Value = serde_json::from_str(&raw).expect("active.json 是 JSON");
        assert_eq!(doc["target"], serde_json::json!(target));
        assert_eq!(doc["profile"], serde_json::json!(["python"]));
        assert_eq!(doc["tools"][0]["id"], serde_json::json!("python"));
        assert_eq!(
            doc["tools"][0]["bin"][0],
            serde_json::json!(format!("{}/bin", install))
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：Zig 包解出来只有 zig 本体；装完补 cc / gcc / g++ / clang 包装（相对路径，产品根搬移后仍可用）。
    #[test]
    fn zig_toolchain_gets_compiler_wrappers() {
        let root = crate::tests::scratch("provision-toolchain-zig");
        let target = host_target();
        let cache = root.join(".vm").join("cache");
        fs::create_dir_all(&cache).expect("建缓存");
        let install = format!(".tools/toolchains/{}/cc/9.9.9", target);
        let tar_path = cache.join("zig.tar");
        {
            let f = fs::File::create(&tar_path).expect("建 tar");
            let mut b = tar::Builder::new(f);
            let mut h = tar::Header::new_gnu();
            h.set_size(3);
            h.set_mode(0o755);
            h.set_cksum();
            b.append_data(&mut h, "zig-demo/zig", &b"zig"[..])
                .expect("写 zig");
            b.finish().expect("收尾");
        }
        let dep = Dep {
            id: "toolchain.cc.test".to_string(),
            os: "any".to_string(),
            arch: "any".to_string(),
            install: install.clone(),
            probe: format!("{}/bin", install),
            source: "release:zig.tar".to_string(),
            kind: "toolchain".to_string(),
            guest: None,
            toolchain: Some(ToolchainSpec {
                tool: "cc".to_string(),
                version: "9.9.9".to_string(),
                target: Some(target),
                bins: vec!["bin".to_string()],
                wrappers: true,
            }),
            family: None,
            strip: 0,
            size_mb: 0,
            sha256: String::new(),
            note: String::new(),
        };
        let mut links = BTreeMap::new();
        links.insert(
            "release.base".to_string(),
            "https://example.invalid".to_string(),
        );
        run(
            &root,
            &[dep],
            &links,
            &[],
            &AtomicBool::new(false),
            &mut |_| {},
            None,
        )
        .expect("装 Zig 工具链");
        let bin = under(&root, &install).join("bin");
        let zig = fs::read_to_string(bin.join("zig")).expect("zig 包装");
        assert!(
            zig.contains("../zig-demo/zig"),
            "zig 包装要指向包内本体：{}",
            zig
        );
        assert!(
            zig.contains("ZIG_GLOBAL_CACHE_DIR") && zig.contains("cache/global"),
            "zig 包装要把缓存钉在安装目录内：{}",
            zig
        );
        let gcc = fs::read_to_string(bin.join("gcc")).expect("gcc 包装");
        assert!(gcc.contains("zig\" cc"), "gcc 包装要调 zig cc：{}", gcc);
        let gxx = fs::read_to_string(bin.join("g++")).expect("g++ 包装");
        assert!(gxx.contains("zig\" c++"), "g++ 包装要调 zig c++：{}", gxx);
        assert!(
            gcc.contains("--target=*unknown-*") && gcc.contains("--target=\\1-"),
            "gcc 包装要把 Rust 三元组的 unknown- 去掉且保留 --target= 前缀：{}",
            gcc
        );
        for name in ["cc", "gcc", "g++", "c++", "clang", "clang++"] {
            assert!(bin.join(name).is_file(), "缺包装：{}", name);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(bin.join("gcc"))
                .expect("看模式")
                .permissions()
                .mode();
            assert!(mode & 0o111 != 0, "包装要有执行位：{:o}", mode);
        }
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：全量补齐时已在场的基础工具链不重装（260 MB 的 MinGW 不该每次重解）；显式点名才强制重装。
    #[test]
    fn present_toolchain_is_skipped_on_full_ensure() {
        let root = crate::tests::scratch("provision-toolchain-skip");
        let target = host_target();
        let install = format!(".tools/toolchains/{}/python/9.9.9", target);
        fs::create_dir_all(under(&root, &install).join("bin")).expect("铺已装好的工具");
        let dep = Dep {
            id: "toolchain.python.test".to_string(),
            os: "any".to_string(),
            arch: "any".to_string(),
            install: install.clone(),
            probe: format!("{}/bin", install),
            source: "release:missing.tar".to_string(),
            kind: "toolchain".to_string(),
            guest: None,
            toolchain: Some(ToolchainSpec {
                tool: "python".to_string(),
                version: "9.9.9".to_string(),
                target: Some(target),
                bins: vec!["bin".to_string()],
                wrappers: false,
            }),
            family: None,
            strip: 0,
            size_mb: 0,
            sha256: String::new(),
            note: String::new(),
        };
        let mut links = BTreeMap::new();
        links.insert("release.base".to_string(), "http://127.0.0.1:1".to_string());
        let mut lines: Vec<String> = Vec::new();
        run(
            &root,
            std::slice::from_ref(&dep),
            &links,
            &[],
            &AtomicBool::new(false),
            &mut |l| lines.push(l.to_string()),
            None,
        )
        .expect("在位就跳过，不碰网络");
        assert!(
            lines.iter().any(|l| l.contains("已就位（跳过）")),
            "{:?}",
            lines
        );
        // 显式点名 = 你要的就是它：缓存里没有、源不可达时如实失败，不假装成功。
        let err = run(
            &root,
            &[dep],
            &links,
            &["toolchain.python.test".to_string()],
            &AtomicBool::new(false),
            &mut |_| {},
            None,
        )
        .expect_err("点名就必须真装");
        assert!(err.contains("下载失败"), "{}", err);
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：校验不符拒绝落盘——报台账值 / 实际值，安装目录一个字节都不写；缓存保留（证据与续传都靠它）。
    #[test]
    fn toolchain_pin_mismatch_refuses_to_land() {
        let root = crate::tests::scratch("provision-toolchain-pin");
        let target = host_target();
        let cache = root.join(".vm").join("cache");
        fs::create_dir_all(&cache).expect("建缓存");
        fs::write(cache.join("py.tar"), b"abc").expect("写缓存");
        let install = format!(".tools/toolchains/{}/python/9.9.9", target);
        let dep = Dep {
            id: "toolchain.python.test".to_string(),
            os: "any".to_string(),
            arch: "any".to_string(),
            install: install.clone(),
            probe: format!("{}/bin", install),
            source: "release:py.tar".to_string(),
            kind: "toolchain".to_string(),
            guest: None,
            toolchain: Some(ToolchainSpec {
                tool: "python".to_string(),
                version: "9.9.9".to_string(),
                target: Some(target),
                bins: vec!["bin".to_string()],
                wrappers: false,
            }),
            family: None,
            strip: 0,
            size_mb: 0,
            sha256: "0".repeat(64),
            note: String::new(),
        };
        let mut links = BTreeMap::new();
        links.insert(
            "release.base".to_string(),
            "https://example.invalid".to_string(),
        );
        let err = run(
            &root,
            &[dep],
            &links,
            &[],
            &AtomicBool::new(false),
            &mut |_| {},
            None,
        )
        .expect_err("pin 不符要拒绝");
        assert!(err.contains("sha256 与台账不符"), "{}", err);
        assert!(err.contains(&"0".repeat(64)), "要报台账值：{}", err);
        assert!(
            err.contains("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
            "要报实际值：{}",
            err
        );
        assert!(!under(&root, &install).exists(), "校验失败不许落盘");
        assert!(
            cache.join("py.tar").is_file(),
            "不删缓存（证据与续传都靠它）"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：镜像只是候选之一（上游在前），GitHub 资产才加；选源排除不可达的候选，全都不可达时说清状态码。
    #[test]
    fn mirror_and_source_choice() {
        assert_eq!(
            mirror_of("https://github.com/a/b/c.zip", "https://gh.example/"),
            Some("https://gh.example/github.com/a/b/c.zip".to_string())
        );
        assert_eq!(
            mirror_of("https://example.test/a.zip", "https://gh.example"),
            None
        );
        assert_eq!(mirror_of("https://github.com/a/b", ""), None);
        let sample = |url: &str, code: u16, kbps: f64| (url.to_string(), code, kbps);
        let picked = choose_fastest(&[
            sample("https://a", 200, 10.0),
            sample("https://b", 200, 900.0),
            sample("https://c", 404, 0.0),
        ])
        .expect("要选出最快的可达源");
        assert_eq!(picked, "https://b");
        let err = choose_fastest(&[sample("https://a", 0, 0.0)]).unwrap_err();
        assert!(err.contains("没有可用的下载源"), "{}", err);
    }

    /// 目的：断点续传——分片与来源标记一致时带 Range 续上；来源不明就从头下（不把两份字节接起来）。
    #[test]
    fn resume_uses_range_only_for_a_known_source() {
        use std::sync::Arc;
        use std::time::Duration;
        let body: &'static [u8] = b"0123456789";
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").expect("起本地源"));
        let addr = server.server_addr().to_ip().expect("TCP 地址");
        let url = format!("http://127.0.0.1:{}/a.bin", addr.port());
        let stop = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(Vec::<Option<String>>::new()));
        let (srv, st, sn) = (Arc::clone(&server), Arc::clone(&stop), Arc::clone(&seen));
        let handle = std::thread::spawn(move || {
            while !st.load(Ordering::Relaxed) {
                let Ok(Some(req)) = srv.recv_timeout(Duration::from_millis(50)) else {
                    continue;
                };
                let range = req
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("Range"))
                    .map(|h| h.value.as_str().to_string());
                sn.lock().expect("锁").push(range.clone());
                let resp = match range {
                    Some(r) => {
                        let start: usize = r
                            .trim_start_matches("bytes=")
                            .trim_end_matches('-')
                            .parse()
                            .unwrap_or(0);
                        tiny_http::Response::from_data(body[start..].to_vec()).with_status_code(206)
                    }
                    None => tiny_http::Response::from_data(body.to_vec()),
                };
                let _ = req.respond(resp);
            }
        });
        let root = crate::tests::scratch("provision-resume");
        let dest = root.join("a.bin");
        let part = part_path(&dest);
        // 分片 + 来源标记一致：续传（Range: bytes=<断点>-）。
        fs::write(&part, &body[..4]).expect("写分片");
        fs::write(part_source_marker(&part), &url).expect("写来源标记");
        let agent = crate::kernel::detail::outbound::agent(5, 30);
        download_with_resume(&agent, &url, &dest, &mut |_| {}).expect("续传");
        assert_eq!(
            fs::read(&dest).expect("读下载结果"),
            body,
            "续传后内容要完整"
        );
        assert!(!part.exists(), "成功后分片要改名掉");
        // 来历不明的分片：从头下（不发 Range）。
        fs::write(&part, b"XYZ").expect("写来历不明的分片");
        download_with_resume(&agent, &url, &dest, &mut |_| {}).expect("从头下");
        assert_eq!(fs::read(&dest).expect("读下载结果"), body);
        let ranges = seen.lock().expect("锁").clone();
        assert_eq!(ranges.len(), 2, "{:?}", ranges);
        assert_eq!(ranges[0].as_deref(), Some("bytes=4-"), "首个请求要带断点");
        assert_eq!(ranges[1], None, "来源不明必须从头下");
        stop.store(true, Ordering::Relaxed);
        handle.join().expect("收本地源线程");
        let _ = fs::remove_dir_all(&root);
    }
}
