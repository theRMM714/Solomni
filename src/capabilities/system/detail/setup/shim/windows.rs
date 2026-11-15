//! 目的：便携启动的 shim——`<root>/bin/solomni.cmd`，从任意 cwd 都把产品根固定成安装根。
//! 管：shim 内容渲染、按需创建/刷新（内容变了才写）、路径与新鲜度判定。
//! 不管：PATH 怎么改（path.rs）、围栏授权（fence.rs）。
//! 联动：path.rs 在改 PATH 前先调用 ensure，保证 PATH 指向的入口存在。

use std::path::{Path, PathBuf};

/// 目的：shim 的落点：产品根的 bin 目录（与 .tools 等产品内目录同级）。
pub(crate) fn shim_path(root: &Path) -> PathBuf {
    root.join("bin").join("solomni.cmd")
}

/// 目的：shim 内容：调用本程序并固定 `--root`，其余参数原样透传（`%*`）。
/// 约束：用引号包住 exe 与根（路径可能含空格）；行尾用 CRLF（cmd 在旧版解析下更稳）。
pub(crate) fn render(exe: &Path, root: &Path) -> String {
    format!(
        "@echo off\r\n\"{}\" --root \"{}\" %*\r\n",
        exe.display(),
        root.display()
    )
}

/// 目的：本程序的 exe 路径；取不到时退回产品根下的 solomni.exe（shim 仍可解释执行）。
pub(crate) fn exe_path(root: &Path) -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| root.join("solomni.exe"))
}

/// 目的：确保 shim 存在且内容是最新的。
/// 返回：shim 路径 + 是否真的改了文件（false = 本来就最新）。
pub(crate) fn ensure(root: &Path) -> Result<(PathBuf, bool), String> {
    let path = shim_path(root);
    let want = render(&exe_path(root), root);
    let current = std::fs::read_to_string(&path).ok();
    if current.as_deref() == Some(want.as_str()) {
        return Ok((path, false));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("建 bin 目录失败（{}）：{}", dir.display(), e))?;
    }
    std::fs::write(&path, &want)
        .map_err(|e| format!("写 shim 失败（{}）：{}", path.display(), e))?;
    Ok((path, true))
}

/// 目的：shim 现状：在不在、内容是否就是当前该有的样子（根搬过或 exe 变了就是 stale）。
pub(crate) fn inspect(root: &Path) -> (bool, bool) {
    let path = shim_path(root);
    match std::fs::read_to_string(&path) {
        Ok(text) => (true, text == render(&exe_path(root), root)),
        Err(_) => (false, false),
    }
}
