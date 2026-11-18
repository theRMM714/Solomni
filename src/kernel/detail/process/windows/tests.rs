//! 目的：Windows 进程后端的测试：解释器解析的三态（真身 / 应用执行别名 / 缺失）与别名判据。
//! 管：只覆盖本后端的纯决策与判据；起真进程的契约在 `src/kernel/detail/process/tests.rs`。
//! 不管：其它平台后端、上层装配。
//! 联动：与 `src/kernel/detail/process/windows.rs` 同模块，`#[cfg(test)] mod tests;` 指到这里。

use super::{ExeFact, looks_like_app_exec_alias, pick_powershell};

/// 目的：只有真身才选 pwsh 7；应用执行别名与缺失都回退 Windows PowerShell 5.1。
#[test]
fn only_a_real_pwsh_is_chosen_and_alias_or_missing_falls_back() {
    assert_eq!(pick_powershell(None, ExeFact::Real), "pwsh");
    assert_eq!(pick_powershell(None, ExeFact::AppAlias), "powershell");
    assert_eq!(pick_powershell(None, ExeFact::Missing), "powershell");
}

/// 目的：显式覆盖压过一切（真机验 5.1 回退与测试夹具都走这条）。
#[test]
fn explicit_override_wins_over_the_fact() {
    assert_eq!(
        pick_powershell(Some("powershell"), ExeFact::Real),
        "powershell"
    );
    assert_eq!(pick_powershell(Some("pwsh"), ExeFact::Missing), "pwsh");
}

/// 目的：别名判据的三态：普通文件不是别名、Store 版 0 字节重解析点是别名、符号链接按真身用。
#[test]
fn app_exec_alias_needs_a_reparse_point_without_a_target_and_no_length() {
    /// 目的：`FILE_ATTRIBUTE_REPARSE_POINT`。
    const REPARSE: u32 = 0x0000_0400;
    /// 目的：`FILE_ATTRIBUTE_ARCHIVE`（Store 版别名的实际属性组合）。
    const ARCHIVE: u32 = 0x0000_0020;
    assert!(
        !looks_like_app_exec_alias(ARCHIVE, false, 300 * 1024),
        "本机安装的 pwsh 7 是普通文件"
    );
    assert!(
        looks_like_app_exec_alias(REPARSE | ARCHIVE, false, 0),
        "Store 版 pwsh 是 0 字节重解析点"
    );
    assert!(
        !looks_like_app_exec_alias(REPARSE, true, 0),
        "符号链接指向真身，不算别名"
    );
    assert!(
        !looks_like_app_exec_alias(ARCHIVE, false, 0),
        "0 字节但不是重解析点（建坏的文件）不是别名"
    );
}
