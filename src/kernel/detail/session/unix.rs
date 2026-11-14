//! 目的：`session` 的非 Windows 后端——没有容器围栏准备。
//! 管：保存构造参数并如实返回「未准备、无家目录」。
//! 不管：管道、按行收发与进程账（在门面）；围栏策略。
//! 联动：由 `src/kernel/detail/session.rs` 在非 Windows 目标选中。

use super::*;
use crate::kernel::ports::ProcessTexts;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// 目的：非 Windows 没有需要随宿主一起保存的平台状态。
pub(super) struct State;

impl State {
    /// 目的：非 Windows 不保存文案 / 私有区 / 写权限开关。
    pub(super) fn new(
        _texts: Arc<dyn ProcessTexts>,
        _home: PathBuf,
        _write_allowed: Arc<AtomicBool>,
    ) -> State {
        State
    }
}

/// 目的：非 Windows 没有容器围栏准备，直接放行。
pub(super) fn prepare(
    _state: &State,
    _spec: &SessionSpec,
) -> Result<(bool, Option<PathBuf>), String> {
    Ok((false, None))
}
