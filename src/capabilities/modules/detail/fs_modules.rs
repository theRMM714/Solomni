//! 目的：模块清单来源——扫描 `modules/` 目录（实现 `ModuleSource` 端口）。
//! 管：目录遍历、yaml 解析、`userdata/` 的确保存在、拒收原因如实呈现。
//! 不管：清单契约与校验（在 `domain/module.rs`）；重扫策略（在 `service.rs`）。
//! 联动：由本能力 `service.rs` 持有；见 docs/modules/README.md。

use crate::capabilities::modules::api::{Module, ModuleManifest, Roster};
use crate::capabilities::modules::ports::ModuleSource;
use std::path::{Path, PathBuf};

pub struct FsModules {
    /// **保留名表**（内置工具名）：由组合根在装配期问一次 `tools` 交进来——
    /// 清单校验归本能力，名字空间归工具能力，两边不互相依赖。
    reserved: Vec<String>,
    dir: PathBuf,
}

impl FsModules {
    pub fn new(dir: PathBuf, reserved: Vec<String>) -> FsModules {
        FsModules { dir, reserved }
    }
}

impl ModuleSource for FsModules {
    fn scan(&self) -> Roster {
        scan_dir(&self.dir, &self.reserved)
    }
}

/// 列出 modules/ 下每个含合法 module.yaml 的文件夹。
/// 放入即出现，移出即消失；id 与文件夹名不一致 = 非法，拒收并说明原因。
fn scan_dir(modules_dir: &Path, reserved: &[String]) -> Roster {
    let mut modules = Vec::new();
    let mut rejected = Vec::new();
    let entries = match std::fs::read_dir(modules_dir) {
        Ok(e) => e,
        Err(_) => return Roster { modules, rejected },
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let yaml_path = path.join("module.yaml");
        let yaml_text = match std::fs::read_to_string(&yaml_path) {
            Ok(t) => t,
            Err(_) => {
                rejected.push(format!(
                    "{}: 缺少 module.yaml",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ));
                continue;
            }
        };
        match yaml_serde::from_str::<ModuleManifest>(&yaml_text) {
            Ok(m) => {
                let dir_name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                if m.id != dir_name {
                    rejected.push(format!("{}: id '{}' 与文件夹名不一致", dir_name, m.id));
                    continue;
                }
                if let Err(why) = crate::capabilities::modules::api::check_runtimes(&m) {
                    rejected.push(format!("{}: {}", dir_name, why));
                    continue;
                }
                if let Err(why) = crate::capabilities::modules::api::check_tools(&m, reserved) {
                    rejected.push(format!("{}: {}", dir_name, why));
                    continue;
                }
                if let Err(why) = crate::capabilities::modules::api::check_services(&m) {
                    rejected.push(format!("{}: {}", dir_name, why));
                    continue;
                }
                if let Err(why) = crate::capabilities::modules::api::check_secrets(&m) {
                    rejected.push(format!("{}: {}", dir_name, why));
                    continue;
                }
                // 载入即确保私有区存在：模块恒有一个可写的 userdata/（产品唯一的自动写盘，幂等）。
                // 建不了不阻断加载（只读挂载、权限不足）：如实记一条，事实字段给"能不能用"。
                let userdata = path.join("userdata");
                let has_userdata = userdata.is_dir() || std::fs::create_dir_all(&userdata).is_ok();
                if !has_userdata {
                    eprintln!(
                        "[模块] {} 的 userdata 建不了：这一席拿不到私有可写落点",
                        dir_name
                    );
                }
                modules.push(Module {
                    manifest: m,
                    root: path.clone(),
                    has_userdata,
                });
            }
            Err(e) => rejected.push(format!(
                "{}: module.yaml 非法（{}）",
                path.file_name().unwrap_or_default().to_string_lossy(),
                e
            )),
        }
    }
    modules.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    Roster { modules, rejected }
}
