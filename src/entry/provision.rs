//! 目的：入口层的依赖补齐机制——依赖清单、下载源解析、状态报告与下载安装作业。
//! 管：内嵌 `dependencies.yaml`（清单）与 `links.yaml`（台账）的解析、按本机筛选、下载缓存、解包落盘与作业状态。
//! 不管：虚拟机档的能力语义（在 capabilities/vm）；界面怎么展示（在 presentation）。
//! 联动：清单与台账的唯一真相在仓库根，产品里**编译期内嵌**；命令行入口是 `--provision`，作业经 `Provisioner`。

use crate::kernel::ports::HostProbe;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// 清单与台账在构建期嵌进二进制：分发的二进制不依赖仓库里的文件，读到的仍是同一份真相。
const DEPS_YAML: &str = include_str!("../../dependencies.yaml");
const LINKS_YAML: &str = include_str!("../../links.yaml");

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

/// 目的：一条依赖在本机的状态——在不在场、装到哪、从哪个 URL 来。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepStatus {
    pub dep: Dep,
    pub present: bool,
    pub path: PathBuf,
    /// 目的：解析出来的下载 URL（`release:` 前缀换成 `release.base`）；解析不了给 None。
    pub url: Option<String>,
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
        let base = links
            .get("release.base")
            .ok_or_else(|| "links.yaml 缺 release.base".to_string())?;
        return Ok(format!("{}/{}", base.trim_end_matches('/'), asset));
    }
    links
        .get(source)
        .cloned()
        .ok_or_else(|| format!("links.yaml 里没有这个键：{}", source))
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

/// 目的：本机每一条依赖的状态——probe 指向的路径存在即在位；URL 解析失败如实标 None。
pub fn status(
    root: &Path,
    os: &str,
    arch: &str,
    probe: &dyn HostProbe,
) -> Result<Vec<DepStatus>, String> {
    let links = links()?;
    let mut out = Vec::new();
    for dep in for_host(&deps()?, os, arch) {
        let path = under(root, &dep.probe);
        out.push(DepStatus {
            present: probe.is_file(&path) || probe.is_dir(&path),
            url: source_url(&dep.source, &links).ok(),
            path,
            dep,
        });
    }
    Ok(out)
}

/// 目的：依赖补齐的执行体——持产品根；底层同步补齐走它（作业与 Web 面在 provision 能力那一步）。
pub struct Provisioner {
    root: PathBuf,
}

impl Provisioner {
    /// 目的：以产品根建一个执行体。
    pub fn new(root: &Path) -> Provisioner {
        Provisioner {
            root: root.to_path_buf(),
        }
    }

    /// 目的：同步补齐：逐条下载 / 解包 / 校验，进度逐行回调。
    pub fn ensure(&self, ids: &[String], on_line: &mut dyn FnMut(&str)) -> Result<(), String> {
        run_ensure(&self.root, ids, on_line)
    }
}

/// 目的：补齐的执行核心——选中依赖、逐条下载(带缓存) + 解包 + 校验；作业与同步入口共用。
fn run_ensure(root: &Path, ids: &[String], on_line: &mut dyn FnMut(&str)) -> Result<(), String> {
    let all = deps()?;
    let links = links()?;
    let all = for_host(&all, std::env::consts::OS, std::env::consts::ARCH);
    run(root, &all, &links, ids, on_line)
}

/// 目的：在给定的依赖清单上执行补齐（清单/台账由调用方给，便于用例注入）。
fn run(
    root: &Path,
    all: &[Dep],
    links: &BTreeMap<String, String>,
    ids: &[String],
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
    let agent = crate::capabilities::llm::detail::http_agent::agent(15, 7200);
    let cache = root.join(".vm").join("cache");
    fs::create_dir_all(&cache).map_err(|e| format!("建缓存目录失败：{}", e))?;
    for dep in picked {
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
        crate::diagnostics::unpack::extract_into(&cached, &dest, dep.strip)
            .map_err(|e| format!("{}：解包失败：{}", dep.id, e))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::doubles::FixedProbe;

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
    }

    /// 目的：按本机筛出的状态——平台不匹配的不出现，probe 在场才判在位。
    #[test]
    fn host_filter_and_status_report_presence() {
        let root = crate::tests::scratch("provision-status");
        let probe = FixedProbe::default();
        let items = status(&root, "linux", "x86_64", &probe).expect("状态");
        let ids: Vec<&str> = items.iter().map(|s| s.dep.id.as_str()).collect();
        assert!(ids.contains(&"vm.qemu.linux") && ids.contains(&"vm.virtiofsd.linux"));
        assert!(!ids.contains(&"vm.qemu.windows"), "windows 条目不该出现");
        assert!(items.iter().all(|s| !s.present), "什么都没声明 = 全缺");
        assert!(items.iter().all(|s| s.url.is_some()), "每条都要有 URL");

        let want = under(&root, ".vm/qemu/bin/qemu-system-x86_64");
        let mut probe2 = FixedProbe::default();
        probe2.files.push(want);
        let items2 = status(&root, "linux", "x86_64", &probe2).expect("状态");
        let q = items2.iter().find(|s| s.dep.id == "vm.qemu.linux").unwrap();
        assert!(q.present, "probe 在场就该判在位");
        let _ = std::fs::remove_dir_all(&root);
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
        run(&root, &[dep], &links, &[], &mut |l| {
            lines.push(l.to_string())
        })
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
        let p = Provisioner::new(&root);
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
