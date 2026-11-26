//! 目的：会话类型表的装配输入——`systools/profiles.yaml`。
//! 管：经 `Assets` 读文档、反序列化成 `Profiles`、做装配期校验；缺文件 / 非法如实报错。
//! 不管：表的形状与规则（在 `domain/profile.rs`）；目录与读盘（在 `assets`）。
//! 联动：只有入口层的组合根构造它；`ids()` 交给提示词册子，`check_roles` 吃工具单元的机制席位。

use crate::units::business::session_model::api::Profiles;
use crate::units::resource::assets::api::{Assets, Doc};
use std::sync::Arc;

/// 目的：会话类型表的装配输入（读 `systools/profiles.yaml`）。
pub struct YamlProfiles {
    assets: Arc<dyn Assets>,
}

impl YamlProfiles {
    /// 目的：`assets` = 随包只读内容的入站面（`systools/` 的落点与读法归它）。
    pub fn new(assets: Arc<dyn Assets>) -> YamlProfiles {
        YamlProfiles { assets }
    }

    /// 目的：读出全部会话类型并校验。
    /// 返回：校验通过的 `Profiles`。
    /// 错误：读不了 / 语法非法 / 规则不过，文案都点名 `systools/profiles.yaml`。
    pub fn load(&self) -> Result<Profiles, String> {
        let text = self
            .assets
            .read(Doc::Profiles)
            .map_err(|e| format!("会话类型表读不了（{}）：{}", Doc::Profiles.label(), e))?;
        let profiles: Profiles = yaml_serde::from_str(&text)
            .map_err(|e| format!("会话类型表非法（{}）：{}", Doc::Profiles.label(), e))?;
        profiles.validate()?;
        Ok(profiles)
    }
}
