//! 目的：隐秘字段的解码与编码（`data/.home/secrets.yaml`；权限由 `home` 单元的私密写入负责）。
//! 管：一份 (模块/字段 → 值) 的文件形态与读写；文件不存在 = 空表；文件非法 = 如实报错。
//! 不管：文件机制与权限（在 `home` 单元）；字段声明；实际注入子进程。
//! 联动：实现 ports::SecretStore；由组合根装配（一份 `Home` 与登记处共用）。

use crate::units::business::catalog::secrets::ports::SecretStore;
use crate::units::resource::home::api::{Doc, Home};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

/// secrets.yaml 的文件形态。
#[derive(Default, Serialize, Deserialize)]
struct SecretsFile {
    #[serde(default)]
    values: BTreeMap<String, String>,
}

/// 目的：隐秘字段的文件存储（组合根构造）。
pub struct YamlSecrets {
    home: Arc<dyn Home>,
}

impl YamlSecrets {
    /// 目的：用登记处读写面造一个存储（组合根给 `data/.home`）。
    pub fn new(home: Arc<dyn Home>) -> YamlSecrets {
        YamlSecrets { home }
    }
}

impl SecretStore for YamlSecrets {
    fn load(&self) -> Result<BTreeMap<String, String>, String> {
        match self.home.read(Doc::Secrets)? {
            Some(t) => Ok(yaml_serde::from_str::<SecretsFile>(&t)
                .map_err(|e| format!("secrets.yaml 非法：{}", e))?
                .values),
            None => Ok(BTreeMap::new()),
        }
    }

    fn save(&self, values: &BTreeMap<String, String>) -> Result<(), String> {
        let text = yaml_serde::to_string(&SecretsFile {
            values: values.clone(),
        })
        .map_err(|e| e.to_string())?;
        self.home.write(Doc::Secrets, &text)
    }
}
