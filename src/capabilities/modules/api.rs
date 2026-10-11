//! 目的：模块能力的入站面——契约类型与只读/用例两个 trait。
//! 管：`ModulesOps`（只读清单事实）与 `Modules`（用例）两个 trait，以及模块契约类型的重导出。
//! 不管：清单来源机制与端口（在 `ports.rs` / `detail/`）；消费方怎么使用这些事实。
//! 联动：实现见 `service.rs`；其它能力与呈现层只经这里引用模块事实（R1）。

pub use crate::capabilities::modules::domain::module::{
    check_runtimes, check_secrets, check_services, check_tools, listing, Module, ModuleManifest,
    Param, ParamType, Roster, SecretDecl, ServiceDecl, ToolDecl,
};

/// 目的：模块公地的只读面（呈现层与消费方）——清单事实。
pub trait ModulesOps: Send + Sync {
    /// 目的：模块公地的清单事实（合法模块 + 拒收原因）。
    fn roster(&self) -> Result<Roster, String>;
}

/// 目的：模块能力的用例面（`service.rs` 实现）——别的能力要清单事实走这里。
pub trait Modules: Send + Sync {
    /// 目的：模块公地：合法模块 + 拒收原因（清单即事实，每次重扫）。
    fn roster(&self) -> Roster;
}
