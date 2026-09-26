//! 登记处的**出站端口**：四份 yaml 的持久化（机制在适配层）。

use crate::capabilities::registry::domain::providers::Settings;

/// 登记处持久化端口：供应商与模型分开保存（机制/文件名在适配层）。
pub trait SettingsStore {
    fn load(&self) -> Result<Settings, String>;
    fn save(&self, settings: &Settings) -> Result<(), String>;
}
