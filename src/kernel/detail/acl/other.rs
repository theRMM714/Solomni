//! 目的：`acl` 的非 Windows 后端——没有本机权限项可改。
//! 管：解释器目录基线判据的放行结论（没有 ACL 要写，恒可用）。
//! 不管：任何 DACL 读写（只有 Windows 有，在 `acl/windows.rs`）。
//! 联动：`detail.rs` 按平台选中本后端；入口选工具链时跨平台问 `interpreter_dir_usable`。

use std::path::Path;

/// 目的：Windows 之外没有 ACL 这一步，解释器目录只读放行（目录是否存在由调用方自己判）。
///   存在的理由：入口在选工具链时跨平台问这一句，不能因为平台不同就少一条判据。
pub(crate) fn interpreter_dir_usable(_dir: &Path) -> Result<(), String> {
    Ok(())
}
