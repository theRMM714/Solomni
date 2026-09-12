//! 核心层：定义抽象（ports）、编排业务（会话/引擎）、会话中心。
//! 分层纪律：本层不出现文件读、ureq、stdin/stdout——机制全部在 adapters，
//! 装配（new 适配器）只发生在 main 组合根。前端只见 Core 门面、会话句柄与 SessionEvent 流。

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
use std::collections::HashMap;
use std::sync::Arc;

/// 前端唯一的会话标识。
pub type SessionId = u64;

/// 会话实例：模式与会话本体的合取（本体自带端口，可跨线程移动）。
pub enum Session {
    Direct(session::DirectSession),
    Omni(session::OmniSession),
    Collab(CollabSession),
}

/// 协作推进阶段：由前端按 pending 决定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollabStep {
    SetTask,
    ConfirmSlate,
    Begin,
    Answer,
}

/// 核心门面：持有注入的端口与会话中心；前端只经此操作。
/// 线程共享形态：组合根把它放进 Arc 加 Mutex（Web 多连接/多会话所需）。
pub struct Core {
    store: Arc<dyn ProviderStore + Send + Sync>,
    source: Arc<dyn ModuleSource + Send + Sync>,
    gateway: Arc<dyn ChatGateway + Send + Sync>,
    log: Arc<dyn crate::core::ports::Log + Send + Sync>,
    registry: Registry,
    prompts: Prompts,
    sessions: HashMap<SessionId, Session>,
    next_id: SessionId,
}

impl Core {
    /// 组合根专用：main 负责创建适配器并注入；core 不自建任何具体实现。
    pub fn new(
        store: Arc<dyn ProviderStore + Send + Sync>,
        source: Arc<dyn ModuleSource + Send + Sync>,
        gateway: Arc<dyn ChatGateway + Send + Sync>,
        prompt_source: Box<dyn PromptSource>,
        log: Arc<dyn crate::core::ports::Log + Send + Sync>,
    ) -> Result<Core, String> {
        let log_for_core = Arc::clone(&log);
        let outcome = (|| -> Result<Core, String> {
            let registry = store.load()?;
            let prompts = prompt_source.load()?;
            Ok(Core { store, source, gateway, log: log_for_core, registry, prompts, sessions: HashMap::new(), next_id: 1 })
        })();
        if let Err(e) = &outcome {
            log.error("core::new", &format!("装配失败：{}", e)); // 仅错误时借用，不与闭包 move 冲突
        }
        outcome
    }

    /// 清单即事实：每次调用重扫（策略在 core，机制在 ModuleSource）。
    pub fn scan(&self) -> module::Roster {
        self.source.scan()
    }

    // ---- 登记处操作端口（密钥在此层进出；前端只见 id） ----

    pub fn provider_list(&self) -> Vec<String> {
        self.registry.display_lines()
    }

    /// 结构化供应商视图（不含密钥；Web 用）。
    pub fn provider_views(&self) -> Vec<providers::ProviderView> {
        self.registry.view()
    }

    /// 当前全局默认供应商 id。
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
        let r = self.store.save(&self.registry);
        if let Err(e) = &r {
            self.log.error("core::provider_upsert", &format!("登记处持久化失败：{}", e));
        }
        r
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

    // ---- 会话工厂（会话注册进中心，前端只持 id） ----

    /// 模式一：单模块直连。返回 (会话 id, 开场事件)。
    pub fn start_direct(&mut self, id: &str) -> Result<(SessionId, Vec<SessionEvent>), String> {
        let m = self.find(id)?;
        let provider = self
            .registry
            .resolve(m.selected_provider.as_deref(), m.manifest.model.provider.as_deref())
            .map(|(_, p)| p);
        let (chat, note) = self.gateway.member_channel(provider, id);
        self.log.info("core::start_direct", &format!("直连会话：模块 {}，供应商 {}", id, provider.map(|p| p.base_url.as_str()).unwrap_or("无（演示）")));
        let s = session::DirectSession::new(id, m.system_block(&self.prompts), chat, note);
        let opened = s.open();
        let sid = self.put(Session::Direct(s));
        Ok((sid, opened))
    }

    /// 模式三：全能（ids 空 = 全部模块；拼装职责提示词）。返回 (会话 id, 开场事件)。
    pub fn start_omni(&mut self, ids: &str) -> Result<(SessionId, Vec<SessionEvent>), String> {
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
        self.log.info("core::start_omni", &format!("全能会话：模块数 {}，供应商 {}", chosen.len(), provider.map(|p| p.base_url.as_str()).unwrap_or("无（演示）")));
        let s = session::OmniSession::new(merged, chat, note);
        let opened = s.open();
        let sid = self.put(Session::Omni(s));
        Ok((sid, opened))
    }

    /// 模式二：多模块协作（ids = 点名名单，或 "?" 委托代拟）。返回会话 id。
    pub fn start_collab(&mut self, ids: &str) -> Result<SessionId, String> {
        let outcome = CollabSession::start(
            Arc::clone(&self.gateway),
            Arc::clone(&self.source),
            &self.registry,
            self.prompts.clone(),
            ids,
        );
        match &outcome {
            Ok(_) => self.log.info("core::start_collab", &format!("协作会话：名单 {}", ids)),
            Err(e) => self.log.error("core::start_collab", &format!("协作会话创建失败（名单 {}）：{}", ids, e)),
        }
        Ok(self.put(Session::Collab(outcome?)))
    }

    // ---- 会话中心：按 id 收发（前端永不接触会话本体） ----

    /// 直连发言。
    pub fn direct_say(&mut self, sid: SessionId, text: &str) -> Result<Vec<SessionEvent>, String> {
        match self.sessions.get_mut(&sid) {
            Some(Session::Direct(d)) => Ok(vec![d.say(text)]),
            Some(_) => Err("该会话不是直连模式".to_string()),
            None => Err("无此会话".to_string()),
        }
    }

    /// 全能发言。
    pub fn omni_say(&mut self, sid: SessionId, text: &str) -> Result<Vec<SessionEvent>, String> {
        match self.sessions.get_mut(&sid) {
            Some(Session::Omni(o)) => Ok(vec![o.say(text)]),
            Some(_) => Err("该会话不是全能模式".to_string()),
            None => Err("无此会话".to_string()),
        }
    }

    /// 协作推进一步：由前端按 pending 驱动；返回期间产生的全部事件。
    pub fn collab_continue(&mut self, sid: SessionId, step: CollabStep, text: &str) -> Result<Vec<SessionEvent>, String> {
        let mut out = Vec::new();
        {
            let s = self.sessions.get_mut(&sid).ok_or("无此会话")?;
            let collab = match s {
                Session::Collab(c) => c,
                _ => return Err("该会话不是协作模式".to_string()),
            };
            match step {
                CollabStep::SetTask => collab.set_task(text, &mut |e| out.push(e)),
                CollabStep::ConfirmSlate => collab.confirm_slate(text.eq_ignore_ascii_case("yes"), &mut |e| out.push(e)),
                CollabStep::Begin => collab.begin(text.contains("allow"), &mut |e| out.push(e)),
                CollabStep::Answer => collab.answer(text, &mut |e| out.push(e)),
            }
        }
        // 会话终结后移出中心（前端据 Ended 回收）。
        if let Some(Session::Collab(c)) = self.sessions.get(&sid) {
            if c.is_done() {
                self.sessions.remove(&sid);
            }
        }
        Ok(out)
    }

    /// 协作会话当前介入请求（None = 无挂起或已终结）。
    pub fn collab_pending(&self, sid: SessionId) -> Result<Option<Pending>, String> {
        match self.sessions.get(&sid) {
            Some(Session::Collab(c)) => Ok(c.pending.clone()),
            Some(_) => Err("该会话不是协作模式".to_string()),
            None => Err("无此会话".to_string()),
        }
    }

    fn put(&mut self, s: Session) -> SessionId {
        let id = self.next_id;
        self.next_id += 1;
        self.sessions.insert(id, s);
        id
    }

    fn find(&self, id: &str) -> Result<Module, String> {
        self.scan()
            .modules
            .into_iter()
            .find(|m| m.manifest.id == id)
            .ok_or_else(|| format!("无此模块：{}", id))
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
// 测试访问器：验证职责提示词已种入历史首条（回归：直连/全能曾丢失 system 提示词）。
#[cfg(test)]
impl Core {
    pub fn direct_history(&self, sid: SessionId) -> Option<Vec<crate::core::ports::Msg>> {
        match self.sessions.get(&sid) {
            Some(Session::Direct(d)) => Some(d.history().to_vec()),
            _ => None,
        }
    }
    pub fn omni_history(&self, sid: SessionId) -> Option<Vec<crate::core::ports::Msg>> {
        match self.sessions.get(&sid) {
            Some(Session::Omni(o)) => Some(o.history().to_vec()),
            _ => None,
        }
    }
}