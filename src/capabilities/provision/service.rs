//! 目的：依赖补齐能力的实现——清单解析、状态、下载缓存、解包落盘与一次后台作业。
//! 管：内嵌 dependencies.yaml（清单）与 links.yaml（台账）的解析、按本机筛选、下载 / 解包 / 校验、作业状态与取消。
//! 不管：虚拟机档的能力语义（在 vm）；界面怎么展示（在 presentation）。
//! 联动：呈现层经 api::Provision 用它；组合根构造它；下载走 kernel::detail::outbound，解包走 kernel::detail::unpack。

use super::api::{DepStatus, GuestBase, JobState, Provision};
use crate::capabilities::vm::api::Vm;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

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
    /// 目的：安装方式：archive（默认，解归档）/ appimage / file（单文件直下）/ guest-rootfs（多步构造）。
    #[serde(default = "default_kind")]
    pub kind: String,
    /// 目的：归属的虚拟根名（只有 guest 类条目有）；一个虚拟根可以由多条条目共同装。
    #[serde(default)]
    pub guest: Option<String>,
    /// 目的：解包时剥掉归档里的前 N 段目录（便携包常带一层包装目录）；缺省 0。
    #[serde(default)]
    pub strip: usize,
    #[serde(default)]
    pub size_mb: u64,
    #[serde(default)]
    pub note: String,
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

/// 目的：把 source 解析成可下载的 URL——`release:<资产名>` 走自托管包，其余按 links.yaml 的键。
pub fn source_url(source: &str, links: &BTreeMap<String, String>) -> Result<String, String> {
    if let Some(asset) = source.strip_prefix("release:") {
        let base = link("release.base").ok_or_else(|| "links.yaml 缺 release.base".to_string())?;
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
    /// 目的：画像里声明的运行能力（内联列表 `runtimes: [python, node]`；缺画像 = 空）。
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
                current: String::new(),
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

/// 目的：把 URL 下到目标文件——先写 .part 再改名，避免半截文件被当成缓存。
fn download(
    agent: &ureq::Agent,
    url: &str,
    dest: &Path,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let resp = agent
        .get(url)
        .call()
        .map_err(|e| format!("下载失败（{}）：{}", url, e))?;
    if !resp.status().is_success() {
        return Err(format!("下载 {} 返回 {}", url, resp.status().as_u16()));
    }
    let body = resp.into_body();
    let mut reader = body.into_reader();
    let tmp = dest.with_extension("part");
    let mut f = fs::File::create(&tmp).map_err(|e| format!("建 {} 失败：{}", tmp.display(), e))?;
    let n =
        io::copy(&mut reader, &mut f).map_err(|e| format!("写 {} 失败：{}", tmp.display(), e))?;
    drop(f);
    fs::rename(&tmp, dest).map_err(|e| format!("改名 {} 失败：{}", dest.display(), e))?;
    on_line(&format!("下载完成：{} 字节", n));
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
        "# guest root 由内核命令行给出（9p），这里不写 fstab 条目。
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

/// 目的：在 guest 里生成带 9p 模块的 initrd（chroot 走非特权用户命名空间，不需要 root）。
fn ensure_guest_initrd(
    base: &Path,
    abi: &str,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let conf = base.join("etc").join("initramfs-tools").join("modules");
    fs::create_dir_all(conf.parent().expect("有父目录")).map_err(|e| e.to_string())?;
    let mut lines: Vec<String> = fs::read_to_string(&conf)
        .unwrap_or_default()
        .lines()
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty())
        .collect();
    for m in [
        "9p",
        "9pnet",
        "9pnet_virtio",
        "virtio",
        "virtio_pci",
        "virtio_ring",
    ] {
        if !lines.iter().any(|l| l == m) {
            lines.push(m.to_string());
        }
    }
    fs::write(
        &conf,
        format!(
            "{}
",
            lines.join(
                "
"
            )
        ),
    )
    .map_err(|e| format!("写 {} 失败：{}", conf.display(), e))?;
    let script = base.join("tmp").join("solomni-initrd.sh");
    fs::create_dir_all(script.parent().expect("有父目录")).map_err(|e| e.to_string())?;
    fs::write(
        &script,
        format!(
            "#!/bin/bash
set -e
depmod {0}
mkinitramfs -o /boot/initrd.img-{0} {0}
",
            abi
        ),
    )
    .map_err(|e| format!("写 initrd 脚本失败：{}", e))?;
    on_line("在 chroot（用户命名空间，无需 root）里生成带 9p 的 initrd");
    let inner = format!(
        "mount --bind /proc {0} 2>/dev/null || true; chroot {1} /bin/bash /tmp/solomni-initrd.sh",
        base.join("proc").display(),
        base.display()
    );
    let st = std::process::Command::new("unshare")
        .args(["-r", "-m", "sh", "-c", &inner])
        .status()
        .map_err(|e| {
            format!(
                "跑不了 unshare（需要 util-linux 且内核允许非特权用户命名空间）：{}",
                e
            )
        })?;
    let _ = fs::remove_file(&script);
    if !st.success() {
        return Err("生成 initrd 失败（需要内核允许非特权用户命名空间）".to_string());
    }
    if !base
        .join("boot")
        .join(format!("initrd.img-{}", abi))
        .is_file()
    {
        return Err("initrd 没生成出来".to_string());
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
    if !base.join("etc").join("os-release").is_file() {
        on_line(&format!("把 rootfs 解到 {}（debugfs rdump）", dep.install));
        run_tool(
            std::process::Command::new("debugfs")
                .arg("-R")
                .arg(format!("rdump / {}", base.display()))
                .arg(&rootfs),
            "debugfs",
        )?;
    }
    let abi = ensure_guest_kernel(&cache, &base, agent, on_line)?;
    ensure_guest_qga(&cache, &base, agent, on_line)?;
    sanitize_guest_base(&base)?;
    ensure_guest_initrd(&base, &abi, on_line)?;
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
            source: "release:t.tar".to_string(),
            strip: 0,
            size_mb: 1,
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
            source: "release:a.bin".to_string(),
            strip: 0,
            size_mb: 0,
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
            source: "release:d.qcow2".to_string(),
            strip: 0,
            size_mb: 0,
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
            source: "release:a".to_string(),
            strip: 0,
            size_mb: 0,
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
            source: source.to_string(),
            strip: 0,
            size_mb: 0,
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
            source: "release:a.tar".to_string(),
            strip: 0,
            size_mb: 0,
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
}
