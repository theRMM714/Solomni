//! 目的：`AssetFiles` 的本机实现——`std::fs` 读写只在这一层出现。
//! 管：读全文、递归收 `.yaml`（不跟符号链接）。
//! 不管：介质名与文案（在 `domain` / `service`）。
//! 联动：由组合根注入 `service::AssetsService`；契约见 docs/assets/README.md。

use crate::units::assets::ports::AssetFiles;
use std::path::{Path, PathBuf};

/// 目的：本机文件系统上的随包只读内容。
#[derive(Debug, Clone, Copy, Default)]
pub struct FsAssetFs;

impl AssetFiles for FsAssetFs {
    fn read(&self, path: &Path) -> Result<String, String> {
        std::fs::read_to_string(path).map_err(|e| e.to_string())
    }

    fn yaml_tree(&self, dir: &Path) -> Result<Vec<PathBuf>, String> {
        let mut out = Vec::new();
        collect_yaml(dir, &mut out).map_err(|e| e.to_string())?;
        Ok(out)
    }
}

/// 递归收集 `.yaml`（不跟符号链接；排序由 `service` 做）。
fn collect_yaml(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_yaml(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
            out.push(path);
        }
    }
    Ok(())
}
