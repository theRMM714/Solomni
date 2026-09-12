//! 核心层：定义抽象（ports）、编排业务（会话/引擎）。
//! 分层纪律：本层不出现 std::fs 读文件、ureq、stdin/stdout——机制全部在 adapters，
//! 装配（new 适配器）只发生在 main 组合根。前端只见 Core 门面与 SessionEvent 流。

pub mod collab;
pub mod engine;
pub mod envelope;
pub mod events;
pub mod module;
pub mod ports;
pub mod prompt;
pub mod providers;
pub mod session;

pub use events::{Pending, SessionEvent};
pub use ports::{ChatGateway, ModuleSource, PromptSource, ProviderStore};

use crate::core::collab::CollabSession;
use crate::core::module::Module;
use crate::core::prompt::Prompts;
use crate::core::providers::Registry;

/// 核心门面：持有注入的端口；前端只经此操作，永不接触端口对象。
pub struct Core {
    store: Box<dyn ProviderStore>,
    source: Box<dyn ModuleSource>,
    gateway: Box<dyn ChatGateway>,
    registry: Registry,
    prompts: Prompts,
}

impl Core {
    /// 组合根专用：main 负责创建适配器并注入；core 不自建任何具体实现。
    pub fn new(
        store: Box<dyn ProviderStore>,
        source: Box<dyn ModuleSource>,
        gateway: Box<dyn ChatGateway>,
        prompt_source: Box<dyn PromptSource>,
    ) -> Result<Core, String> {
        let registry = store.load()?;
        let prompts = prompt_source.load()?;
        Ok(Core { store, source, gateway, registry, prompts })
    }

    /// 清单即事实：每次调用重扫（策略在 core，机制在 ModuleSource）。
    pub fn scan(&self) -> module::Roster {
        self.source.scan()
    }

    // ---- 登记处操作端口（密钥在此层进出；前端只见 id） ----

    pub fn provider_list(&self) -> Vec<String> {
        self.registry.display_lines()
    }

    /// 当前全局默认供应商 id（前端展示用，不含密钥）。
    pub fn provider_default(&self) -> Option<String> {
        self.registry.default.clone()
    }

    /// 新增/更新供应商；首入者自动成为默认。
    pub fn provider_upsert(&mut self, id: &str, base_url: &str, api_key: &str, models: &[String]) -> Result<(), String> {
        if id.is_empty() || base_url.is_empty() || api_key.is_empty() {
            return Err("id / base_url / api_key 均不能为空".to_string());
        }
        self.registry.providers.insert(
            id.to_string(),
            providers::Provider {
                kind: "llm".to_string(),
                base_url: base_url.to_string(),
                api_key: api_key.to_string(),
                models: models.to_vec(),
            },
        );
        if self.registry.default.is_none() {
            self.registry.default = Some(id.to_string());
        }
        self.store.save(&self.registry)
    }

    pub fn provider_remove(&mut self, id: &str) -> Result<bool, String> {
        let removed = self.registry.providers.remove(id).is_some();
        if removed && self.registry.default.as_deref() == Some(id) {
            self.registry.default = self.registry.providers.keys().next().cloned();
        }
        if removed {
            self.store.save(&self.registry)?;
        }
        Ok(removed)
    }

    pub fn provider_set_default(&mut self, id: &str) -> Result<bool, String> {
        if !self.registry.providers.contains_key(id) {
            return Ok(false);
        }
        self.registry.default = Some(id.to_string());
        self.store.save(&self.registry)?;
        Ok(true)
    }

    // ---- 会话工厂（包装端口访问：会话拿到的是引用，前端拿到的永远是会话） ----

    /// 模式一：单模块直连。
    pub fn start_direct(&self, id: &str) -> Result<session::DirectSession, String> {
        let m = self.find(id)?;
        let provider = self
            .registry
            .resolve(m.selected_provider.as_deref(), m.manifest.model.provider.as_deref())
            .map(|(_, p)| p);
        let (chat, note) = self.gateway.member_channel(provider, id);
        Ok(session::DirectSession::new(id, m.system_block(&self.prompts), chat, note))
    }

    /// 模式三：全能（ids 空 = 全部模块；拼装职责提示词）。
    pub fn start_omni(&self, ids: &str) -> Result<session::OmniSession, String> {
        let roster = self.scan();
        let chosen = pick_borrowed(&roster, ids);
        if chosen.is_empty() {
            return Err("无可拼装模块".to_string());
        }
        let modules = chosen
            .iter()
            .map(|m| format!("\n== {} ==\n{}", m.manifest.id, m.manifest.system))
            .collect::<Vec<_>>()
            .join("");
        let merged = self.prompts.render(&self.prompts.core.omni.system, &[("modules", modules)]);
        let provider = self.registry.resolve(None, None).map(|(_, p)| p);
        let (chat, note) = self.gateway.member_channel(provider, "全能");
        Ok(session::OmniSession::new(merged, chat, note))
    }

    /// 模式二：多模块协作（ids = 点名名单，或 "?" 委托代拟）。
    pub fn start_collab(&self, ids: &str) -> Result<CollabSession, String> {
        CollabSession::start(self.gateway.as_ref(), self.source.as_ref(), &self.registry, self.prompts.clone(), ids)
    }

    fn find(&self, id: &str) -> Result<Module, String> {
        self.scan()
            .modules
            .into_iter()
            .find(|m| m.manifest.id == id)
            .ok_or_else(|| format!("无此模块：{}", id))
    }
}

/// 供 CollabSession 在门面保护下驱动各阶段（端口对象不出 Core）。
pub mod driven {
    pub fn set_task(
        core: &super::Core,
        s: &mut super::collab::CollabSession,
        task: &str,
    ) -> Vec<super::SessionEvent> {
        s.set_task(core.source.as_ref(), task)
    }

    pub fn confirm_slate(
        core: &super::Core,
        s: &mut super::collab::CollabSession,
        ok: bool,
    ) -> Vec<super::SessionEvent> {
        s.confirm_slate(core.source.as_ref(), ok)
    }

    pub fn begin(
        core: &super::Core,
        s: &mut super::collab::CollabSession,
        allow: bool,
    ) -> Vec<super::SessionEvent> {
        s.begin(core.gateway.as_ref(), allow, &|m: &super::module::Module| {
            m.system_block(&core.prompts)
        })
    }
}

fn pick_borrowed<'a>(roster: &'a module::Roster, ids: &str) -> Vec<&'a Module> {
    if ids.trim().is_empty() {
        return roster.modules.iter().collect();
    }
    ids.split(|c| c == ',' || c == '，')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|id| roster.modules.iter().find(|m| m.manifest.id == id))
        .collect()
}