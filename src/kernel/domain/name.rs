//! 目的：会成为目录名的名字的两条纯规则——校验与重名后缀。
//! 管：`validate_dir_name`（非空、无首尾空白、不是 `.` / `..`、不含 Windows 非法字符与控制字符、不是设备保留名）与 `dedup_suffix`（占用时依次追加 `-2` / `-3`）。
//! 不管：名字从哪来、查重查的是哪个集合（`exists` 由调用方给）、报错怎么呈现。
//! 联动：agent 名（`src/units/catalog/registry/domain/agents.rs`）与工作名（`src/units/work/domain.rs`、`src/units/conversation/session/domain/session.rs`）经 `kernel::api` 共用这两条。

/// 目的：校验一个即将成为目录名的名字。
/// 参数：`what` 只进报错文案（如「agent 名」「工作名」），不参与判断。
/// 约束：规则取三平台最严的一份（Windows 的非法字符与设备保留名），落盘到别的平台也不会带出非法名字。
pub fn validate_dir_name(name: &str, what: &str) -> Result<(), String> {
    let n = name.trim();
    if n.is_empty() {
        return Err(format!("{}不能为空", what));
    }
    if name != n {
        return Err(format!("{}首尾不能有空白", what));
    }
    if n == "." || n == ".." {
        return Err(format!("{}非法", what));
    }
    const BAD: [char; 9] = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    if n.chars().any(|c| BAD.contains(&c) || c.is_control()) {
        return Err(format!("{}不能包含 \\ / : * ? \" < > | 或控制字符", what));
    }
    let upper = n.to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&upper.as_str())
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit());
    if reserved {
        return Err(format!("{}是系统保留名", what));
    }
    Ok(())
}

/// 目的：重名去重：`base` 已被占用就依次试 `base-2`、`base-3`，返回第一个空位。
/// 参数：`exists` 由调用方给——被查的是哪个集合（同一工作内的名字、进程内的名字）归调用方决定。
/// 约束：`base` 由调用方先行解析（去空白、空则取缺省）；本函数只做后缀探测。
pub fn dedup_suffix(base: &str, exists: impl Fn(&str) -> bool) -> String {
    if !exists(base) {
        return base.to_string();
    }
    let mut n = 2;
    loop {
        let cand = format!("{}-{}", base, n);
        if !exists(&cand) {
            return cand;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 非法字符、首尾空白、设备保留名各自被拦，正常名字放行。
    #[test]
    fn validation_rejects_bad_dir_names_and_names_the_subject() {
        for bad in [
            "", " a", "a ", ".", "..", "a/b", "a\\b", "con", "LPT1", "a:b",
        ] {
            let err = validate_dir_name(bad, "工作名").expect_err("应当拒绝");
            assert!(err.starts_with("工作名"), "报错带主语：{}", err);
        }
        assert!(validate_dir_name("取名 一", "工作名").is_ok());
        assert!(validate_dir_name("CONSOLE", "工作名").is_ok(), "不是保留名");
    }

    /// 后缀探测：空位原样返回，占用时跳过所有已占的尾号。
    #[test]
    fn suffix_probe_skips_every_taken_name() {
        let mut taken: Vec<String> = Vec::new();
        let probe =
            |taken: &[String], base: &str| dedup_suffix(base, |c| taken.iter().any(|t| t == c));
        assert_eq!(probe(&taken, "w"), "w");
        taken.push("w".to_string());
        assert_eq!(probe(&taken, "w"), "w-2");
        taken.push("w-2".to_string());
        assert_eq!(probe(&taken, "w"), "w-3");
    }
}
