//! 目的：`host_fs` 的非 Windows 后端——不过滤隐藏项，根只有 `/`。
//! 管：目录项过滤（恒 false）与卷根清单。
//! 不管：HTTP 与 JSON。
//! 联动：由 `src/presentation/web/host_fs.rs` 在非 Windows 目标选中。

/// 目的：非 Windows 不做隐藏 / 系统属性过滤。
pub(crate) fn is_hidden_or_system(_entry: &std::fs::DirEntry) -> bool {
    false
}

/// 目的：文件系统根只有 `/`。
pub(crate) fn volume_roots() -> Vec<String> {
    vec!["/".to_string()]
}
