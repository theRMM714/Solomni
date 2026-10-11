//! 目的：模块与服务的启停**覆盖值**（用户决定）与文件形态。
//! 管：`EnabledState` / `ModuleToggle` 的 serde 形态。
//! 不管：有效值判定（用户覆盖 ?? 作者默认，在 `service.rs`）；落盘（在 `detail/yaml_module_state.rs`）。
//! 联动：由 `service.rs` 持有一份；契约与启停语义见 docs/modules/README.md 与 MODULE_SPEC.md。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 目的：一份启停覆盖（`.home/modules.yaml` 的形态）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct EnabledState {
    /// 目的：模块 id → 该模块的启停覆盖。
    #[serde(default)]
    pub modules: BTreeMap<String, ModuleToggle>,
}

/// 目的：一个模块的启停覆盖；缺项 = 用 `module.yaml` 的作者默认。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ModuleToggle {
    /// 目的：模块整体是否启用；`None` = 没覆盖（用作者默认，缺省 true）。
    #[serde(default)]
    pub enabled: Option<bool>,
    /// 目的：服务名 → 启用覆盖；缺项 = 用 `services.<名>.enabled` 的作者默认。
    #[serde(default)]
    pub services: BTreeMap<String, bool>,
}
