//! 目的：隐秘字段的落盘实现（.home/secrets.yaml；unix 下 0600，drvfs/NTFS 上 chmod 无效属平台限制）。
//! 管：一份 (模块/字段 → 值) 的读写；文件不存在 = 空表；文件非法 = 如实报错。
//! 不管：字段声明；实际注入子进程。
//! 联动：实现 ports::SecretStore；路径由组合根给（.home/secrets.yaml）。

use crate::capabilities::secrets::ports::SecretStore;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// secrets.yaml 的文件形态。
#[derive(Default, Serialize, Deserialize)]
struct SecretsFile {
    #[serde(default)]
    values: BTreeMap<String, String>,
}

/// 目的：隐秘字段的文件存储（组合根构造）。
pub struct YamlSecrets {
    path: PathBuf,
}

impl YamlSecrets {
    /// 目的：用文件路径造一个存储（组合根给 .home/secrets.yaml）。
    pub fn new(path: PathBuf) -> YamlSecrets {
        YamlSecrets { path }
    }

    /// 目的：写盘：建目录 → 写 → unix 下 0600。
    fn write(path: &Path, text: &str) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("建隐秘字段目录失败：{}", e))?;
        }
        std::fs::write(path, text).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}

impl SecretStore for YamlSecrets {
    fn load(&self) -> Result<BTreeMap<String, String>, String> {
        match std::fs::read_to_string(&self.path) {
            Ok(t) => Ok(yaml_serde::from_str::<SecretsFile>(&t)
                .map_err(|e| format!("secrets.yaml 非法：{}", e))?
                .values),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(e.to_string()),
        }
    }

    fn save(&self, values: &BTreeMap<String, String>) -> Result<(), String> {
        let text = yaml_serde::to_string(&SecretsFile {
            values: values.clone(),
        })
        .map_err(|e| e.to_string())?;
        Self::write(&self.path, &text)
    }
}
