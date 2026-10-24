//! 目的：ShareBackend 的内部 `none` 实现——供给编排只经 QGA 在 guest 里装东西，不需要挂载或同步。
//! 管：给引擎一个空共享计划；不建挂载点、不搬文件。
//! 不管：命令通道与 QGA 细节（detail/qemu.rs）、用户共享（share_9p / virtiofs / smb）。
//! 联动：实现 ports::ShareBackend；由组合根的**补齐面**（网页补齐 / `--provision`）构造并显式指定，不参与用户选档。

use crate::capabilities::vm::domain::guest::{GuestImage, GuestMount};
use crate::capabilities::vm::ports::{NoShareGuard, ShareBackend, ShareGuard, SharePlan};

/// 目的：无共享后端——供给编排的显式选择，和“用户共享”无关。
pub struct NoneShare;

impl NoneShare {
    /// 目的：组合根专用。
    pub fn new() -> NoneShare {
        NoneShare
    }
}

impl ShareBackend for NoneShare {
    fn id(&self) -> &'static str {
        "none"
    }

    /// 约束：供给编排只要一个能起 QEMU 的空计划；没有挂载，也就不需要 guest 的共享客户端。
    fn supports(&self, engine: &str, _guest: &GuestImage) -> bool {
        engine == "qemu"
    }

    fn prepare(
        &self,
        _engine: &str,
        mounts: &[GuestMount],
        _share_tool: Option<&str>,
    ) -> Result<(SharePlan, Box<dyn ShareGuard>), String> {
        if !mounts.is_empty() {
            return Err("none 档只给供给编排：不支持挂载".to_string());
        }
        Ok((
            SharePlan {
                kind: "none",
                engine_args: Vec::new(),
                net_options: Vec::new(),
                guest_scripts: Vec::new(),
                needs_net: false,
                needs_shared_mem: false,
            },
            Box::new(NoShareGuard),
        ))
    }
}
