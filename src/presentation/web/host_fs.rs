//! 目的：目录选择器的宿主文件系统事实——隐藏/系统项过滤与卷根清单。
//! 管：按平台判断目录项是否该隐藏、列出可选根。
//! 不管：HTTP 路由与 JSON 渲染（在 `web.rs`）。
//! 联动：`src/presentation/web.rs` 的目录选择请求使用。

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as backend;
#[cfg(not(windows))]
mod unix;
#[cfg(not(windows))]
use unix as backend;

/// 目的：目录项是否应被隐藏（Windows 的隐藏 / 系统属性；其余平台不过滤）。
pub(crate) fn is_hidden_or_system(entry: &std::fs::DirEntry) -> bool {
    backend::is_hidden_or_system(entry)
}

/// 目的：目录选择器可列出的根（Windows 盘符根；其余平台只有 `/`）。
pub(crate) fn volume_roots() -> Vec<String> {
    backend::volume_roots()
}
