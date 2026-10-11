//! 目的：模块能力的用例与端口持有者。
//! 管：按用例答话（清单、启停有效值、启停写面）；持有清单来源与状态落盘两个端口。
//! 不管：目录遍历机制（在 `detail/fs_modules.rs`）；契约与校验（在 `domain/module.rs`）。
//! 联动：装配（new 出适配器）在组合根；端口只在这里（R12）。

use crate::capabilities::modules::api::{Modules, Roster};
use crate::capabilities::modules::domain::state::EnabledState;
use crate::capabilities::modules::ports::{ModuleSource, ModuleStateStore};
use std::sync::{Arc, Mutex};

/// 目的：模块能力——持清单来源与状态落盘两个端口，按用例答话。
pub struct ModulesService {
    source: Arc<dyn ModuleSource + Send + Sync>,
    store: Arc<dyn ModuleStateStore + Send + Sync>,
    /// 启停覆盖（用户决定）：写只在核心线程（用户动作），读在各处；与 residents 的运行态同口径（内部锁）。
    state: Mutex<EnabledState>,
}

impl ModulesService {
    /// 目的：组合根专用——注入清单来源与状态落盘端口，并载入覆盖。
    pub fn new(
        source: Arc<dyn ModuleSource + Send + Sync>,
        store: Arc<dyn ModuleStateStore + Send + Sync>,
    ) -> Result<ModulesService, String> {
        let state = store.load()?;
        Ok(ModulesService {
            source,
            store,
            state: Mutex::new(state),
        })
    }

    /// 目的：在清单里按 id 找模块。
    fn find(&self, id: &str) -> Option<crate::capabilities::modules::api::Module> {
        self.source
            .scan()
            .modules
            .into_iter()
            .find(|m| m.manifest.id == id)
    }

    /// 目的：模块的**有效**启用（用户覆盖 ?? 作者默认；不存在 = false）。
    fn effective_module(&self, id: &str) -> bool {
        let Some(m) = self.find(id) else {
            return false;
        };
        let state = self.state.lock().expect("锁");
        state
            .modules
            .get(id)
            .and_then(|t| t.enabled)
            .unwrap_or(m.manifest.enabled)
    }

    /// 目的：服务的**有效**启用（模块停用 ⇒ false；覆盖 ?? 作者默认）。
    fn effective_service(&self, module: &str, name: &str) -> bool {
        if !self.module_enabled(module) {
            return false;
        }
        let Some(m) = self.find(module) else {
            return false;
        };
        let Some(decl) = m.manifest.services.get(name) else {
            return false;
        };
        let state = self.state.lock().expect("锁");
        state
            .modules
            .get(module)
            .and_then(|t| t.services.get(name))
            .copied()
            .unwrap_or(decl.enabled)
    }
}

impl Modules for ModulesService {
    fn roster(&self) -> Roster {
        self.source.scan()
    }

    fn module_enabled(&self, id: &str) -> bool {
        self.effective_module(id)
    }

    fn service_enabled(&self, module: &str, service: &str) -> bool {
        self.effective_service(module, service)
    }

    fn set_module_enabled(&self, id: &str, on: bool) -> Result<(), String> {
        if self.find(id).is_none() {
            return Err(format!("清单里没有模块 {}", id));
        }
        let mut state = self.state.lock().expect("锁");
        state.modules.entry(id.to_string()).or_default().enabled = Some(on);
        self.store.save(&state)
    }

    fn set_service_enabled(&self, module: &str, name: &str, on: bool) -> Result<(), String> {
        let Some(m) = self.find(module) else {
            return Err(format!("清单里没有模块 {}", module));
        };
        if !m.manifest.services.contains_key(name) {
            return Err(format!("模块 {} 没有声明服务 {}", module, name));
        }
        let mut state = self.state.lock().expect("锁");
        state
            .modules
            .entry(module.to_string())
            .or_default()
            .services
            .insert(name.to_string(), on);
        self.store.save(&state)
    }
}
