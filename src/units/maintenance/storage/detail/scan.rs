//! 目的：存储盘点的文件系统原语——目录大小、修改时间、目录项枚举。
//! 管：只读扫描；跳过符号链接目标，避免把根外内容算进来。
//! 不管：业务分类与回收决策。
//! 联动：storage::service 用它们组装 SpaceReport。

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// 目的：一个目录/文件的实际占用（目录递归；文件取 len）。
pub fn bytes(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if !meta.is_dir() {
        return meta.len();
    }
    let mut total = 0u64;
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        let Ok(m) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        if m.file_type().is_symlink() {
            continue;
        }
        if m.is_dir() {
            total = total.saturating_add(bytes(&p));
        } else {
            total = total.saturating_add(m.len());
        }
    }
    total
}

/// 目的：最近修改时间（unix 秒）；取不到给 None。
pub fn modified(path: &Path) -> Option<u64> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    let t = meta.modified().ok()?;
    t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}

/// 目的：目录下的直接子项（文件或目录），按名字排序，跳过符号链接。
pub fn children(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        out.push(p);
    }
    out.sort();
    out
}

/// 目的：目录下指定扩展名的文件（大小写不敏感），按名字排序。
pub fn files_with_ext(dir: &Path, ext: &str) -> Vec<PathBuf> {
    children(dir)
        .into_iter()
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case(ext))
                    .unwrap_or(false)
        })
        .collect()
}

/// 目的：文件/目录名的显示串；非法 UTF-8 如实替换。
pub fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}
