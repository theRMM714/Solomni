//! 目的：模块能力的入站面——契约类型与只读/用例两个 trait。
//! 管：`ModulesOps`（只读清单事实与启停有效值）与 `Modules`（用例）两个 trait，以及契约类型的重导出。
//! 不管：清单来源机制与端口（在 `ports.rs` / `detail/`）；消费方怎么使用这些事实。
//! 联动：实现见 `service.rs`；其它能力与呈现层只经这里引用模块事实（R1）；启停语义见 docs/modules/README.md。

pub use crate::capabilities::modules::domain::module::{
    check_build, check_runtimes, check_secrets, check_services, check_tools, listing, local_entry,
    BuildDecl, Module, ModuleManifest, Param, ParamType, Roster, SecretDecl, ServiceDecl, ToolDecl,
};

/// 目的：模块公地的只读面（呈现层与消费方）——清单事实与启停有效值。
pub trait ModulesOps: Send + Sync {
    /// 目的：模块公地的清单事实（合法模块 + 拒收原因）。
    fn roster(&self) -> Result<Roster, String>;
    /// 目的：模块的**有效**启用（用户覆盖 ?? 作者默认；不存在 = false）。
    fn module_enabled(&self, id: &str) -> Result<bool, String>;
    /// 目的：服务的**有效**启用（模块停用 ⇒ false）。
    fn service_enabled(&self, module: &str, service: &str) -> Result<bool, String>;
}

/// 目的：模块能力的用例面（`service.rs` 实现）——别的能力要清单事实或启停写面，走这里。
pub trait Modules: Send + Sync {
    /// 目的：模块公地：合法模块 + 拒收原因（清单即事实，每次重扫）。
    fn roster(&self) -> Roster;
    /// 目的：模块的**有效**启用（用户覆盖 ?? 作者默认；不存在 = false）。
    fn module_enabled(&self, id: &str) -> bool;
    /// 目的：服务的**有效**启用（模块停用 ⇒ false；覆盖 ?? 作者默认）。
    fn service_enabled(&self, module: &str, service: &str) -> bool;
    /// 目的：写入模块启用覆盖（落盘）；模块不存在 = 报错。
    fn set_module_enabled(&self, id: &str, on: bool) -> Result<(), String>;
    /// 目的：写入服务启用覆盖（落盘）；模块或服务不存在 = 报错。
    fn set_service_enabled(&self, module: &str, service: &str, on: bool) -> Result<(), String>;
}
