//! 目的：长驻子进程台账的只读适配——把 kernel 的进程簿翻成本能力要的条目。
//! 管：列条目并逐条问系统"还活着吗"，以及"有没有 guest 在跑"。
//! 不管：起停进程（在 vm / tool 执行）；对账与清理（在各自的编排里）。
//! 联动：组合根构造 `BookProcs` 注入 service；`processes` 与 `reset_guest` 经它看进程事实。

use std::path::{Path, PathBuf};

use crate::capabilities::provision::api::ProcInfo;
use crate::capabilities::provision::ports::Procs;

/// 目的：进程簿的适配器——产品根下的 `.home` 是进程簿的归属。
pub struct BookProcs {
    home: PathBuf,
}

impl BookProcs {
    pub fn new(root: &Path) -> BookProcs {
        BookProcs {
            home: root.join(".home"),
        }
    }

    fn book(&self) -> std::sync::Arc<crate::kernel::detail::procs::ProcessBook> {
        crate::kernel::detail::procs::ProcessBook::open(&self.home)
    }
}

impl Procs for BookProcs {
    fn entries(&self) -> Vec<ProcInfo> {
        self.book()
            .list()
            .into_iter()
            .map(|e| ProcInfo {
                alive: crate::kernel::detail::procs::alive(e.pid),
                pid: e.pid,
                purpose: e.purpose,
                owner: e.owner,
                since: e.since,
                orphan_risk: e.guard_note.is_some(),
                guard_note: e.guard_note,
            })
            .collect()
    }

    fn guest_running(&self) -> bool {
        self.book()
            .list()
            .iter()
            .any(|e| e.purpose == "guest" && crate::kernel::detail::procs::alive(e.pid))
    }
}
