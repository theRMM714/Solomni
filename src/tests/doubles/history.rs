//! 目的：内存会话历史替身与模型目录替身——供用例断言落盘与回放。
//! 管：会话元数据、事件追加与替换、列出与删除；模型名回放。
//! 不管：登记处（`registry.rs`）、提示词来源（`chat.rs`）。
//! 联动：`src/capabilities/session/ports.rs` 的 `HistoryStore` 与 `src/capabilities/llm/ports.rs`。

use super::*;

impl InMemoryHistory {
    pub(crate) fn new() -> InMemoryHistory {
        InMemoryHistory {
            metas: Mutex::new(BTreeMap::new()),
            events: Mutex::new(BTreeMap::new()),
            fail: None,
        }
    }

    /// 目的：注入失败：全部 HistoryStore 方法一律返回该原因（端口契约测试用）。
    pub(crate) fn fail_with(mut self, msg: &str) -> InMemoryHistory {
        self.fail = Some(msg.to_string());
        self
    }

    /// 目的：直接改掉某条会话已落盘的 meta（测试夹具）：用来构造"落盘档位与当前判据不一致"的情形。例如虚拟机档现在一律不可选，但**已存在的**虚拟机档会话必须还能打开（记录是用户的）。
    pub(crate) fn force_tier(&self, name: &str, tier: crate::kernel::api::Tier) {
        let mut metas = self.metas.lock().expect("锁");
        if let Some(m) = metas.get_mut(name) {
            m.exec.tier = tier;
        }
    }

    /// 目的：失败注入的入口判定：Ok = 未注入。
    fn guard(&self) -> Result<(), String> {
        match &self.fail {
            Some(m) => Err(m.clone()),
            None => Ok(()),
        }
    }
}

impl HistoryStore for InMemoryHistory {
    fn create(&self, meta: &SessionMeta) -> Result<(), String> {
        self.guard()?;
        self.metas
            .lock()
            .expect("锁")
            .insert(meta.name.clone(), meta.clone());
        Ok(())
    }
    fn save_meta(&self, meta: &SessionMeta) -> Result<(), String> {
        self.guard()?;
        self.metas
            .lock()
            .expect("锁")
            .insert(meta.name.clone(), meta.clone());
        Ok(())
    }
    fn append(&self, name: &str, events: &[serde_json::Value]) -> Result<(), String> {
        self.guard()?;
        self.events
            .lock()
            .expect("锁")
            .entry(name.to_string())
            .or_default()
            .extend_from_slice(events);
        Ok(())
    }
    fn replace(&self, name: &str, events: &[serde_json::Value]) -> Result<(), String> {
        self.guard()?;
        self.events
            .lock()
            .expect("锁")
            .insert(name.to_string(), events.to_vec());
        Ok(())
    }
    fn list(&self) -> Result<Vec<HistoryView>, String> {
        self.guard()?;
        let metas = self.metas.lock().expect("锁");
        let events = self.events.lock().expect("锁");
        Ok(metas
            .values()
            .map(|m| HistoryView {
                name: m.name.clone(),
                mode: m.mode.clone(),
                ts: m.ts,
                done: events
                    .get(&m.name)
                    .map(|v| {
                        v.iter()
                            .any(|e| e.get("type").and_then(|t| t.as_str()) == Some("ended"))
                    })
                    .unwrap_or(false),
                exec: m.exec.clone(),
                parent: m.parent.clone(),
                workspace: m
                    .workspace
                    .as_ref()
                    .map(|binding| binding.root.display().to_string()),
                workspace_missing: m
                    .workspace
                    .as_ref()
                    .map(|binding| !binding.root.is_dir())
                    .unwrap_or(false),
                run: m.run,
            })
            .collect())
    }
    fn meta(&self, name: &str) -> Result<SessionMeta, String> {
        self.guard()?;
        self.metas
            .lock()
            .expect("锁")
            .get(name)
            .cloned()
            .ok_or_else(|| format!("无此会话：{}", name))
    }
    fn load(&self, name: &str) -> Result<(SessionMeta, Vec<serde_json::Value>), String> {
        self.guard()?;
        let meta = self
            .metas
            .lock()
            .expect("锁")
            .get(name)
            .cloned()
            .ok_or_else(|| format!("无此会话：{}", name))?;
        let events = self
            .events
            .lock()
            .expect("锁")
            .get(name)
            .cloned()
            .unwrap_or_default();
        Ok((meta, events))
    }
    fn delete(&self, name: &str) -> Result<bool, String> {
        self.guard()?;
        let removed = self.metas.lock().expect("锁").remove(name).is_some();
        self.events.lock().expect("锁").remove(name);
        Ok(removed)
    }
}

/// 目的：内存模型目录：回放固定模型名，并记录收到的 Provider（断言编辑期密钥复用）。
pub(crate) struct FakeCatalog {
    pub(crate) models: Vec<String>,
    pub(crate) seen: Mutex<Vec<String>>,
    pub(crate) fail: Option<String>,
}

impl FakeCatalog {
    pub(crate) fn new(models: Vec<String>) -> FakeCatalog {
        FakeCatalog {
            models,
            seen: Mutex::new(Vec::new()),
            fail: None,
        }
    }

    /// 目的：注入失败：list_models 返回该原因（失败注入不记录调用——错误路径没走到"用哪个通道"）。
    pub(crate) fn fail_with(mut self, msg: &str) -> FakeCatalog {
        self.fail = Some(msg.to_string());
        self
    }
}

impl ModelCatalog for FakeCatalog {
    fn list_models(&self, base_url: &str, _api_key: &str) -> Result<Vec<String>, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.seen.lock().expect("锁").push(base_url.to_string());
        Ok(self.models.clone())
    }
}

pub(crate) struct VecSource(pub(crate) Vec<Module>);

impl ModuleSource for VecSource {
    fn scan(&self) -> crate::capabilities::modules::api::Roster {
        crate::capabilities::modules::api::Roster {
            modules: self.0.clone(),
            rejected: Vec::new(),
        }
    }
}

/// 目的：无声围栏端口：测试里不碰任何 ACL（真实实现在 src/kernel/detail/confine）。
pub(crate) struct NoFenceHost;
impl crate::kernel::ports::FenceHost for NoFenceHost {
    fn release_call(&self, _spec: &crate::kernel::api::FenceSpec) -> Result<(), String> {
        Ok(())
    }
}
