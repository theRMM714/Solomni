//! 目的：PackageSource 端口的磁盘实现——扫描 runtimes/ 下每个含 package.yaml 的文件夹。
//! 管：目录遍历与 yaml 解析；清单即事实，放在依赖文件夹里即出现，移出即消失。
//! 不管：清单校验、去重与冲突预检（在 domain/manifest.rs）。
//! 联动：端口在 ports.rs；只由组合根构造。

use crate::capabilities::packages::domain::manifest::{Library, PackageManifest};
use crate::capabilities::packages::ports::PackageSource;
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
            let folder = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let yaml_path = path.join("package.yaml");
            let text = match std::fs::read_to_string(&yaml_path) {
                Ok(t) => t,
                Err(_) => {
                    rejected.push(format!("{}：缺少 package.yaml", folder));
                    continue;
                }
            };
            match yaml_serde::from_str::<PackageManifest>(&text) {
                Ok(mut m) => {
                    m.source_dir = path.clone();
                    // 认平台子目录（os-arch）：分平台包里只挂与 guest 匹配的那份（见 library_for_guest）。
                    if let Ok(subs) = std::fs::read_dir(&path) {
                        for sub in subs.flatten() {
                            let sub_path = sub.path();
                            if !sub_path.is_dir() {
                                continue;
                            }
                            let name = sub.file_name().to_string_lossy().to_string();
                            if crate::capabilities::packages::domain::manifest::is_platform_dir_name(
                                &name,
                            ) {
                                m.platform_dirs.insert(name, sub_path);
                            }
                        }
                    }
                    found.push(m);
                }
                Err(e) => rejected.push(format!("{}：package.yaml 非法（{}）", folder, e)),
            }
        }
        Library::build(found, rejected)
    }
}
