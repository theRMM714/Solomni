//! 目的：模块能力的出站端口。
//! 管：`ModuleSource`（模块清单来源）。
//! 不管：目录遍历与解析机制（在 `detail/fs_modules.rs`）；清单排序（在 `domain/module.rs`）。
//! 联动：只由 `service.rs` 持有（R12）；实现见 `detail/fs_modules.rs`。
use crate::capabilities::modules::domain::module::Roster;

/// 目的：模块清单来源端口。**只有 `service.rs` 持有它**（R12）。
pub trait ModuleSource {
    fn scan(&self) -> Roster;
}
