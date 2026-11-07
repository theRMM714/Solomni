//! 目的：路径的对外书写形式——一律用 /；以及 Windows 扩展前缀的还原与 UNC 形态识别（纯逻辑）。
//! 管：把路径拼成跨平台可读的字符串（提示词、回执与 API 用）；\\?\ 前缀还原成用户能认的写法；判定与拆分 UNC。
//! 不管：真实文件系统访问（在 kernel::detail）；把用户输入解析回路径。
//! 联动：各处拼对外路径时调用 slash；native / is_unc / unc_parts 由绑定校验、围栏与路径桥共用（与 R9 同源）。

use std::ffi::OsString;
use std::path::{Component, PathBuf, Prefix};

/// 目的：把路径写成对外形式——一律用 / 分隔（给模型看、进提示词与 JSON）。
/// 约束：Windows 的反斜杠在 JSON 字符串里是转义符（"D:\a" 会解析失败），所以对外只给 /——两边系统都认。
pub fn slash(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// 目的：同一物理落点的身份——锁键与「是否同一目录」都按它算，不按路径字符串。
/// 约束：Windows 是（卷序列号，文件 id）；Unix 是（设备号，inode）。取不到身份时调用方按「不知道」处理，
///   既不当成不同，也不当成相同。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Locus {
    pub volume: u64,
    pub id: u128,
}

impl Locus {
    /// 目的：稳定的锁键——同一落点给同一个键（十六进制，定长）。
    pub fn key(&self) -> String {
        format!("{:016x}-{:032x}", self.volume, self.id)
    }
}

/// 目的：把宿主真实路径还原成用户能认的写法——Windows 去掉 \\?\ 扩展前缀。
/// 约束：\\?\UNC\server\share\… 还原成 \\server\share\…（不是相对路径 UNC\server\…）；
///   \\?\C:\… 还原成 C:\…；\\?\Volume{…} 没有普通形态，原样保留，免得把不可还原的写法弄成错路径。
///   非 Windows 原样返回。
pub fn native(p: &std::path::Path) -> std::path::PathBuf {
    native_impl(p)
}

/// 目的：判定路径是不是 UNC（\\server\share 或 //server/share，含 verbatim 写法）。
/// 约束：只有 Windows 的路径才有 Prefix 段，非 Windows 恒为 false——那里的 //x 是普通绝对路径。
pub fn is_unc(p: &std::path::Path) -> bool {
    unc_prefix(p).is_some()
}

/// 目的：拆出 UNC 的（服务器，共享名，共享内的相对路径）。
/// 返回：不是 UNC 给 None；共享根本身的相对路径为空。
pub fn unc_parts(p: &std::path::Path) -> Option<(OsString, OsString, PathBuf)> {
    let (server, share) = unc_prefix(p)?;
    let mut rest = PathBuf::new();
    for c in p.components().skip(2) {
        match c {
            Component::Normal(s) => rest.push(s),
            Component::ParentDir => rest.push(".."),
            Component::RootDir | Component::CurDir => {}
            Component::Prefix(_) => return None,
        }
    }
    Some((server, share, rest))
}

/// 目的：取 UNC 前缀的（服务器，共享名）；不是 UNC 给 None。
fn unc_prefix(p: &std::path::Path) -> Option<(OsString, OsString)> {
    let Component::Prefix(prefix) = p.components().next()? else {
        return None;
    };
    match prefix.kind() {
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
            Some((server.to_os_string(), share.to_os_string()))
        }
        _ => None,
    }
}

#[cfg(windows)]
fn native_impl(p: &std::path::Path) -> PathBuf {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};

    /// 扩展前缀 \\?\
    const VERBATIM: [u16; 4] = [92, 92, 63, 92];
    /// 扩展前缀下的 UNC 形态 \\?\UNC\
    const VERBATIM_UNC: [u16; 8] = [92, 92, 63, 92, 85, 78, 67, 92];

    let Some(Component::Prefix(prefix)) = p.components().next() else {
        return p.to_path_buf();
    };
    // 只有这两种 verbatim 前缀有普通形态；卷 GUID 与设备路径保持原样。
    let unc = match prefix.kind() {
        Prefix::VerbatimUNC(_, _) => true,
        Prefix::VerbatimDisk(_) => false,
        _ => return p.to_path_buf(),
    };
    let head: &[u16] = if unc { &VERBATIM_UNC } else { &VERBATIM };
    let wide: Vec<u16> = p.as_os_str().encode_wide().collect();
    if wide.len() < head.len() || wide[..head.len()] != *head {
        return p.to_path_buf();
    }
    // UNC 把 \\?\UNC\ 换成 \\（少 6 个字符，补两个反斜杠）；盘符只是去掉 \\?\。
    let mut out: Vec<u16> = Vec::with_capacity(wide.len());
    if unc {
        out.extend_from_slice(&[92, 92]);
    }
    out.extend_from_slice(&wide[head.len()..]);
    PathBuf::from(OsString::from_wide(&out))
}

#[cfg(not(windows))]
fn native_impl(p: &std::path::Path) -> PathBuf {
    p.to_path_buf()
}
