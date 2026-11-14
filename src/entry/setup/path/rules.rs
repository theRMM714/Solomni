//! 目的：PATH 分段规则与可移植的 unset / undo 逻辑——Windows 命令与跨平台单测共用。
//! 管：规范化、去重追加、摘除、备份读写与还原。
//! 不管：注册表读写（端口）、命令入口与平台指引（在门面与平台后端）。
//! 联动：由 `src/entry/setup/path.rs` 在 Windows 或测试构建中选中。

use crate::kernel::ports::{PathValue, UserPath};
use std::path::Path;

use super::backup_path;

/// 目的：备份内容：undo 依据它还原原串与原类型。
/// 约束：v 固定写 1（将来换格式时靠它区分）；at 记 Unix 秒（不引本地时区）。
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct PathBackup {
    pub(crate) v: u8,
    pub(crate) original: String,
    pub(crate) expand: bool,
    pub(crate) bin: String,
    pub(crate) at: u64,
}

/// 目的：一段 PATH 的规范化形态：去引号、去尾部分隔符、大小写不敏感。
/// 约束：只用于比较；写回时原文逐段保留。
pub(crate) fn norm(seg: &str) -> String {
    seg.trim()
        .trim_matches('"')
        .trim_end_matches(['\\', '/'])
        .to_lowercase()
}

/// 目的：PATH 里是否已有我们那段。
pub(crate) fn contains(text: &str, bin: &str) -> bool {
    let want = norm(bin);
    text.split(';').any(|seg| norm(seg) == want)
}

/// 目的：把 bin 追加到 PATH 末尾；已有的重复段先摘掉（幂等：重复执行只保留一段）。
pub(crate) fn append(text: &str, bin: &str) -> String {
    let want = norm(bin);
    let mut parts: Vec<String> = text
        .split(';')
        .filter(|seg| !seg.is_empty() && norm(seg) != want)
        .map(|seg| seg.to_string())
        .collect();
    if text.trim().is_empty() {
        parts.clear();
    }
    parts.push(bin.to_string());
    parts.join(";")
}

/// 目的：把 bin 从 PATH 里摘掉（所有位置）。返回：新串 + 是否真的摘到了。
pub(crate) fn remove(text: &str, bin: &str) -> (String, bool) {
    let want = norm(bin);
    let mut found = false;
    let parts: Vec<&str> = text
        .split(';')
        .filter(|seg| {
            if norm(seg) == want {
                found = true;
                false
            } else {
                !seg.is_empty()
            }
        })
        .collect();
    (parts.join(";"), found)
}

/// 目的：读备份；没有或读不懂给 None。
pub(crate) fn read_backup(home: &Path) -> Option<PathBackup> {
    std::fs::read_to_string(backup_path(home))
        .ok()
        .and_then(|t| serde_json::from_str::<PathBackup>(&t).ok())
}

/// 目的：--setup=unset 的核心：只摘我们那段，其它段原样保留；幂等。
pub(crate) fn do_unset(port: &dyn UserPath, root: &Path) -> Result<String, String> {
    let bin = super::bin_text(root);
    let current = port.read()?;
    let (next, found) = remove(&current.text, &bin);
    if !found {
        return Ok(format!("用户 PATH 未含 {}（无需改动）", bin));
    }
    port.write(&PathValue {
        text: next,
        expand: current.expand,
    })?;
    let _ = port.broadcast();
    Ok(format!("已从用户 PATH 摘掉 {}", bin))
}

/// 目的：--setup=undo 的核心：按备份还原原串与原类型（无备份 = 失败并说明，不猜）。
pub(crate) fn do_undo(port: &dyn UserPath, home: &Path) -> Result<String, String> {
    let Some(backup) = read_backup(home) else {
        return Err(
            "没有 PATH 备份（.home/setup-path.json），无法还原；可先跑 --setup=unset 摘掉那一段"
                .to_string(),
        );
    };
    port.write(&PathValue {
        text: backup.original.clone(),
        expand: backup.expand,
    })?;
    let _ = port.broadcast();
    let _ = std::fs::remove_file(backup_path(home));
    Ok("已按备份还原用户 PATH（原串与原类型）".to_string())
}
