//! 目的：`host_fs` 的 Windows 后端——隐藏/系统属性与盘符根。
//! 管：过滤 `$RECYCLE.BIN` 这类目录；列出存在的盘符根。
//! 不管：HTTP 与 JSON。
//! 联动：由 `src/presentation/web/host_fs.rs` 按平台选中。

/// 目的：目录项带隐藏或系统属性。
pub(crate) fn is_hidden_or_system(entry: &std::fs::DirEntry) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    entry
        .metadata()
        .map(|m| m.file_attributes() & (HIDDEN | SYSTEM) != 0)
        .unwrap_or(false)
}

/// 目的：列出当前存在的盘符根（A:–Z:）。
pub(crate) fn volume_roots() -> Vec<String> {
    let mut roots = Vec::new();
    for b in b'A'..=b'Z' {
        let p = format!("{}:\\", b as char);
        if std::path::Path::new(&p).is_dir() {
            roots.push(p);
        }
    }
    roots
}
