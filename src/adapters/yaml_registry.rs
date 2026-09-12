//! 登记处持久化：providers.yaml 读写（实现 core 的 ProviderStore 端口）。
//! 密钥只落本文件（unix 下 0600）；drvfs/NTFS 上 chmod 无效属平台限制，如实告知不隐瞒。

use crate::core::ports::ProviderStore;
use crate::core::providers::Registry;
use std::path::PathBuf;

pub struct YamlRegistryStore {
    path: PathBuf,
}

impl YamlRegistryStore {
    pub fn new(path: PathBuf) -> YamlRegistryStore {
        YamlRegistryStore { path }
    }
}

impl ProviderStore for YamlRegistryStore {
    fn load(&self) -> Result<Registry, String> {
        match std::fs::read_to_string(&self.path) {
            Ok(t) => serde_yaml::from_str(&t).map_err(|e| format!("providers.yaml 非法：{}", e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Registry::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    fn save(&self, registry: &Registry) -> Result<(), String> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = serde_yaml::to_string(registry).map_err(|e| e.to_string())?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("建登记处目录失败：{}", e))?;
        }
        std::fs::write(&self.path, text).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}