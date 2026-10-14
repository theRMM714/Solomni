//! 目的：依赖补齐能力的实现——清单解析、状态、下载缓存、解包落盘与一次后台作业。
//! 管：内嵌 dependencies.yaml（清单）与 links.yaml（台账）的解析、按本机筛选、下载 / 解包 / 校验、作业状态与取消。
//! 不管：虚拟机档的能力语义（在 vm）；界面怎么展示（在 presentation）。
//! 联动：呈现层经 api::Provision 用它；组合根构造它；下载走 kernel::detail::outbound，解包走 kernel::detail::unpack。

use super::api::{DepStatus, JobState, Provision};
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

/// 目的：一条依赖——它是什么、装到哪、怎么判在场、从哪来。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Dep {
    pub id: String,
    pub os: String,
    pub arch: String,
    pub install: String,
    pub probe: String,
    pub source: String,
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
fn under(root: &Path, rel: &str) -> PathBuf {
    let mut out = root.to_path_buf();
    for seg in rel.split('/').filter(|s| !s.is_empty()) {
        out.push(seg);
    }
    out
}

/// 目的：本机（os/arch）要用的依赖清单（含 `any`）。
pub fn for_host(deps: &[Dep], os: &str, arch: &str) -> Vec<Dep> {
    deps.iter()
        .filter(|d| (d.os == os || d.os == "any") && (d.arch == arch || d.arch == "any"))
        .cloned()
        .collect()
}

/// 目的：依赖补齐的实现——持产品根、作业状态与取消旗；下载 / 解包在下面同一文件。
pub struct ProvisionService {
    root: PathBuf,
    job: Arc<Mutex<JobState>>,
    cancel: Arc<AtomicBool>,
}

impl ProvisionService {
    /// 目的：以产品根建一个实现（进程内一个就够；作业状态是进程内的）。
    pub fn new(root: &Path) -> ProvisionService {
        ProvisionService {
            root: root.to_path_buf(),
            job: Arc::new(Mutex::new(JobState::default())),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 目的：同步补齐（CLI 用）：逐条下载 / 解包 / 校验，进度逐行回调。
    pub fn ensure(&self, ids: &[String], on_line: &mut dyn FnMut(&str)) -> Result<(), String> {
        run_ensure(&self.root, ids, &self.cancel, on_line)
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
        std::thread::spawn(move || {
            let sink = Arc::clone(&job);
            let mut on_line = move |line: &str| push_line(&sink, line);
            let r = run_ensure(&root, &ids, &cancel, &mut on_line);
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
) -> Result<(), String> {
    let all = deps()?;
    let links = links()?;
    let all = for_host(&all, std::env::consts::OS, std::env::consts::ARCH);
    run(root, &all, &links, ids, cancel, on_line)
}

/// 目的：在给定的依赖清单上执行补齐（清单/台账由调用方给，便于用例注入）。
fn run(
    root: &Path,
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
        if cached.is_file() {
            on_line(&format!("{}：用缓存 {}", dep.id, asset));
        } else {
            on_line(&format!("{}：下载 {}", dep.id, url));
            download(&agent, &url, &cached, on_line)?;
        }
        let dest = under(root, &dep.install);
        fs::create_dir_all(&dest).map_err(|e| format!("建安装目录 {} 失败：{}", dep.install, e))?;
        let lower = cached
            .file_name()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if lower.ends_with(".appimage") {
            install_appimage(&cached, &dest, on_line).map_err(|e| format!("{}：{}", dep.id, e))?;
        } else {
            crate::kernel::detail::unpack::extract_into(&cached, &dest, dep.strip)
                .map_err(|e| format!("{}：解包失败：{}", dep.id, e))?;
        }
        let probe = under(root, &dep.probe);
        if !(probe.is_file() || probe.is_dir()) {
            return Err(format!("{}：装完仍找不到 {}", dep.id, dep.probe));
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(ids.contains(&"vm.guest.rootfs"), "any 条目哪个平台都在");
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
            install: ".vm/x".to_string(),
            probe: ".vm/x/f.txt".to_string(),
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
