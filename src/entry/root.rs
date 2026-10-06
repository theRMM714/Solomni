//! 目的：产品根规范化——传入的根变成**干净的绝对路径**。
//! 管：纯词法拼接与归一化（不解析 `..`、不碰盘符大小写、不碰盘）；取不到当前目录时退回 canonicalize（并剥掉 Windows 的 `\\?\` 前缀）。
//! 不管：路径是否真实存在（不验证）；各能力怎么用这个根。
//! 联动：由入口层调用（`src/main.rs`）——解析出的根交给组合根（日志、围栏回收、各能力）。

use std::path::PathBuf;

/// 目的：把传入的产品根规范成干净的绝对路径。
/// 参数：`raw` 绝对路径直接用；相对路径与当前目录做**纯词法**拼接。
/// 返回：规范化的根；以及一条说明——退回别的办法时写明原因，正常情况是 `None`。
/// 约束：只有取不到当前目录才退回 canonicalize（Windows 上并剥掉 `\\?\` 扩展长度前缀）。
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
