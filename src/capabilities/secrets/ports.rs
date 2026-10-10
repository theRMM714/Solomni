//! 目的：隐秘字段的出站端口（只有 service.rs 持有，R12）：值存储。
//! 管：SecretStore 的形状——一份 (模块/字段 → 值) 的读写。
//! 不管：权限与路径（在 detail 实现）；字段声明（在 workspace）。
//! 联动：实现见 detail/yaml_secrets.rs；由 service.rs 消费。

use std::collections::BTreeMap;

/// 目的：隐秘字段的持久化端口（键 = 模块/字段）。
pub trait SecretStore: Send + Sync {
    /// 目的：读出全部值；文件不存在 = 空表。
    fn load(&self) -> Result<BTreeMap<String, String>, String>;
    /// 目的：整表写回（值只落在这里）。
    fn save(&self, values: &BTreeMap<String, String>) -> Result<(), String>;
}
