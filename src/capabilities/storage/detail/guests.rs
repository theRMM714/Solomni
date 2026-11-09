//! 目的：guest 存活的真实判定——读进程台账，问进程是否还在。
//! 管：`.home` 的进程台账与 pid 存活；只回答"有没有 guest 在跑"。
//! 不管：guest 怎么起（vm）、怎么停（conductor）、盘点怎么分类（survey）。
//! 联动：service 在回收与还原前经 GuestLiveness 端口问它；survey 判可写层保护时用同一个函数。

use crate::capabilities::storage::ports::GuestLiveness;
use std::path::{Path, PathBuf};

/// 目的：按进程台账判 guest 存活。
pub struct ProcessGuests {
    home: PathBuf,
}

impl ProcessGuests {
    /// 目的：组合根专用——取产品根（台账在 `<root>/.home`）。
    pub fn new(root: &Path) -> ProcessGuests {
        ProcessGuests {
            home: root.join(".home"),
        }
    }
}

impl GuestLiveness for ProcessGuests {
    fn any_running(&self) -> bool {
        live_in(&self.home)
    }
}

/// 目的：这个家目录的进程台账里有没有活着的 guest。
pub fn live_in(home: &Path) -> bool {
    crate::kernel::detail::procs::ProcessBook::open(home)
        .list()
        .iter()
        .any(|e| e.purpose == "guest" && crate::kernel::detail::procs::alive(e.pid))
}
