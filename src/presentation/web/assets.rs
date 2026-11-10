//! 目的：浏览器端资源的嵌入表与按 URL 路径查找。
//! 管：引入 `build.rs` 生成的 `(路径, MIME, 内容)` 表；按路径精确查找（只命中表里有的文件，不读文件系统）。
//! 不管：路由匹配与缓存头（`src/presentation/web/routes.rs`、`src/presentation/web/mod.rs`）；哪些文件进表（`build.rs`）。
//! 联动：`GET /` 取 `index.html`，`GET /assets/{*path}` 取其余资源；表由 `build.rs` 扫描
//!   `src/presentation/web/assets/` 生成。

/// 目的：编译期嵌入的全部资源，按 URL 路径排序。
pub(crate) const ASSETS: &[(&str, &str, &str)] =
    include!(concat!(env!("OUT_DIR"), "/web_assets.rs"));

/// 目的：按资源目录内的相对 URL 路径取 `(MIME, 内容)`。
/// 返回：表里没有这条路径时为 `None`（含 `..`、空段等任何不在表里的写法）。
pub(crate) fn lookup(path: &str) -> Option<(&'static str, &'static str)> {
    ASSETS
        .iter()
        .find(|(p, _, _)| *p == path)
        .map(|(_, mime, body)| (*mime, *body))
}
