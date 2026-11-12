//! 目的：registry 能力的出站端口——四份 yaml 的持久化。
//! 管：SettingsStore trait 的定义（读 / 写 Settings）。
//! 不管：yaml 解析与文件机制（在 detail/yaml_settings.rs）；状态与用例（在 service.rs）。
//! 联动：只由 service.rs 使用；实现见 detail/yaml_settings.rs。

use crate::capabilities::registry::domain::providers::Settings;

/// 目的：登记处持久化端口：供应商与模型分开保存（机制/文件名在适配层）。
pub trait SettingsStore {
    fn load(&self) -> Result<Settings, String>;
    fn save(&self, settings: &Settings) -> Result<(), String>;
}
