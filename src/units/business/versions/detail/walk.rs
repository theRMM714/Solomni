//! 目的：工作区目录遍历的唯一实现（列文件、稳定排序、可选规则剪枝与条数上限）。
//! 管：从根往下递归列文件（只列文件）；相对路径一律 / 分隔；逐层按名字排序；给了规则就把命中的子树整棵剪掉。
//! 不管：不判权限、不读文件内容、不跟随符号链接；是否把链接当文件由调用方决定。
//! 联动：`fs_workstore` 的 list / list_visible 与 `fs_workspace` 的清单共用这一份，规则语义只在 `domain/path_filter.rs`。

use crate::units::business::workspace::api::path_filter::Rules;
use std::path::{Path, PathBuf};

/// 目的：递归列文件到 out（只列文件），相对路径用 / 分隔；超过 limit 条就停。
/// 参数：root 是遍历根；prefix 是已经走过的相对前缀（顶层传空串）；rules 为 None 时不过滤。
pub(super) fn collect(
    root: &Path,
    out: &mut Vec<String>,
    prefix: &str,
    rules: Option<&Rules>,
    limit: usize,
) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let mut items: Vec<(String, PathBuf, bool)> = entries
        .flatten()
        .map(|e| {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            let is_dir = p.is_dir();
            (name, p, is_dir)
        })
        .collect();
    items.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, path, is_dir) in items {
        if out.len() >= limit {
            return;
        }
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{}/{}", prefix, name)
        };
        if rules.is_some_and(|r| r.is_ignored(&rel, is_dir)) {
            continue;
        }
        if is_dir {
            collect(&path, out, &rel, rules, limit);
        } else {
            out.push(rel);
        }
    }
}
