//! 目的：`--setup` 维护旗：便携 PATH（bin shim + 用户 PATH）与围栏的机器级一次性预授。
//! 管：子命令分派（path / unset / undo / status / fence / all）、JSON 状态输出、退出码。
//! 不管：PATH 分段与 shim 内容（path.rs / shim.rs）、ACL 机制（kernel 的 confine::machine_ensure）。
//! 联动：入口 main.rs 在解析出产品根后立刻分派（早于工具链与启动对账，status 的 JSON 才不被别的输出污染）。

pub(crate) mod fence;
pub(crate) mod path;
#[cfg(windows)]
pub(crate) mod shim;

use std::path::Path;

/// 目的：`--setup` 的状态视图（`--setup=status` 打印成 JSON；字段只描述可观察事实）。
#[derive(serde::Serialize)]
pub(crate) struct SetupStatus {
    platform: &'static str,
    root: String,
    path: path::PathInfo,
    fence: FenceInfo,
}

#[derive(serde::Serialize)]
pub(crate) struct FenceInfo {
    supported: bool,
    ready: bool,
    missing: Vec<String>,
    error: Option<String>,
}

/// 目的：分派 `--setup=<mode>`，返回进程退出码（0 成功 / 1 失败 / 2 用法错）。
pub(crate) fn run(root: &Path, mode: &str) -> i32 {
    match mode {
        "path" => path::command(root),
        "unset" => path::unset_command(root),
        "undo" => path::undo_command(root),
        "status" => match status_json(root) {
            Ok(text) => {
                println!("{}", text);
                0
            }
            Err(e) => {
                eprintln!("[便携] 状态读取失败：{}", e);
                1
            }
        },
        "fence" => fence::command(root),
        "fence-calibrate" => fence::calibrate_command(root),
        "all" => {
            let a = path::command(root);
            let b = fence::command(root);
            if a == 0 { b } else { a }
        }
        other => {
            eprintln!(
                "[错误] 未知的 --setup 值「{}」；可用：path / unset / undo / status / fence / fence-calibrate / all（省略值 = status）",
                other
            );
            2
        }
    }
}

/// 目的：拼出状态 JSON：平台、根、PATH/shim 事实、机器级预授缺项。只读，不改任何状态。
pub(crate) fn status_json(root: &Path) -> Result<String, String> {
    let path = path::info(root);
    let fence = match fence::missing(root) {
        Ok(missing) => FenceInfo {
            supported: true,
            ready: missing.is_empty(),
            missing,
            error: None,
        },
        Err(e) => FenceInfo {
            supported: false,
            ready: false,
            missing: Vec::new(),
            error: Some(e),
        },
    };
    let view = SetupStatus {
        platform: std::env::consts::OS,
        root: root.display().to_string(),
        path,
        fence,
    };
    serde_json::to_string_pretty(&view).map_err(|e| e.to_string())
}

/// 目的：启动期只读对账：机器级预授缺项时打一条提示（不静默慢写、不阻断启动）。
/// 返回：true = 已就绪；false = 有缺项（调用方只提示一次）。
#[cfg(windows)]
pub(crate) fn fence_hint(root: &Path) -> bool {
    match fence::missing(root) {
        Ok(missing) if missing.is_empty() => true,
        Ok(missing) => {
            println!(
                "[便携] 围栏机器级授权还缺 {} 处（{}）：跑一次 solomni --setup=fence 可免掉新会话的重复 ACL 写",
                missing.len(),
                missing.join("；")
            );
            false
        }
        Err(e) => {
            eprintln!("[便携] 围栏预授对账失败：{}", e);
            true
        }
    }
}

/// 目的：Windows 之外没有「机器级 ACL」这一步，如实返回已就绪（缺口见 tests/gaps.yaml）。
#[cfg(not(windows))]
pub(crate) fn fence_hint(_root: &Path) -> bool {
    true
}

/// 目的：首次授权写盘后的那一趟：把机器级落点一次授好并显示进度（失败只告警，不拦重启）。
#[cfg(windows)]
pub(crate) fn ensure_after_authorize(root: &Path) {
    let home = root.join(".home");
    println!("[便携] 首次授权：预授围栏的机器级授权（只需一次，之后新会话不再重写这批 ACL）");
    if let Err(e) = fence::ensure(root, &home) {
        eprintln!(
            "[便携] 机器级预授未完成：{}（稍后可用 solomni --setup=fence 重跑）",
            e
        );
    }
}

/// 目的：Windows 之外没有机器级预授，这一趟无事可做（如实收尾，缺口见 tests/gaps.yaml）。
#[cfg(not(windows))]
pub(crate) fn ensure_after_authorize(root: &Path) {
    let _ = fence::ensure(root, &root.join(".home"));
}

#[cfg(test)]
mod tests {
    /// 目的：非 Windows 上 `fence-calibrate` 如实报不可用（不静默成功）。
    #[cfg(not(windows))]
    #[test]
    fn fence_calibrate_reports_unavailable_off_windows() {
        assert_eq!(fence::calibrate_command(Path::new(".")), 1);
    }
}
