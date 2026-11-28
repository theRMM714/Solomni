//! 目的：登记处持久化适配器——四份 yaml 的解码与编码（实现 ports::SettingsStore）。
//! 管：providers / models / settings / agents 四份 yaml 的文件形态、解码与校验；写回经 `Home` 端口。
//! 不管：文件机制与权限（在 `home` 单元）；内存形态与校验规则（在 domain）；状态与用例（在 service.rs）。
//! 联动：实现 registry/ports.rs 的 SettingsStore，由组合根装配给 service。

use crate::units::business::catalog::registry::api::{AppSettings, Settings};
use crate::units::business::catalog::registry::ports::SettingsStore;
use crate::units::pure::registry_model::api::Agents;
use crate::units::pure::registry_model::api::ModelEntry;
use crate::units::pure::registry_model::api::Provider;
use crate::units::resource::home::api::{Doc, Home};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

pub struct YamlSettingsStore {
    home: Arc<dyn Home>,
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

/// 目的：settings.yaml 读不懂、或读懂了却自相矛盾时的报错——点名文件与两条出路，别让人对着 serde 行号猜。
/// 参数：`reason` = 解码器或校验给的原文。
/// 返回：带 `data/.home/settings.yaml` 位置与出路的整句。
fn settings_error(reason: &str) -> String {
    format!(
        "settings.yaml 配置非法：{reason}（文件在 data/.home/settings.yaml：删掉多出来的键，或删掉整份文件后重启，会按默认重建）"
    )
}

impl YamlSettingsStore {
    /// 目的：用登记处读写面造一个存储（组合根给 `data/.home`）。
    pub fn new(home: Arc<dyn Home>) -> YamlSettingsStore {
        YamlSettingsStore { home }
    }
}

impl SettingsStore for YamlSettingsStore {
    fn load(&self) -> Result<Settings, String> {
        let providers = match self.home.read(Doc::Providers)? {
            Some(t) => {
                yaml_serde::from_str::<ProvidersFile>(&t)
                    .map_err(|e| format!("providers.yaml 非法：{}", e))?
                    .providers
            }
            None => BTreeMap::new(),
        };
        let (models, core) = match self.home.read(Doc::Models)? {
            Some(t) => {
                let f: ModelsFile =
                    yaml_serde::from_str(&t).map_err(|e| format!("models.yaml 非法：{}", e))?;
                (f.models, f.core)
            }
            None => (BTreeMap::new(), None),
        };
        let app = match self.home.read(Doc::Settings)? {
            Some(t) => {
                let app = yaml_serde::from_str::<AppSettings>(&t)
                    .map_err(|e| settings_error(&e.to_string()))?;
                // 首启没有文件时上面的 None 分支不拦（交给引导决定）；已存在的文件必须自洽。
                crate::units::business::catalog::registry::domain::providers::validate_sandbox_authorization(
                    &app.sandbox.policy,
                )
                .map_err(|e| settings_error(&e))?;
                app
            }
            None => AppSettings::default(),
        };
        let agents = match self.home.read(Doc::Agents)? {
            Some(t) => {
                yaml_serde::from_str::<AgentsFile>(&t)
                    .map_err(|e| format!("agents.yaml 非法：{}", e))?
                    .agents
            }
            None => Agents::new(),
        };
        Ok(Settings {
            providers,
            models,
            core,
            app,
            agents,
        })
    }

    fn save(&self, settings: &Settings) -> Result<(), String> {
        let providers = ProvidersFile {
            providers: settings.providers.clone(),
        };
        let text = yaml_serde::to_string(&providers).map_err(|e| e.to_string())?;
        self.home.write(Doc::Providers, &text)?;

        let models = ModelsFile {
            models: settings.models.clone(),
            core: settings.core.clone(),
        };
        let text = yaml_serde::to_string(&models).map_err(|e| e.to_string())?;
        self.home.write(Doc::Models, &text)?;

        let text = yaml_serde::to_string(&settings.app).map_err(|e| e.to_string())?;
        self.home.write(Doc::Settings, &text)?;

        let agents = AgentsFile {
            agents: settings.agents.clone(),
        };
        let text = yaml_serde::to_string(&agents).map_err(|e| e.to_string())?;
        self.home.write(Doc::Agents, &text)?;
        Ok(())
    }
}
