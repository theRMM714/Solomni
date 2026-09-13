//! 登记处持久化：providers.yaml 与 models.yaml 分开读写（实现 core 的 SettingsStore 端口）。
//! 密钥只落 providers.yaml（unix 下 0600）；drvfs/NTFS 上 chmod 无效属平台限制，如实告知不隐瞒。

use crate::core::agents::Agents;
use crate::core::ports::SettingsStore;
use crate::core::providers::{AppSettings, ModelEntry, Provider, Settings};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub struct YamlSettingsStore {
    providers_path: PathBuf,
    models_path: PathBuf,
    settings_path: PathBuf,
    agents_path: PathBuf,
}

/// agents.yaml 的文件形态。
#[derive(Default, Serialize, Deserialize)]
struct AgentsFile {
    #[serde(default)]
    agents: Agents,
}

/// providers.yaml 的文件形态。
#[derive(Default, Serialize, Deserialize)]
struct ProvidersFile {
    #[serde(default)]
    providers: BTreeMap<String, Provider>,
}

/// models.yaml 的文件形态（模型与核心默认模型在一起）。
#[derive(Default, Serialize, Deserialize)]
struct ModelsFile {
    #[serde(default)]
    models: BTreeMap<String, ModelEntry>,
    #[serde(default)]
    core: Option<String>,
}

impl YamlSettingsStore {
    pub fn new(providers_path: PathBuf, models_path: PathBuf, settings_path: PathBuf, agents_path: PathBuf) -> YamlSettingsStore {
        YamlSettingsStore { providers_path, models_path, settings_path, agents_path }
    }

    /// 写一个 yaml 文件：建目录 → 写盘 → unix 下 0600。
    fn write(path: &Path, text: &str) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("建登记处目录失败：{}", e))?;
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

impl SettingsStore for YamlSettingsStore {
    fn load(&self) -> Result<Settings, String> {
        let providers = match std::fs::read_to_string(&self.providers_path) {
            Ok(t) => serde_yaml::from_str::<ProvidersFile>(&t)
                .map_err(|e| format!("providers.yaml 非法：{}", e))?
                .providers,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e.to_string()),
        };
        let (models, core) = match std::fs::read_to_string(&self.models_path) {
            Ok(t) => {
                let f: ModelsFile = serde_yaml::from_str(&t).map_err(|e| format!("models.yaml 非法：{}", e))?;
                (f.models, f.core)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (BTreeMap::new(), None),
            Err(e) => return Err(e.to_string()),
        };
        let app = match std::fs::read_to_string(&self.settings_path) {
            Ok(t) => serde_yaml::from_str::<AppSettings>(&t).map_err(|e| format!("settings.yaml 非法：{}", e))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => AppSettings::default(),
            Err(e) => return Err(e.to_string()),
        };
        let agents = match std::fs::read_to_string(&self.agents_path) {
            Ok(t) => serde_yaml::from_str::<AgentsFile>(&t).map_err(|e| format!("agents.yaml 非法：{}", e))?.agents,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Agents::new(),
            Err(e) => return Err(e.to_string()),
        };
        Ok(Settings { providers, models, core, app, agents })
    }

    fn save(&self, settings: &Settings) -> Result<(), String> {
        let providers = ProvidersFile { providers: settings.providers.clone() };
        let text = serde_yaml::to_string(&providers).map_err(|e| e.to_string())?;
        Self::write(&self.providers_path, &text)?;

        let models = ModelsFile { models: settings.models.clone(), core: settings.core.clone() };
        let text = serde_yaml::to_string(&models).map_err(|e| e.to_string())?;
        Self::write(&self.models_path, &text)?;

        let text = serde_yaml::to_string(&settings.app).map_err(|e| e.to_string())?;
        Self::write(&self.settings_path, &text)?;

        let agents = AgentsFile { agents: settings.agents.clone() };
        let text = serde_yaml::to_string(&agents).map_err(|e| e.to_string())?;
        Self::write(&self.agents_path, &text)?;
        Ok(())
    }
}
