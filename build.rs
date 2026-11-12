//! 目的：编译期把 `src/presentation/web/assets/` 生成为浏览器端资源的嵌入表。
//! 管：递归扫描资源目录；按扩展名定 MIME；生成 `OUT_DIR/web_assets.rs`（按 URL 路径排序的 `(路径, MIME, 内容)` 表）。
//! 不管：HTTP 分发与缓存头（`src/presentation/web/assets.rs`、`src/presentation/web.rs`）；
//!   node 侧开发文件（`.cjs` / `.mjs` 冒烟脚本与 `package.json` 只给 node 用，不进发行版）。
//! 联动：`src/presentation/web/assets.rs` 用 `include!` 引入生成表；路由 `GET /assets/{*path}` 见
//!   `src/presentation/web/routes.rs`；未知扩展名直接让构建失败，防止资源被静默漏掉。

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// 进发行版的扩展名与 MIME。
const SERVED: &[(&str, &str)] = &[
    ("html", "text/html"),
    ("css", "text/css"),
    ("js", "application/javascript"),
    ("svg", "image/svg+xml"),
];

/// 只给 node 用的开发脚本：留在资源目录里，但不嵌入。
const DEV_ONLY: &[&str] = &["cjs", "mjs"];

/// 只给 node 用的文件名（`package.json` 声明 `app/` 下的 `.js` 是 ES Module）：不嵌入。
const DEV_ONLY_NAMES: &[&str] = &["package.json"];

/// 只在 debug 构建嵌入的目录：组件陈列页是开发工具，发行版不带。
const DEV_ONLY_DIRS: &[&str] = &["dev"];

fn main() {
    let root =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo 必给 CARGO_MANIFEST_DIR"));
    let assets = root
        .join("src")
        .join("presentation")
        .join("web")
        .join("assets");
    println!("cargo:rerun-if-changed={}", assets.display());
    // PROFILE 变化要重跑：同一份 OUT_DIR 可能先 debug 后 release。
    println!("cargo:rerun-if-env-changed=PROFILE");
    let debug = std::env::var("PROFILE")
        .map(|p| p == "debug")
        .unwrap_or(false);

    let mut entries: Vec<(String, &'static str, PathBuf)> = Vec::new();
    collect(&assets, &assets, debug, &mut entries);
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = String::from("&[\n");
    for (url, mime, file) in &entries {
        // 路径用 Debug 输出：反斜杠与引号都按 Rust 字面量转义，Windows 路径同样成立。
        writeln!(
            out,
            "    ({:?}, {:?}, include_str!({:?})),",
            url,
            mime,
            file.display().to_string()
        )
        .expect("写 String 不会失败");
    }
    out.push(']');

    let dest =
        PathBuf::from(std::env::var("OUT_DIR").expect("cargo 必给 OUT_DIR")).join("web_assets.rs");
    fs::write(&dest, out).expect("写生成表失败");
}

fn collect(base: &Path, dir: &Path, debug: bool, out: &mut Vec<(String, &'static str, PathBuf)>) {
    let mut items: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("读不了资源目录 {}：{}", dir.display(), e))
        .map(|e| e.expect("可读的目录项").path())
        .collect();
    items.sort();
    for path in items {
        if path.is_dir() {
            let dev = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| DEV_ONLY_DIRS.contains(&n))
                .unwrap_or(false);
            if dev && !debug {
                continue;
            }
            collect(base, &path, debug, out);
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if DEV_ONLY.contains(&ext) || DEV_ONLY_NAMES.contains(&name) {
            continue;
        }
        let Some((_, mime)) = SERVED.iter().find(|(e, _)| *e == ext) else {
            panic!(
                "资源目录里有不认识的文件 {}：要发行就把扩展名加进 build.rs 的 SERVED，开发脚本用 .cjs / .mjs",
                path.display()
            );
        };
        // URL 路径段用 '/' 连接（URL 语义，与宿主路径分隔符无关）。
        let url = path
            .strip_prefix(base)
            .expect("子路径必在资源目录下")
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        out.push((url, mime, path));
    }
}
