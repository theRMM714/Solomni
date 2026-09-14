//! 运行包库来源：扫描 runtimes/ 下每个含 package.yaml 的文件夹（实现 core 的 PackageSource 端口）。
//! 目录遍历与 yaml 解析是机制；清单校验、去重、冲突预检在 core（packages::Library::build）。
//! 「清单即事实」：放在依赖文件夹里即出现，移出即消失——没有注册仪式。

use crate::core::packages::{Library, PackageManifest};
use crate::core::ports::PackageSource;
use std::path::PathBuf;

pub struct FsPackages {
    dir: PathBuf,
}

impl FsPackages {
    pub fn new(dir: PathBuf) -> FsPackages {
        FsPackages { dir }
    }
}

impl PackageSource for FsPackages {
    fn dir(&self) -> PathBuf {
        self.dir.clone()
    }

    fn scan(&self) -> Library {
        let mut found = Vec::new();
        let mut rejected = Vec::new();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(_) => return Library::default(),
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let folder = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            let yaml_path = path.join("package.yaml");
            let text = match std::fs::read_to_string(&yaml_path) {
                Ok(t) => t,
                Err(_) => {
                    rejected.push(format!("{}：缺少 package.yaml", folder));
                    continue;
                }
            };
            match serde_yaml::from_str::<PackageManifest>(&text) {
                Ok(m) => found.push(m),
                Err(e) => rejected.push(format!("{}：package.yaml 非法（{}）", folder, e)),
            }
        }
        Library::build(found, rejected)
    }
}
