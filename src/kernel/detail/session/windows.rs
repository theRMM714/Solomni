//! 目的：`session` 的 Windows 后端——常驻服务拉起前的容器围栏准备。
//! 管：写 ACL 授权、未授权只提示一次、机制自检失败时 fail-closed。
//! 不管：管道、按行收发与进程账（在门面）；围栏策略。
//! 联动：由 `src/kernel/detail/session.rs` 按平台选中；围栏机制在 `confine`。

use super::*;
use crate::kernel::ports::ProcessTexts;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// 目的：Windows 侧需要随会话宿主一起持有的状态。
pub(super) struct State {
    texts: Arc<dyn ProcessTexts>,
    home: PathBuf,
    write_allowed: Arc<AtomicBool>,
    disclosed: Arc<AtomicBool>,
}

impl State {
    /// 目的：保存组合根注入的文案、产品私有区与写权限开关。
    pub(super) fn new(
        texts: Arc<dyn ProcessTexts>,
        home: PathBuf,
        write_allowed: Arc<AtomicBool>,
    ) -> State {
        State {
            texts,
            home,
            write_allowed,
            disclosed: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// 目的：拉起常驻服务前先准备容器围栏；必要落点授不上时 fail-closed。
pub(super) fn prepare(
    state: &State,
    spec: &SessionSpec,
) -> Result<(bool, Option<PathBuf>), String> {
    let mut prepared = false;
    if state.write_allowed.load(Ordering::Relaxed) {
        let prep =
            crate::kernel::detail::confine::prepare_fence(&spec.fence, &spec.command, &state.home);
        // 可选落点授不上只记事实（不牵动这次拉起）。
        for n in &prep.notes {
            eprintln!("[围栏] {}（可选落点，本次照常执行）", n);
        }
        if prep.ok() {
            prepared = true;
        } else if let Some(blocked) = &prep.blocked {
            return Err(format!(
                "[围栏] 服务起不来（必要落点授不上）：{}",
                blocked.line()
            ));
        }
    } else if !state.disclosed.swap(true, Ordering::Relaxed) {
        eprintln!(
            "[围栏] 容器围栏未启用（用户没授权在本机写权限）：常驻服务按无围栏执行（要启用：在设置里打开，或写 sandbox.policy.windows.write_acl: true）"
        );
    }
    if !prepared
        && let Some(outcome) = crate::kernel::detail::process::refuse_when_broken(
            &*state.texts,
            crate::kernel::detail::confine::verify(&spec.fence, &spec.command),
        )
    {
        return Err(outcome.output);
    }
    Ok((prepared, Some(state.home.clone())))
}
