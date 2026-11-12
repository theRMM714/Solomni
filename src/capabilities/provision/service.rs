//! 目的：依赖补齐的编排——清单筛选、缓存与安装身份、按 kind 派给端口、作业与取消。
//! 管：什么时候下 / 验 / 装 / 记台账 / 清旧安装，进度行与后台作业状态，guest 根的重建流程。
//! 不管：网络取数（Fetcher）、宿主落点（HostFs）、安装机制（Installers）、镜像构造（GuestImage）、
//!   台账格式（Ledger）、进程事实（Procs）、清单语义与规则（domain）。
//! 联动：呈现层经 api::Provision；组合根构造六个适配器并注入；guest-runtime 的运行期编排在 guest.rs。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::capabilities::provision::api::{DepStatus, GuestBase, JobState, ProcInfo, Provision};
use crate::capabilities::provision::domain::{
    Dep, deps, for_host, install_identity, is_toolchain, links, pinned_sha256, source_url, under,
};
use crate::capabilities::provision::ports::{
    Fetcher, GuestImage, HostFs, Installers, Ledger, Procs,
};
use crate::capabilities::vm::api::Vm;
use crate::kernel::ports::HostProbe;

/// 目的：一次补齐要用的端口组合——由 `ProvisionService` 借出，后台线程自己持有一份。
pub(crate) struct Pipeline<'a> {
    pub(crate) root: &'a Path,
    pub(crate) host: &'a dyn HostFs,
    pub(crate) ledger: &'a dyn Ledger,
    pub(crate) installers: &'a dyn Installers,
    pub(crate) images: &'a dyn GuestImage,
    pub(crate) fetch: &'a dyn Fetcher,
    pub(crate) probe: &'a dyn HostProbe,
    pub(crate) vm: Option<&'a (dyn Vm + Send + Sync)>,
}

/// 目的：组合根交给补齐能力的全部出站端口——端口由能力声明，适配器由根在装配时选。
pub struct ProvisionDeps {
    pub host: Arc<dyn HostFs>,
    pub ledger: Arc<dyn Ledger>,
    pub installers: Arc<dyn Installers>,
    pub images: Arc<dyn GuestImage>,
    pub procs: Arc<dyn Procs>,
    pub fetch: Arc<dyn Fetcher>,
    pub probe: Arc<dyn HostProbe>,
}

/// 目的：依赖补齐的实现——持产品根与全部出站端口；作业状态与取消旗在本层。
pub struct ProvisionService {
    root: PathBuf,
    host: Arc<dyn HostFs>,
    ledger: Arc<dyn Ledger>,
    installers: Arc<dyn Installers>,
    images: Arc<dyn GuestImage>,
    procs: Arc<dyn Procs>,
    fetch: Arc<dyn Fetcher>,
    probe: Arc<dyn HostProbe>,
    /// Windows guest 的运行期编排要起 guest；组合根或 --provision 入口注入；只查状态时可为 None。
    vm: Option<Arc<dyn Vm + Send + Sync>>,
    job: Arc<Mutex<JobState>>,
    cancel: Arc<AtomicBool>,
}

impl ProvisionService {
    /// 目的：以产品根与全部出站端口建一个实现（只查状态、或只装不启动 guest 的条目用；guest-runtime 会被如实拒绝）。
    pub fn new(root: &Path, deps: ProvisionDeps) -> ProvisionService {
        ProvisionService {
            root: root.to_path_buf(),
            host: deps.host,
            ledger: deps.ledger,
            installers: deps.installers,
            images: deps.images,
            procs: deps.procs,
            fetch: deps.fetch,
            probe: deps.probe,
            vm: None,
            job: Arc::new(Mutex::new(JobState::default())),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 目的：带 vm 能力面的实现（组合根 / --provision 入口用）；guest-runtime 编排需要它。
    pub fn with_vm(
        root: &Path,
        deps: ProvisionDeps,
        vm: Arc<dyn Vm + Send + Sync>,
    ) -> ProvisionService {
        ProvisionService {
            vm: Some(vm),
            ..ProvisionService::new(root, deps)
        }
    }

    fn pipeline(&self) -> Pipeline<'_> {
        Pipeline {
            root: &self.root,
            host: self.host.as_ref(),
            ledger: self.ledger.as_ref(),
            installers: self.installers.as_ref(),
            images: self.images.as_ref(),
            fetch: self.fetch.as_ref(),
            probe: self.probe.as_ref(),
            vm: self.vm.as_deref(),
        }
    }

    /// 目的：同步补齐（CLI 用）：逐条下载 / 解包 / 校验，进度逐行回调。
    pub fn ensure(&self, ids: &[String], on_line: &mut dyn FnMut(&str)) -> Result<(), String> {
        run_ensure(&self.pipeline(), ids, &self.cancel, on_line)
    }

    /// 目的：`--ensure=<取值>` 的补齐入口：kind 组别名只补缺项（如 `toolchain`），
    ///   id 列表按原语义点名安装（空列表 = 清单全部）。
    /// 约束：组别名一条都不缺时如实说「都在场」且不碰网络；在位项不重装。
    pub fn ensure_spec(&self, spec: &str, on_line: &mut dyn FnMut(&str)) -> Result<(), String> {
        let all = for_host(&deps()?, std::env::consts::OS, std::env::consts::ARCH);
        match split_ensure_spec(&all, spec)? {
            EnsureSpec::Ids(ids) => self.ensure(&ids, on_line),
            EnsureSpec::Kinds(kinds) => {
                let pending = pending_of_kinds(&all, self.ledger.as_ref(), &kinds);
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
                    &self.pipeline(),
                    &all,
                    &links()?,
                    &pending,
                    &self.cancel,
                    on_line,
                )
            }
        }
    }
}

impl Provision for ProvisionService {
    fn status(&self) -> Result<Vec<DepStatus>, String> {
        let links = links()?;
        let ledger = self.ledger.snapshot();
        let mut out = Vec::new();
        for dep in for_host(&deps()?, std::env::consts::OS, std::env::consts::ARCH) {
            let url = source_url(&dep.source, &links).ok();
            let present = installed(self.ledger.as_ref(), &ledger, &dep);
            let stale = !present && self.ledger.present(&dep);
            out.push(DepStatus {
                id: dep.id,
                guest: dep.guest.clone(),
                present,
                stale,
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

    /// 约束：账本是"我们亲手起的长驻子进程"这一份事实源；alive 由适配器逐条问系统（死条目如实留着，等对账清）。
    fn processes(&self) -> Result<Vec<ProcInfo>, String> {
        Ok(self.procs.entries())
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
        let probe = Arc::clone(&self.probe);
        let fetch = Arc::clone(&self.fetch);
        let host = Arc::clone(&self.host);
        let ledger = Arc::clone(&self.ledger);
        let installers = Arc::clone(&self.installers);
        let images = Arc::clone(&self.images);
        std::thread::spawn(move || {
            let sink = Arc::clone(&job);
            let mut on_line = move |line: &str| push_line(&sink, line);
            let pipeline = Pipeline {
                root: &root,
                host: host.as_ref(),
                ledger: ledger.as_ref(),
                installers: installers.as_ref(),
                images: images.as_ref(),
                fetch: fetch.as_ref(),
                probe: probe.as_ref(),
                vm: vm.as_deref(),
            };
            let r = run_ensure(&pipeline, &ids, &cancel, &mut on_line);
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
        if self.procs.guest_running() {
            return Err("还有 guest 在跑：先停止所有虚拟机会话，再还原虚拟根".to_string());
        }
        let dir = crate::capabilities::vm::api::resolve_guest(&self.root, guest)?;
        let lock = self
            .root
            .join(".vm")
            .join("build-cache")
            .join(format!("{}.lock", guest));
        if self.host.exists(&lock) {
            return Err("这个虚拟根有构建在跑：等构建结束再还原".to_string());
        }
        if self.host.is_dir(&dir) {
            on_line(&format!("删除旧虚拟根 {}", guest));
            self.host
                .remove_dir_all(&dir)
                .map_err(|e| format!("删除旧虚拟根失败（{}）：{}", dir.display(), e))?;
        }
        let ids: Vec<String> = picked.iter().map(|d| d.id.clone()).collect();
        on_line(&format!("按清单重建 {}（{} 条依赖）", guest, ids.len()));
        self.ensure(&ids, on_line)
    }
}

impl ProvisionService {
    /// 目的：在给定清单上归并虚拟根（清单由调用方给，便于用例注入；`guest_bases` 只是它的"本机清单"版）。
    pub(crate) fn guest_bases_of(&self, deps: &[Dep]) -> Result<Vec<GuestBase>, String> {
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
        let ledger = self.ledger.snapshot();
        let mut out = Vec::new();
        for name in order {
            let members = &by_guest[&name];
            let missing: Vec<String> = members
                .iter()
                .filter(|d| !installed(self.ledger.as_ref(), &ledger, d))
                .map(|d| d.id.clone())
                .collect();
            let install = members
                .first()
                .map(|d| d.install.clone())
                .unwrap_or_default();
            let base = under(&self.root, &install);
            let profile = self.ledger.profile(&base);
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
pub(crate) fn push_line(job: &Mutex<JobState>, line: &str) {
    if let Ok(mut j) = job.lock() {
        j.seq += 1;
        j.lines.push(line.to_string());
        if j.lines.len() > 200 {
            j.lines.remove(0);
        }
    }
}

/// 目的：这条依赖现在算不算装好——探针在场 **且** 台账里的身份与当前清单一致。
/// 约束：只看探针在场不算装好（pin 升级后旧产物仍然在场，这正是上一版会漏掉的缺口）。
pub(crate) fn installed(
    ledger: &dyn Ledger,
    snapshot: &BTreeMap<String, String>,
    dep: &Dep,
) -> bool {
    ledger.present(dep) && snapshot.get(&dep.id) == Some(&install_identity(dep))
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
pub(crate) fn pending_of_kinds(all: &[Dep], ledger: &dyn Ledger, kinds: &[String]) -> Vec<String> {
    let snapshot = ledger.snapshot();
    let mut ids: Vec<String> = Vec::new();
    for dep in all {
        if kinds.iter().any(|k| k == &dep.kind)
            && !installed(ledger, &snapshot, dep)
            && !ids.contains(&dep.id)
        {
            ids.push(dep.id.clone());
        }
    }
    ids
}

/// 目的：从 URL 取末段做缓存文件名（去掉查询串与锚点）。
pub(crate) fn asset_name(url: &str) -> String {
    url.split(['?', '#'])
        .next()
        .unwrap_or(url)
        .rsplit('/')
        .next()
        .unwrap_or("asset")
        .to_string()
}

/// 目的：缓存里那份文件与 pin 的 sha256 一致吗——**下载完要验，复用旧缓存也要验**。
/// 参数：id = 依赖 id（报错点名）；pinned = pin 的校验值（None = 没 pin）；cached = 缓存文件；url = 下载源。
/// 约束：不符时**不删缓存**——删了既丢证据、又会在 pin 过期时变成"每次重下 165 MB"的死循环，改由人看结论决定。
pub(crate) fn verify_cached(
    id: &str,
    pinned: Option<&str>,
    cached: &Path,
    url: &str,
    fetch: &dyn Fetcher,
) -> Result<(), String> {
    let Some(want) = pinned else {
        return Ok(());
    };
    let want = want.trim().to_ascii_lowercase();
    let got = fetch.digest(cached)?;
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

/// 目的：从清单取本机条目并跑一次补齐（作业与 CLI 共用）。
pub(crate) fn run_ensure(
    pipeline: &Pipeline<'_>,
    ids: &[String],
    cancel: &AtomicBool,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let all = deps()?;
    let links = links()?;
    let all = for_host(&all, std::env::consts::OS, std::env::consts::ARCH);
    run(pipeline, &all, &links, ids, cancel, on_line)
}

/// 目的：在给定的依赖清单上执行补齐（清单/台账由调用方给，便于用例注入）。
pub(crate) fn run(
    pipeline: &Pipeline<'_>,
    all: &[Dep],
    links: &BTreeMap<String, String>,
    ids: &[String],
    cancel: &AtomicBool,
    on_line: &mut dyn FnMut(&str),
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
    let cache = pipeline.root.join(".vm").join("cache");
    pipeline.host.mkdir_all(&cache)?;
    let mut ledger = pipeline.ledger.snapshot();
    for dep in picked {
        if cancel.load(Ordering::Relaxed) {
            on_line("已取消");
            return Ok(());
        }
        // 基础工具链是每次启动都要用的运行前提：全量补齐时已在场就不重装
        // （260 MB 的 MinGW 重解一遍没有意义）；其余 kind 的「在位即跳过」由调用方筛 pending。
        if is_toolchain(dep) && !explicit && installed(pipeline.ledger, &ledger, dep) {
            on_line(&format!("{}：已就位（跳过）", dep.id));
            continue;
        }
        let url = source_url(&dep.source, links)?;
        let asset = asset_name(&url);
        let cached = cache.join(&asset);
        let identity = install_identity(dep);
        let stale = pipeline.ledger.present(dep) && ledger.get(&dep.id) != Some(&identity);
        if stale {
            on_line(&format!("{}：装的是旧版本，清掉重装", dep.id));
            pipeline.ledger.clear(dep)?;
            pipeline.ledger.forget(&dep.id)?;
            ledger = pipeline.ledger.snapshot();
        }
        // 运行期编排自己不下东西：它要的镜像必须已由 guest-disk 装好；缺了编排会如实拒绝。
        if dep.kind != "guest-runtime" {
            let pinned = pinned_sha256(dep, links);
            // 缓存按 pin 现验：旧 pin 下缓存的那份不是错误证据，而是该换一份——删掉重下一次。
            let cached_ok = pipeline.host.is_file(&cached)
                && verify_cached(&dep.id, pinned.as_deref(), &cached, &url, pipeline.fetch).is_ok();
            if cached_ok {
                on_line(&format!("{}：用缓存 {}", dep.id, asset));
            } else {
                if pipeline.host.is_file(&cached) {
                    on_line(&format!("{}：缓存与 pin 不符，重新下载", dep.id));
                    pipeline.host.remove_file(&cached)?;
                } else {
                    on_line(&format!("{}：下载 {}", dep.id, url));
                }
                pipeline.fetch.download(&url, &cached, on_line)?;
            }
            // 完整性闸：刚下好的这份必须与 pin 一致（再不符就是台账或上游的问题，如实报错）。
            verify_cached(&dep.id, pinned.as_deref(), &cached, &url, pipeline.fetch)?;
        }
        let dest = under(pipeline.root, &dep.install);
        pipeline.host.mkdir_all(&dest)?;
        let installed = match dep.kind.as_str() {
            "appimage" => pipeline
                .installers
                .appimage(&cached, &dest, on_line)
                .map_err(|e| format!("{}：{}", dep.id, e)),
            "file" => pipeline
                .installers
                .file(&cached, &dest, &dep.probe)
                .map_err(|e| format!("{}：{}", dep.id, e)),
            "guest-disk" => pipeline
                .installers
                .guest_disk(&cached, &dest, &dep.probe, on_line)
                .map_err(|e| format!("{}：{}", dep.id, e)),
            "guest-rootfs" => pipeline
                .images
                .install(dep, &cached, on_line)
                .map_err(|e| format!("{}：{}", dep.id, e)),
            "toolchain" => pipeline
                .installers
                .toolchain(&cached, &dest, dep, on_line)
                .map_err(|e| format!("{}：{}", dep.id, e)),
            "guest-runtime" => {
                let r = super::guest::install_guest_runtime(
                    pipeline.root,
                    dep,
                    pipeline.probe,
                    pipeline.vm,
                    explicit,
                    on_line,
                );
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
            _ => pipeline
                .installers
                .extract(&cached, &dest, dep.strip)
                .map_err(|e| format!("{}：解包失败：{}", dep.id, e)),
        };
        installed?;
        let probe = under(pipeline.root, &dep.probe);
        if !pipeline.host.exists(&probe) {
            return Err(format!("{}：装完仍找不到 {}", dep.id, dep.probe));
        }
        // 归档里的 probe 就是入口程序：zip 可能没记 unix 权限位，解出来会是 0644——补可执行位。
        if dep.kind == "archive" {
            pipeline.host.make_executable(&probe);
        }
        pipeline.ledger.record(dep)?;
        // 台账以磁盘为准：上面清旧安装时写的是本轮开始时的快照，
        // 不刷新就会把前几条刚登记的身份覆盖掉（表现为「装完的条目又变成没装」）。
        ledger = pipeline.ledger.snapshot();
        on_line(&format!("{}：就绪（{}）", dep.id, dep.install));
    }
    // 装完就把基础工具链登记进 active.json：启动期只读这一份（entry::toolchains）；
    // 单独点名装一条时也要刷新，所以登记以磁盘为准（见 sync_toolchain_active）。
    if all.iter().any(is_toolchain) {
        pipeline.ledger.sync_active(all)?;
    }
    Ok(())
}
