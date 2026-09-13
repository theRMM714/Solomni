//! 模块清单来源：扫描 modules/ 目录（实现 core 的 ModuleSource 端口）。
//! 目录遍历与 yaml 解析是机制；「清单即事实」的重扫策略由 core 决定。

use crate::core::module::{Module, ModuleManifest, Roster};
use crate::core::ports::ModuleSource;
use std::path::{Path, PathBuf};

pub struct FsModules {
    dir: PathBuf,
}

impl FsModules {
    pub fn new(dir: PathBuf) -> FsModules {
        FsModules { dir }
    }
}

impl ModuleSource for FsModules {
    fn scan(&self) -> Roster {
        scan_dir(&self.dir)
    }
}

/// 列出 modules/ 下每个含合法 module.yaml 的文件夹。
/// 放入即出现，移出即消失；id 与文件夹名不一致 = 非法，拒收并说明原因。
fn scan_dir(modules_dir: &Path) -> Roster {
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
        match serde_yaml::from_str::<ModuleManifest>(&yaml_text) {
            Ok(m) => {
                let dir_name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                if m.id != dir_name {
                    rejected.push(format!("{}: id '{}' 与文件夹名不一致", dir_name, m.id));
                    continue;
                }
                modules.push(Module { manifest: m, root: path });
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
