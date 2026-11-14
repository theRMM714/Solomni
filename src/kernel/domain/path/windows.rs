//! 目的：`path` 的 Windows 后端——还原 `\\?\` 扩展前缀。
//! 管：verbatim UNC / verbatim 盘符前缀的普通形态还原；卷 GUID 与设备路径保持原样。
//! 不管：UNC 判定与拆分（在门面，两平台共用）；对外 `/` 书写。
//! 联动：由 `src/kernel/domain/path.rs` 按平台选中。

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf, Prefix};

/// 扩展前缀 `\\?\`
const VERBATIM: [u16; 4] = [92, 92, 63, 92];
/// 扩展前缀下的 UNC 形态 `\\?\UNC\`
const VERBATIM_UNC: [u16; 8] = [92, 92, 63, 92, 85, 78, 67, 92];

/// 目的：把宿主真实路径还原成用户能认的写法——去掉 `\\?\` 扩展前缀。
pub(crate) fn native(p: &Path) -> PathBuf {
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
    // UNC 把 `\\?\UNC\` 换成 `\\`（少 6 个字符，补两个反斜杠）；盘符只是去掉 `\\?\`。
    let mut out: Vec<u16> = Vec::with_capacity(wide.len());
    if unc {
        out.extend_from_slice(&[92, 92]);
    }
    out.extend_from_slice(&wide[head.len()..]);
    PathBuf::from(OsString::from_wide(&out))
}
