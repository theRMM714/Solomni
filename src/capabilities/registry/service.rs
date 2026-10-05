//! 登记处的**状态与用例**：四份 yaml 的内存形态（`Settings`）只由这里写。
//!
//! 端口（持久化）与**别能力的 api 面**（通道探测 / 模型发现 → `llm::api::Llm`）由**组合根**注入；
//! 本文件不 new 任何适配器、也**不持别人的端口**（R12）。
//! 对外只经 `registry::api::Registry`：`conductor` 与呈现层拿不到 `settings` 字段。

use crate::capabilities::llm::api::{Channel, Llm, ProbeOutcome, ReplayReport, ToolMode};
use crate::capabilities::registry::api::{AgentView, ModelView, ProviderView, Registry};
use crate::capabilities::registry::domain::agents;
use crate::capabilities::registry::domain::providers::{
    AppSettings, ModelEntry, Provider, Settings, DEFAULT_CONTEXT_TOKENS,
};
use crate::capabilities::registry::ports::SettingsStore;
use crate::capabilities::workspace::api::Roster;
use crate::kernel::ports::Log;
use std::sync::Arc;

/// 登记处能力：持四份 yaml 的内存形态，按用例读写、改完即落盘。
pub struct RegistryService {
    /// **状态**：登记处的内存形态。私有——别的能力只经 `Registry` 读。
    settings: Settings,
    store: Arc<dyn SettingsStore + Send + Sync>,
    /// 通道探测（原生工具调用 / 回放形状）与模型发现：**调 llm 的用例面**，不持它的端口（R12）。
    llm: Arc<dyn Llm + Send + Sync>,
    log: Arc<dyn Log + Send + Sync>,
}

impl RegistryService {
    /// 组合根专用：注入端口，并把四份 yaml 读进内存。
    pub fn new(
        store: Arc<dyn SettingsStore + Send + Sync>,
        llm: Arc<dyn Llm + Send + Sync>,
        log: Arc<dyn Log + Send + Sync>,
    ) -> Result<RegistryService, String> {
        let settings = store.load()?;
        Ok(RegistryService {
            settings,
            store,
            llm,
            log,
        })
    }

    /// 落盘并把失败如实记进日志（写方法共用这一句）。
    fn save(&self, at: &str) -> Result<(), String> {
        let r = self.store.save(&self.settings);
        if let Err(e) = &r {
            self.log.error(at, &format!("登记处持久化失败：{}", e));
        }
        r
    }
}

impl Registry for RegistryService {
    // ---- 视图 ----

    fn provider_views(&self) -> Vec<ProviderView> {
        self.settings.provider_views()
    }

    fn model_views(&self) -> Vec<ModelView> {
        self.settings.model_views()
    }

    fn agent_views(&self) -> Vec<AgentView> {
        agents::views(&self.settings.agents)
    }

    fn pick_agents(&self, names: &[String]) -> Result<Vec<AgentView>, String> {
        agents::pick(&self.settings.agents, names)
    }

    fn app(&self) -> &AppSettings {
        &self.settings.app
    }

    fn app_settings(&self) -> AppSettings {
        self.settings.app.clone()
    }

    fn core_model(&self) -> Option<String> {
        self.settings.core.clone()
    }

    // ---- 只读事实 ----

    fn resolve(&self, model: &str) -> Result<Channel, String> {
        self.settings.resolve(model)
    }

    fn core_channel(&self) -> Option<Channel> {
        self.settings.core_channel()
    }

    fn channel(&self, model: Option<&str>) -> Option<Channel> {
        match model {
            Some(id) => self.settings.resolve(id).ok(),
            None => self.settings.core_channel(),
        }
    }

    fn has_model(&self, id: &str) -> bool {
        self.settings.models.contains_key(id)
    }

    fn any_models(&self) -> bool {
        !self.settings.models.is_empty()
    }

    fn has_agent(&self, name: &str) -> bool {
        self.settings.agents.contains_key(name)
    }

    fn tool_mode(&self, model: Option<&str>) -> ToolMode {
        self.settings.tool_mode_for(model)
    }

    fn context_of(&self, model: Option<&str>) -> u64 {
        model
            .map(|s| s.to_string())
            .or_else(|| self.settings.core.clone())
            .and_then(|id| self.settings.models.get(&id).map(|m| m.context))
            .unwrap_or(DEFAULT_CONTEXT_TOKENS)
    }

    fn snapshot(&self) -> Settings {
        self.settings.clone()
    }

    // ---- 写 ----

    fn provider_upsert(&mut self, id: &str, base_url: &str, api_key: &str) -> Result<(), String> {
        if id.is_empty() || base_url.is_empty() {
            return Err("id / base_url 不能为空".to_string());
        }
        let key = if api_key.is_empty() {
            self.settings
                .providers
                .get(id)
                .map(|p| p.api_key.clone())
                .ok_or_else(|| "api_key 不能为空".to_string())?
        } else {
            api_key.to_string()
        };
        self.settings.providers.insert(
            id.to_string(),
            Provider {
                base_url: base_url.to_string(),
                api_key: key,
            },
        );
        self.save("registry::provider_upsert")
    }

    fn provider_remove(&mut self, id: &str) -> Result<bool, String> {
        let referenced: Vec<String> = self
            .settings
            .models
            .iter()
            .filter(|(_, m)| m.provider == id)
            .map(|(mid, _)| mid.clone())
            .collect();
        if !referenced.is_empty() {
            return Err(format!(
                "供应商 {} 仍被模型引用：{}；请先删除这些模型",
                id,
                referenced.join("、")
            ));
        }
        let removed = self.settings.providers.remove(id).is_some();
        if removed {
            self.save("registry::provider_remove")?;
        }
        Ok(removed)
    }

    fn model_upsert(
        &mut self,
        id: &str,
        name: &str,
        api_model: &str,
        provider: &str,
        note: &str,
        context: u64,
    ) -> Result<(), String> {
        if id.is_empty() || name.is_empty() || api_model.is_empty() || provider.is_empty() {
            return Err("id / name / api_model / provider 均不能为空".to_string());
        }
        if !self.settings.providers.contains_key(provider) {
            return Err(format!("无此供应商：{}", provider));
        }
        // 工具调用形态：编辑时**保留原值**（登记表单暂不带这个字段，不能因为没带就重置成缺省），
        // 新建缺省 envelope（任何供应商都能用的手写信封）。
        let (tools, old_ctx) = self
            .settings
            .models
            .get(id)
            .map(|m| (m.tools, m.context))
            .unwrap_or_default();
        // 上下文窗口：表单没带（0）就保留现值（新建缺省 32k）——编辑别的字段不该顺手重置它。
        let context = if context == 0 { old_ctx } else { context };
        self.settings.models.insert(
            id.to_string(),
            ModelEntry {
                name: name.to_string(),
                api_model: api_model.to_string(),
                provider: provider.to_string(),
                note: note.to_string(),
                tools,
                context,
            },
        );
        self.save("registry::model_upsert")
    }

    fn model_remove(&mut self, id: &str) -> Result<bool, String> {
        if self.settings.core.as_deref() == Some(id) {
            return Err(format!(
                "{} 是核心默认模型；请先把核心默认模型改成别的再删",
                id
            ));
        }
        let removed = self.settings.models.remove(id).is_some();
        if removed {
            self.save("registry::model_remove")?;
        }
        Ok(removed)
    }

    fn core_set_model(&mut self, id: &str) -> Result<bool, String> {
        if !self.settings.models.contains_key(id) {
            return Ok(false);
        }
        self.settings.core = Some(id.to_string());
        self.save("registry::core_set_model")?;
        Ok(true)
    }

    fn agent_upsert(
        &mut self,
        name: &str,
        module_ids: &[String],
        model: &str,
        note: &str,
        roster: &Roster,
    ) -> Result<(), String> {
        agents::validate_name(name)?;
        for id in module_ids {
            if !roster.modules.iter().any(|m| &m.manifest.id == id) {
                return Err(format!("无此模块：{}", id));
            }
        }
        if !model.is_empty() && !self.settings.models.contains_key(model) {
            return Err(format!("无此模型：{}", model));
        }
        self.settings.agents.insert(
            name.to_string(),
            agents::Agent {
                modules: module_ids.to_vec(),
                model: if model.is_empty() {
                    None
                } else {
                    Some(model.to_string())
                },
                note: note.to_string(),
            },
        );
        self.save("registry::agent_upsert")
    }

    fn agent_remove(&mut self, name: &str) -> Result<bool, String> {
        let removed = self.settings.agents.remove(name).is_some();
        if removed {
            self.save("registry::agent_remove")?;
        }
        Ok(removed)
    }

    fn set_app_settings(&mut self, app: AppSettings) -> Result<(), String> {
        self.settings.app = app;
        self.save("registry::set_app_settings")
    }

    fn probe_model_tools(&mut self, id: &str) -> Result<ProbeOutcome, String> {
        let channel = self.settings.resolve(id)?;
        let outcome = self.llm.probe_tools(&channel)?;
        let want = match &outcome {
            ProbeOutcome::Supported { .. } => Some(ToolMode::Native),
            ProbeOutcome::Unsupported { .. } => Some(ToolMode::Envelope),
            ProbeOutcome::Unknown { .. } => None,
        };
        if let Some(mode) = want {
            if let Some(m) = self.settings.models.get_mut(id) {
                if m.tools != mode {
                    m.tools = mode;
                    self.save("registry::probe_model_tools")?;
                }
            }
        }
        Ok(outcome)
    }

    fn probe_replay_shape(&self, id: &str) -> Result<ReplayReport, String> {
        let channel = self.settings.resolve(id)?;
        self.llm.probe_replay(&channel)
    }

    fn discover_models(&self, provider_id: &str) -> Result<Vec<String>, String> {
        let provider = self
            .settings
            .providers
            .get(provider_id)
            .ok_or_else(|| format!("无此供应商：{}", provider_id))?;
        let outcome = self.llm.list_models(&provider.base_url, &provider.api_key);
        match &outcome {
            Ok(models) => self.log.info(
                "registry::discover_models",
                &format!("供应商 {} 拉取模型 {} 个", provider_id, models.len()),
            ),
            Err(e) => self.log.error(
                "registry::discover_models",
                &format!("供应商 {} 拉取模型失败：{}", provider_id, e),
            ),
        }
        outcome
    }
}
