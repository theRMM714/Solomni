//! 目的：共享区相对路径的校验与拼接——所有 `work_*` 入口的第一道关。
//! 管：`clean_rel` / `validate_paths` / `join_rel` / `expand_paths` / `lost_rel`。
//! 不管：路径落在哪个根之内的校验由调用方在 `join_rel` 之后再判；备份内容的写入在 `workspace::detail`。
//! 联动：被 `merge.rs` 的提交计划与 `workspace/service` 消费（docs/workspace/README.md）。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// 目的：校验一条共享区相对路径：非空、以 `/` 分隔、无空段、无 `.` / `..`、无控制字符、无反斜杠。
///    只做路径本身的事；落在哪个根之内由调用方用 `join_rel` 之后再校验。
pub fn clean_rel(raw: &str) -> Result<String, String> {
    if raw.is_empty() {
        return Err("路径不能为空".to_string());
    }
    if raw.contains('\\') {
        return Err(format!("路径要用 / 分隔：{}", raw));
    }
    if raw.starts_with('/') {
        return Err(format!("路径必须是共享区内的相对路径：{}", raw));
    }
    let mut parts: Vec<&str> = Vec::new();
    for seg in raw.split('/') {
        if seg.is_empty() {
            return Err(format!("路径不能有空段或首尾斜杠：{}", raw));
        }
        if seg == "." || seg == ".." {
            return Err(format!("路径不能包含 . 或 ..：{}", raw));
        }
        if seg.chars().any(|c| c.is_control()) {
            return Err(format!("路径不能包含控制字符：{}", raw));
        }
        parts.push(seg);
    }
    Ok(parts.join("/"))
}

/// 目的：校验一组共享区相对路径：所有 `work_*` 的 `paths` 入口都先过这里。
///   非法路径整条拒绝，不再降级成 `missing`（否则“路径写错”会被误报成“上游没有”）。
pub fn validate_paths(paths: &[String]) -> Result<(), String> {
    for raw in paths {
        clean_rel(raw)?;
    }
    Ok(())
}

/// 目的：把一条已校验的相对路径拼到一个根下（一律用路径组件，禁止把分隔符写进字符串再拼）。
pub fn join_rel(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let clean = clean_rel(rel)?;
    let mut p = root.to_path_buf();
    for seg in clean.split('/') {
        p.push(seg);
    }
    Ok(p)
}

/// 目的：把 `paths`（文件或文件夹）展开成具体文件集合：路径本身是文件就取它，
///    是文件夹就取该前缀下的全部文件（available 是候选文件全集）。返回（展开后的路径，未命中的请求）。
pub fn expand_paths(paths: &[String], available: &BTreeSet<String>) -> (Vec<String>, Vec<String>) {
    let mut out: BTreeSet<String> = BTreeSet::new();
    let mut missing: Vec<String> = Vec::new();
    for raw in paths {
        let Ok(rel) = clean_rel(raw) else {
            missing.push(raw.clone());
            continue;
        };
        if available.contains(&rel) {
            out.insert(rel);
            continue;
        }
        let prefix = format!("{}/", rel);
        let hit = available.iter().any(|p| p.starts_with(&prefix));
        if hit {
            for p in available.iter().filter(|p| p.starts_with(&prefix)) {
                out.insert(p.clone());
            }
        } else {
            missing.push(rel);
        }
    }
    (out.into_iter().collect(), missing)
}

/// 目的：把一个 agent 的一条路径映射到版本库里的备份相对路径：`lost/<agent>/<rel>`。
/// 约束：agent 必须能作为单个路径组件；rel 必须是干净的共享区相对路径。
pub fn lost_rel(agent: &str, rel: &str) -> Result<String, String> {
    let rel = clean_rel(rel)?;
    if agent.is_empty()
        || agent == "."
        || agent == ".."
        || agent.contains('/')
        || agent.contains('\\')
    {
        return Err(format!("agent 名不能用作备份目录：{}", agent));
    }
    Ok(format!("lost/{}/{}", agent, rel))
}
