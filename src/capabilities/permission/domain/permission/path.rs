//! 目的：共享区路径条目的写法与命中——归一化、子树匹配、条目与声明层的合法性。
//! 管：`hit` / `normalize` / `validate_entry` / `validate_defaults` / `validate_layer`。
//! 不管：条目由哪一层给出（`layer.rs`）；生效态的求交（`set.rs`）。
//! 联动：`PermissionSet` 的黑名单与范围判定读 `hit` / `normalize`（docs/permission/README.md）。

use super::*;

/// 目的：命中判定：点（或空）代表整棵共享区；其余按路径组件匹配目录子树。
pub fn hit(list: &[String], rel: &str) -> bool {
    let rel = normalize(rel);
    list.iter().any(|entry| {
        let e = normalize(entry);
        if e.is_empty() || e == "." {
            return true;
        }
        rel == e || rel.starts_with(&format!("{}/", e))
    })
}

/// 目的：统一书写形式：去首尾空白与斜杠、反斜杠归一成斜杠。
pub fn normalize(s: &str) -> String {
    s.trim().replace('\\', "/").trim_matches('/').to_string()
}

/// 目的：把一组路径条目写成回执文案（空 = 整棵共享区）。
pub(super) fn listing(entries: &[String]) -> String {
    if entries.is_empty() {
        return "（整棵共享区）".to_string();
    }
    entries
        .iter()
        .map(|e| normalize(e))
        .collect::<Vec<_>>()
        .join("、")
}

/// 目的：两条路径条目是否有交集（一条是另一条的前缀子树或同一条）。
pub(super) fn overlaps(a: &str, b: &str) -> bool {
    let a = normalize(a);
    let b = normalize(b);
    if a.is_empty() || a == "." || b.is_empty() || b == "." {
        return true;
    }
    a == b || a.starts_with(&format!("{}/", b)) || b.starts_with(&format!("{}/", a))
}

/// 目的：两条路径条目里更窄的那一条。
pub(super) fn narrower(a: &str, b: &str) -> String {
    let a = normalize(a);
    let b = normalize(b);
    if a.is_empty() || a == "." {
        return b;
    }
    if b.is_empty() || b == "." {
        return a;
    }
    let ac = a.split('/').count();
    let bc = b.split('/').count();
    if ac >= bc { a } else { b }
}

/// 目的：校验一条共享区相对路径条目：非空、相对、无上级段与空段；点合法（整棵）。
pub fn validate_entry(entry: &str) -> Result<(), String> {
    let raw = entry.trim();
    if raw.is_empty() {
        return Err("路径条目不能为空".to_string());
    }
    let flat = raw.replace('\\', "/");
    if flat.starts_with('/') || flat.starts_with('\\') {
        return Err(format!("路径条目必须是相对共享区根的路径：{}", raw));
    }
    let bytes = flat.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && (bytes[0] as char).is_ascii_alphabetic() {
        return Err(format!("路径条目必须是相对共享区根的路径：{}", raw));
    }
    for seg in flat.split('/') {
        if seg.is_empty() {
            return Err(format!("路径条目含空段（连续分隔符）：{}", raw));
        }
        if seg == ".." {
            return Err(format!("路径条目不能含上级段：{}", raw));
        }
    }
    Ok(())
}

/// 目的：校验全局默认权限。
pub fn validate_defaults(d: &PermissionDefaults) -> Result<(), String> {
    d.shared.read.validate("shared.read")?;
    d.shared.write.validate("shared.write")?;
    for e in &d.shared.deny {
        validate_entry(e).map_err(|why| format!("shared.deny：{}", why))?;
    }
    for name in d.tools.keys() {
        if name.trim().is_empty() {
            return Err("tools 里的工具名不能为空".to_string());
        }
    }
    Ok(())
}

/// 目的：校验一层会话权限的条目形状。
pub fn validate_layer(layer: &PermissionLayer) -> Result<(), String> {
    layer.validate()
}
