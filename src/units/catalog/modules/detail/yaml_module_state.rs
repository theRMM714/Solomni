//! 目的：启停覆盖的解码与编码（`data/.home/modules.yaml`）。
//! 管：一份 `EnabledState` 的文件形态与读写；文件不存在 = 空；文件非法 = 如实报错。
//! 不管：文件机制与权限（在 `home` 单元）；有效值判定（在 `service.rs`）；模块清单与契约（在 `domain/module.rs`）。
//! 联动：实现 ports::ModuleStateStore；由组合根装配（与登记处共用一份 `Home`）。

use crate::units::catalog::modules::domain::state::EnabledState;
use crate::units::catalog::modules::ports::ModuleStateStore;
use crate::units::home::api::{Doc, Home};
use std::sync::Arc;

/// 目的：启停覆盖的文件存储（组合根构造）。
pub struct YamlModuleState {
    home: Arc<dyn Home>,
}

impl YamlModuleState {
    /// 目的：用登记处读写面造一个存储（组合根给 `data/.home`）。
    pub fn new(home: Arc<dyn Home>) -> YamlModuleState {
        YamlModuleState { home }
    }
}

impl ModuleStateStore for YamlModuleState {
    fn load(&self) -> Result<EnabledState, String> {
        match self.home.read(Doc::Modules)? {
            Some(t) => yaml_serde::from_str::<EnabledState>(&t)
                .map_err(|e| format!("modules.yaml 非法：{}", e)),
            None => Ok(EnabledState::default()),
        }
    }

    fn save(&self, state: &EnabledState) -> Result<(), String> {
        let text = yaml_serde::to_string(state).map_err(|e| e.to_string())?;
        self.home.write(Doc::Modules, &text)
    }
}
