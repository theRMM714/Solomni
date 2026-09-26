//! 产品根规范化：传入的根 → **干净的绝对路径**。
//! 机制层（读当前目录、必要时 canonicalize），所以不在 kernel、也不在组合根里。

use std::path::PathBuf;

/// 产品根 → 干净的绝对路径：用 current_dir 与传入的根做**纯词法**拼接（不去解析 ..、不碰盘符大小写、不碰盘）。
/// 只有"取不到 current_dir"这种异常情况才退回 canonicalize（并剥掉 Windows 的 \\?\ 扩展长度前缀）。
pub fn resolve_root(raw: &std::path::Path) -> (PathBuf, Option<String>) {
    match std::env::current_dir() {
        Ok(cwd) => {
            let joined = if raw.is_absolute() {
                raw.to_path_buf()
            } else {
                cwd.join(raw)
            };
            (lexical_abs(&joined), None)
        }
        Err(e) => match std::fs::canonicalize(raw) {
            Ok(p) => (
                strip_unc_prefix(p),
                Some(format!(
                    "取不到当前目录（{}）：改用 canonicalize 规范化产品根",
                    e
                )),
            ),
            Err(e2) => (
                lexical_abs(raw),
                Some(format!(
                    "取不到当前目录（{}），canonicalize 也失败（{}）：产品根可能不是绝对路径",
                    e, e2
                )),
            ),
        },
    }
}

/// 纯词法归一化：去掉 . 段、收掉重复与尾部分隔符（components 自带），保留盘符/根前缀与 .. 段（不解析）。
fn lexical_abs(p: &std::path::Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::Prefix(_) | std::path::Component::RootDir => {
                out.push(c.as_os_str())
            }
            std::path::Component::Normal(s) => out.push(s),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => out.push(".."),
        }
    }
    out
}

/// Windows 上 canonicalize 会给出 \\?\C:\… 形式：它对多数工具可用但会污染提示词，去掉这个前缀。
#[cfg(windows)]
fn strip_unc_prefix(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy().into_owned();
    match s.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => p,
    }
}

#[cfg(not(windows))]
fn strip_unc_prefix(p: PathBuf) -> PathBuf {
    p
}
