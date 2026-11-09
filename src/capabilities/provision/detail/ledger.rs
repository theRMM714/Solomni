//! 目的：安装台账与在场判定——把 `.vm/state/deps.json` 与探针落盘成 Ledger 的实现。
//! 管：台账读写、安装身份比对、清旧安装、guest.yaml 画像读取、工具链 active.json 登记。
//! 不管：什么时候重装、装什么（在 service 的编排里）；下载与解包（在 fetch / install）。
//! 联动：组合根构造 `FileLedger` 注入 service；本文件只经 domain 的纯规则算身份。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::capabilities::provision::domain::{
    host_target, install_identity, is_toolchain, join_rel, under, Dep, GuestProfile,
};
use crate::capabilities::provision::ports::Ledger;

/// 目的：文件台账——产品根下的 `.vm/state/deps.json`。
pub struct FileLedger {
    root: PathBuf,
}

impl FileLedger {
    pub fn new(root: &Path) -> FileLedger {
        FileLedger {
            root: root.to_path_buf(),
        }
    }
}

/// 目的：读虚拟根画像里的 `id` / `os` / `arch` / `runtimes`（`guest.yaml`；不在场 = 空画像）。
/// 约束：只读几个扁平标量与一个内联列表，不引 YAML 依赖。
pub(crate) fn read_guest_profile(base: &Path) -> GuestProfile {
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

/// 目的：这条依赖的 probe 是否已在场（文件或目录；与 status 的判据同一把尺子）。
pub(crate) fn probe_present(root: &Path, dep: &Dep) -> bool {
    let p = under(root, &dep.probe);
    p.is_file() || p.is_dir()
}

/// 目的：台账落点——产品根下 `.vm/state/deps.json`。
pub(crate) fn ledger_path(root: &Path) -> PathBuf {
    root.join(".vm").join("state").join("deps.json")
}

/// 目的：读台账；不在场或坏 JSON 都按空台账（首次安装的常态）。
pub(crate) fn read_ledger(root: &Path) -> BTreeMap<String, String> {
    fs::read_to_string(ledger_path(root))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// 目的：写台账——临时文件再改名，中途被杀不会留下半份 JSON。
pub(crate) fn write_ledger(root: &Path, ledger: &BTreeMap<String, String>) -> Result<(), String> {
    let file = ledger_path(root);
    let dir = file.parent().ok_or_else(|| "台账没有父目录".to_string())?;
    fs::create_dir_all(dir).map_err(|e| format!("建 {} 失败：{}", dir.display(), e))?;
    let text =
        serde_json::to_string_pretty(ledger).map_err(|e| format!("序列化安装台账失败：{}", e))?;
    let tmp = file.with_extension("json.tmp");
    fs::write(&tmp, text + "\n").map_err(|e| format!("写 {} 失败：{}", tmp.display(), e))?;
    fs::rename(&tmp, &file).map_err(|e| format!("提交 {} 失败：{}", file.display(), e))?;
    Ok(())
}

/// 目的：登记「这条依赖按当前身份装好了」——每装好一条就落一次盘（后面某条失败时前面的成果不丢）。
pub(crate) fn record_installed(root: &Path, dep: &Dep) -> Result<(), String> {
    let mut ledger = read_ledger(root);
    ledger.insert(dep.id.clone(), install_identity(dep));
    write_ledger(root, &ledger)
}

/// 目的：清掉这条依赖自己产出的那份东西——只有身份变了才调；共享同一安装目录的条目只动自己的探针。
pub(crate) fn clear_install(root: &Path, dep: &Dep) -> Result<(), String> {
    let probe = under(root, &dep.probe);
    let install = under(root, &dep.install);
    let removed = match dep.kind.as_str() {
        "guest-runtime" | "file" => {
            if probe.is_file() {
                fs::remove_file(&probe)
            } else if probe.is_dir() {
                fs::remove_dir_all(&probe)
            } else {
                Ok(())
            }
        }
        _ => {
            if install.is_dir() {
                fs::remove_dir_all(&install)
            } else {
                Ok(())
            }
        }
    };
    removed.map_err(|e| format!("{}：清旧安装失败：{}", dep.id, e))
}

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

impl Ledger for FileLedger {
    fn snapshot(&self) -> BTreeMap<String, String> {
        read_ledger(&self.root)
    }

    fn present(&self, dep: &Dep) -> bool {
        probe_present(&self.root, dep)
    }

    fn profile(&self, base: &Path) -> GuestProfile {
        read_guest_profile(base)
    }

    fn record(&self, dep: &Dep) -> Result<(), String> {
        record_installed(&self.root, dep)
    }

    fn clear(&self, dep: &Dep) -> Result<(), String> {
        clear_install(&self.root, dep)
    }

    fn forget(&self, id: &str) -> Result<(), String> {
        let mut ledger = read_ledger(&self.root);
        ledger.remove(id);
        write_ledger(&self.root, &ledger)
    }

    fn sync_active(&self, all: &[Dep]) -> Result<(), String> {
        sync_toolchain_active(&self.root, all)
    }
}
