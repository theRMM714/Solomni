//! 目的：登记处的状态与用例——四份 yaml 的内存形态只由这里写。
//! 管：实现 api::Registry 与 RegistryOps（视图 / 只读事实 / 写三类方法）；注入端口与 llm 能力面；应用模型发现结果。
//! 不管：yaml 解析与落盘（在 detail/yaml_settings.rs）；DTO 与校验规则（在 domain）；适配器装配（在组合根）。
//! 联动：组合根 new 时注入 SettingsStore 与 llm::api::Llm；对外只经 registry::api::Registry。

use crate::units::catalog::modules::api::Roster;
use crate::units::catalog::registry::api::{AgentView, ModelView, ProviderView, Registry};
use crate::units::catalog::registry::domain::agents;
use crate::units::catalog::registry::domain::providers::{
    self, AppSettings, ModelDraft, ModelEntry, Provider, Settings,
};
use crate::units::catalog::registry::ports::SettingsStore;
use crate::units::llm::api::{Channel, Llm, ProbeOutcome, ReplayReport, ToolMode};
use crate::kernel::ports::Log;
use std::sync::Arc;

/// 目的：只在目标为空时用发现值填上（已登记的一律保留）。
fn fill_if_empty(target: &mut String, found: Option<String>) {
    if target.trim().is_empty()
        && let Some(p) = found
    {
        *target = p;
    }
}

/// 目的：一份 app 设置必须与现有模型登记自洽才能落盘（整份设置与单项授权两个写入口共用）。
/// 约束：不合法就如实拒绝，不把坏的策略（绝对路径 / .. / 未知字段 / 悬空思考档）落进登记处。
fn validate_app(
    app: &AppSettings,
    models: &std::collections::BTreeMap<String, ModelEntry>,
) -> Result<(), String> {
    crate::units::catalog::registry::domain::providers::validate_sandbox_settings(
        &app.sandbox,
    )
    .map_err(|e| format!("settings.yaml 配置非法：{}", e))?;
    providers::validate_compact_settings(app, models)
        .map_err(|e| format!("settings.yaml 配置非法：{}", e))?;
    providers::validate_storage_settings(app)
        .map_err(|e| format!("settings.yaml 配置非法：{}", e))?;
    crate::units::catalog::registry::domain::providers::validate_sandbox_authorization(
        &app.sandbox.policy,
    )
    .map_err(|e| format!("settings.yaml 配置非法：{}", e))?;
    // 思考目录：新目录必须自洽，且不能删掉仍被模型引用的档位（否则旧映射悬空）。
    providers::validate_thinking_catalog(&app.thinking)
        .map_err(|e| format!("settings.yaml 配置非法：{}", e))?;
    for (mid, m) in models {
        for level in m.thinking.levels.keys() {
            if !app.thinking.levels.iter().any(|l| &l.id == level) {
                return Err(format!(
                    "思考档位 {} 仍被模型 {} 使用：先改模型登记再删它",
                    level, mid
                ));
            }
        }
        if let Some(d) = &m.thinking.default
            && !app.thinking.levels.iter().any(|l| &l.id == d)
        {
            return Err(format!("模型 {} 的默认思考档位 {} 不在新目录里", mid, d));
        }
    }
    Ok(())
}

/// 目的：不经进程内的登记处状态、直接把"允许写 ACL"记进 settings.yaml——`--setup fence` 在装配登记处之前就要落这一笔。
/// 约束：只改这一项；走与整份设置同一套校验与落盘（四份 yaml 一并刷新）。
pub fn grant_write_acl(store: &dyn SettingsStore) -> Result<(), String> {
    let mut settings = store.load()?;
    settings.app.sandbox.policy.windows.write_acl = true;
    validate_app(&settings.app, &settings.models)?;
    store.save(&settings)
}

/// 目的：登记处能力：持四份 yaml 的内存形态，按用例读写、改完即落盘。
pub struct RegistryService {
    /// **状态**：登记处的内存形态。私有——别的能力只经 `Registry` 读。
    settings: Settings,
    store: Arc<dyn SettingsStore + Send + Sync>,
    /// 通道探测（原生工具调用 / 回放形状）与模型发现：**调 llm 的用例面**，不持它的端口（R12）。
    llm: Arc<dyn Llm + Send + Sync>,
    log: Arc<dyn Log + Send + Sync>,
}

impl RegistryService {
    /// 目的：按 draft 的供应商 + api_model 造一条通道（模型还没落盘时给保存闸门用）。
    /// 约束：供应商必须已在册；密钥只在本层用，不出视图。
    fn draft_channel(&self, provider: &str, api_model: &str) -> Result<Channel, String> {
        let p = self
            .settings
            .providers
            .get(provider)
            .ok_or_else(|| format!("无此供应商：{}", provider))?;
        if api_model.trim().is_empty() {
            return Err("api_model 不能为空".to_string());
        }
        Ok(Channel {
            base_url: p.base_url.clone(),
            api_key: p.api_key.clone(),
            model: api_model.to_string(),
        })
    }

    /// 目的：组合根专用：注入端口，并把四份 yaml 读进内存。
    pub fn new(
        store: Arc<dyn SettingsStore + Send + Sync>,
        llm: Arc<dyn Llm + Send + Sync>,
        log: Arc<dyn Log + Send + Sync>,
    ) -> Result<RegistryService, String> {
        let settings = store.load()?;
        crate::units::catalog::registry::domain::providers::validate_sandbox_settings(
            &settings.app.sandbox,
        )
        .map_err(|e| format!("settings.yaml 配置非法：{}", e))?;
        providers::validate_compact_percents(&settings.app)
            .map_err(|e| format!("settings.yaml 配置非法：{}", e))?;
        providers::validate_storage_settings(&settings.app)
            .map_err(|e| format!("settings.yaml 配置非法：{}", e))?;
        providers::validate_thinking(&settings.models, &settings.app.thinking)
            .map_err(|e| format!("登记处配置非法：{}", e))?;
        Ok(RegistryService {
            settings,
            store,
            llm,
            log,
        })
    }

    /// 目的：**未登记**时采用依赖补齐装到项目内的可运行包（显式登记优先）。
    /// 约束：只填内存、不落盘——用户显式登记的路径永远优先，且这里不改用户的 `settings.yaml`。
    pub fn apply_discovered(
        &mut self,
        qemu: Option<String>,
        virtiofsd: Option<String>,
        smbd: Option<String>,
    ) {
        fill_if_empty(&mut self.settings.app.qemu_path, qemu);
        fill_if_empty(&mut self.settings.app.virtiofsd_path, virtiofsd);
        fill_if_empty(&mut self.settings.app.smbd_path, smbd);
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

/// 目的：保存模型草稿：原生工具模式先实测通道（支持原生工具调用 + 至少一种回放形状被接受）。
/// 参数：`r` 登记处能力面；`draft` 待保存的模型草稿。
/// 返回：写入并落盘成功返回 `()`；信封形态是用户显式选择，不实测、直接保存。
/// 错误：通道不支持原生工具调用 / 无法判定 / 不接受任何回放形状——错误文案点明「模型未保存」。
/// 约束：所有写入口共用这一处判据（不只是 Web 表单）。
pub fn save_model(r: &mut dyn Registry, draft: ModelDraft) -> Result<(), String> {
    if draft.tools == ToolMode::Native {
        let (provider, api_model) = (draft.provider.clone(), draft.api_model.clone());
        match r.probe_channel_tools(&provider, &api_model)? {
            ProbeOutcome::Supported { .. } => {}
            ProbeOutcome::Unsupported { detail } => {
                return Err(format!(
                    "供应商不支持原生工具调用：{}；模型未保存。",
                    detail
                ));
            }
            ProbeOutcome::Unknown { detail } => {
                return Err(format!(
                    "无法判定该通道是否支持原生工具调用：{}；模型未保存，请重试。",
                    detail
                ));
            }
        }
        let report = r.probe_channel_replay(&provider, &api_model)?;
        if !report.shapes.iter().any(|s| s.accepted) {
            return Err("该通道不接受任何工具调用回放形状；模型未保存。".to_string());
        }
    }
    r.model_upsert(draft)
}

/// 目的：保存 agent：目录保留名与模块存在性都在这一处校验，再交给登记处写。
/// 参数：`r` 登记处能力面；`draft` 用户给的定义；`roster` 清单事实（模块校验用）；`reserved` 目录保留名。
/// 返回：写入并落盘成功返回 `()`。
/// 错误：名字是保留名、模块不在清单里、模型不存在——都不写入。
/// 约束：清单归 `modules`、保留名归 `tools`，本函数只取调用方给的事实，不自己去找。
pub fn save_agent(
    r: &mut dyn Registry,
    draft: &crate::units::catalog::registry::api::AgentDraft,
    roster: &Roster,
    reserved: &[String],
) -> Result<(), String> {
    agents::check_reserved(&draft.name, reserved)?;
    r.agent_upsert(
        &draft.name,
        &draft.modules,
        &draft.model,
        &draft.note,
        &draft.chara,
        roster,
    )
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

    fn core_channel(&self) -> Result<Channel, String> {
        self.settings.core_channel()
    }

    fn channel(&self, model: Option<&str>) -> Result<Channel, String> {
        self.settings.effective_channel(model)
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
        self.settings.context_of(model)
    }

    fn thinking_fragment(
        &self,
        model: Option<&str>,
        level: Option<&str>,
    ) -> Result<Option<serde_json::Value>, String> {
        self.settings.thinking_fragment(model, level)
    }

    fn thinking_supports(&self, model: Option<&str>, level: &str) -> bool {
        self.settings.thinking_supports(model, level)
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

    fn model_upsert(&mut self, draft: ModelDraft) -> Result<(), String> {
        let ModelDraft {
            id,
            name,
            api_model,
            provider,
            note,
            tools,
            context,
            thinking,
        } = draft;
        if id.is_empty() || name.is_empty() || api_model.is_empty() || provider.is_empty() {
            return Err("id / name / api_model / provider 均不能为空".to_string());
        }
        if !self.settings.providers.contains_key(&provider) {
            return Err(format!("无此供应商：{}", provider));
        }
        // 工具调用形态随 draft 一起进来（原生形态的实测闸门在 runtime 的保存用例里，落盘前已通过）。
        // 思考强度：档位必须在全局目录里、默认档必须被支持、片段不得覆盖协议字段。
        providers::validate_thinking_spec(&thinking, &self.settings.app.thinking)
            .map_err(|e| format!("模型 {} 的思考强度非法：{}", id, e))?;
        // 上下文窗口：0 = 用户不填（自动压缩按 DEFAULT_CONTEXT_TOKENS 兜底），原样保存。
        self.settings.models.insert(
            id,
            ModelEntry {
                name,
                api_model,
                provider,
                note,
                tools,
                context,
                thinking,
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
        if self.settings.app.compact_model.trim() == id {
            return Err(format!(
                "{} 是压缩用模型；请先把设置里的压缩模型改成别的再删",
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
        chara: &str,
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
                chara: chara.trim().to_string(),
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
        validate_app(&app, &self.settings.models)?;
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
        if let Some(mode) = want
            && let Some(m) = self.settings.models.get_mut(id)
            && m.tools != mode
        {
            m.tools = mode;
            self.save("registry::probe_model_tools")?;
        }
        Ok(outcome)
    }

    fn probe_replay_shape(&self, id: &str) -> Result<ReplayReport, String> {
        let channel = self.settings.resolve(id)?;
        self.llm.probe_replay(&channel)
    }

    fn probe_channel_tools(&self, provider: &str, api_model: &str) -> Result<ProbeOutcome, String> {
        let channel = self.draft_channel(provider, api_model)?;
        self.llm.probe_tools(&channel)
    }

    fn probe_channel_replay(
        &self,
        provider: &str,
        api_model: &str,
    ) -> Result<ReplayReport, String> {
        let channel = self.draft_channel(provider, api_model)?;
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

#[cfg(test)]
mod tests {
    use super::fill_if_empty;

    /// 目的：显式登记优先；只在空（未登记）时才采用发现值；没有发现值不动。
    #[test]
    fn discovered_fills_only_when_unset() {
        let mut explicit = "/opt/qemu".to_string();
        fill_if_empty(&mut explicit, Some("/project/.vm/qemu".to_string()));
        assert_eq!(explicit, "/opt/qemu", "显式登记优先");
        let mut empty = String::new();
        fill_if_empty(&mut empty, Some("/project/.vm/qemu".to_string()));
        assert_eq!(empty, "/project/.vm/qemu");
        let mut blank = "  ".to_string();
        fill_if_empty(&mut blank, None);
        assert_eq!(blank, "  ", "没有发现值就不动");
    }
}
