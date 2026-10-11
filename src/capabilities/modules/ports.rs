//! 目的：模块能力的出站端口。
//! 管：`ModuleSource`（清单来源）与 `ModuleStateStore`（启停覆盖落盘）。
//! 不管：目录遍历与解析机制（在 `detail/`）；有效值判定（在 `service.rs`）。
//! 联动：都只由 `service.rs` 持有（R12）；实现见 `detail/`。
use crate::capabilities::modules::domain::module::Roster;

/// 目的：模块清单来源端口。**只有 `service.rs` 持有它**（R12）。
pub trait ModuleSource {
    /// 目的：重扫目录，给出合法模块与拒收原因。
    fn scan(&self) -> Roster;
}

/// 目的：启停覆盖的落盘端口（`.home/modules.yaml`）。**只有 `service.rs` 持有它**（R12）。
pub trait ModuleStateStore: Send + Sync {
    /// 目的：读一份覆盖；文件不存在 = 空。
    fn load(&self) -> Result<crate::capabilities::modules::domain::state::EnabledState, String>;
    /// 目的：写一份覆盖。
    fn save(
        &self,
        state: &crate::capabilities::modules::domain::state::EnabledState,
    ) -> Result<(), String>;
}
