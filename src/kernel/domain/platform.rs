//! 目的：平台事实——编译期确定的常量与纯判断，供各机制共用。
//! 管：可执行文件后缀、PATH 列表分隔符、宿主路径的大小写语义、Windows 特有的路径与授权事实。
//! 不管：任何 IO；任何平台机制实现（各机制的 `<os>.rs` 后端负责）；业务策略。
//! 联动：由 `src/kernel/api.rs` 重导出；机制后端与各能力的 detail 按需引用。

use std::path::{Component, Path, Prefix};

/// 目的：可执行文件的后缀（Windows 为 `.exe`，其余平台为空）。
pub const EXE_SUFFIX: &str = if cfg!(windows) { ".exe" } else { "" };

/// 目的：PATH 类环境变量的列表分隔符。
pub const PATH_LIST_SEPARATOR: char = if cfg!(windows) { ';' } else { ':' };

/// 目的：宿主路径比较是否忽略 ASCII 大小写（Windows 盘符与路径大小写不敏感）。
pub const HOST_PATH_CASE_INSENSITIVE: bool = cfg!(windows);

/// 目的：本平台是否有“外层先把可达范围写进 ACL”的授权步骤（当前只有 Windows 有）。
pub const HAS_EXTERNAL_ACL_STEP: bool = cfg!(windows);

/// 目的：当前目标是不是 Windows。
pub const IS_WINDOWS: bool = cfg!(windows);

/// 目的：当前目标是不是 Linux。
pub const IS_LINUX: bool = cfg!(target_os = "linux");

/// 目的：当前目标是不是 macOS。
pub const IS_MACOS: bool = cfg!(target_os = "macos");

/// 目的：当前目标的 CPU 架构（与 `std::env::consts::ARCH` 同源）。
pub const HOST_ARCH: &str = std::env::consts::ARCH;

/// 目的：判断两条宿主路径是不是同一个落点。
/// 约束：Windows 忽略 ASCII 大小写；非 UTF-8 在 lossy 比较下会丢字节，两边都是合法 UTF-8 才比，否则给 false。
pub fn host_path_eq(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    if !HOST_PATH_CASE_INSENSITIVE {
        return false;
    }
    match (a.to_str(), b.to_str()) {
        (Some(x), Some(y)) => x.eq_ignore_ascii_case(y),
        _ => false,
    }
}

/// 目的：路径是不是 UNC 网络共享（Windows 前缀；其余平台恒 false）。
/// 约束：按路径前缀判定，不能先 canonicalize（扩展路径 `\\?\C:\` 会被误判成 UNC）。
pub fn is_network_share(path: &Path) -> bool {
    matches!(
        path.components().next(),
        Some(Component::Prefix(p))
            if matches!(p.kind(), Prefix::UNC(..) | Prefix::VerbatimUNC(..))
    )
}

/// 目的：按卷分组时的卷键（Windows 盘符或前缀；其余平台没有这个维度）。
/// 约束：canonicalize 之后的扩展写法（`\\?\D:\`）要归一成 `D:`，否则同一个卷会记成两个键。
pub fn volume_key(path: &Path) -> Option<String> {
    if !cfg!(windows) {
        return None;
    }
    match path.components().next() {
        Some(Component::Prefix(p)) => match p.kind() {
            Prefix::VerbatimDisk(d) => Some(format!("{}:", d as char)),
            _ => Some(p.as_os_str().to_string_lossy().into_owned()),
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_suffix_matches_platform() {
        assert_eq!(cfg!(windows), EXE_SUFFIX == ".exe");
        assert_eq!(cfg!(windows), PATH_LIST_SEPARATOR == ';');
    }

    #[test]
    fn host_path_compare_follows_platform_case_rules() {
        let a = Path::new("Root/Alpha");
        let b = Path::new("Root/Alpha");
        assert!(host_path_eq(a, b), "同一写法必须相等");
        let mixed = Path::new("root/alpha");
        assert_eq!(cfg!(windows), host_path_eq(a, mixed));
    }

    #[test]
    fn volume_key_and_network_share_only_exist_on_windows_prefixes() {
        if cfg!(windows) {
            assert_eq!(volume_key(Path::new("D:/data")), Some("D:".to_string()));
            assert!(is_network_share(Path::new("//server/share")));
        } else {
            assert_eq!(volume_key(Path::new("/data")), None);
            assert!(!is_network_share(Path::new("/data")));
        }
    }
}
