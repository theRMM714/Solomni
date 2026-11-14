//! 目的：`prep` 的 Windows 后端——围栏授权准备、机器级标定与按次回收。
//! 管：把 `confine/windows/` 的实现转交给门面；标定报告的跨层形状。
//! 不管：台账条目形状（在 `ledger.rs`）；非 Windows 的降级（在 `prep.rs` 的共享函数）。
//! 联动：由 `src/kernel/detail/confine/prep.rs` 按平台选中。

use super::FencePrep;
use crate::kernel::api::FenceSpec;
use crate::kernel::detail::confine::windows as backend;

/// 目的：把围栏要用的授权一次性做好（写目录 ACL）。
pub fn prepare_fence(spec: &FenceSpec, command: &str, home: &std::path::Path) -> FencePrep {
    backend::prepare_fence(spec, command, home)
}

/// 目的：`--setup=fence-calibrate` 的合成树标定结果。
pub(crate) struct CalibrationReport {
    pub(crate) path: std::path::PathBuf,
    pub(crate) objects: u64,
    pub(crate) ms: u64,
}

/// 目的：建树、真写一次机器级只读 ACL、计时、撤权。
pub(crate) fn calibrate_machine_grant(home: &std::path::Path) -> Result<CalibrationReport, String> {
    let c = backend::calibrate_machine_grant(home)?;
    Ok(CalibrationReport {
        path: c.path,
        objects: c.objects,
        ms: c.ms,
    })
}

/// 目的：删掉标定用的合成树。
pub(crate) fn remove_calibration_tree(path: &std::path::Path) -> Result<(), String> {
    backend::remove_calibration_tree(path)
}

/// 目的：默认回收：按台账撤掉按次 / 会话权限项与容器 profile，保留机器级授权（仅测试用）。
#[cfg(test)]
pub fn clean(home: &std::path::Path) -> Result<String, String> {
    backend::clean(home)
}
