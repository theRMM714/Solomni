//! 目的：内存登记处替身——预置供应商、模型与核心默认模型，让会话都拿到真实通道。
//! 管：设置的读写与保存失败注入；供应商与模型的预置。
//! 不管：历史（`history.rs`）、工作区（`workspace.rs`）。
//! 联动：`src/capabilities/registry/ports.rs` 的 `SettingsStore`。

use super::*;

/// 目的：内存登记处：预置「供应商 p + 模型 m + 核心默认 m」，让会话都能拿到真实通道。
pub(crate) struct InMemorySettings {
    pub(crate) s: Mutex<Settings>,
    pub(crate) fail: Option<String>,
}

impl InMemorySettings {
    pub(crate) fn new() -> InMemorySettings {
        let mut s = Settings::default();
        s.providers.insert(
            "p".to_string(),
            Provider {
                base_url: "http://test".to_string(),
                api_key: "k".to_string(),
            },
        );
        s.models.insert(
            "m".to_string(),
            ModelEntry {
                name: "M".to_string(),
                api_model: "m".to_string(),
                provider: "p".to_string(),
                note: String::new(),
                tools: crate::capabilities::llm::api::ToolMode::Envelope,
                context: 32_000,
                thinking: Default::default(),
            },
        );
        s.core = Some("m".to_string());
        InMemorySettings {
            s: Mutex::new(s),
            fail: None,
        }
    }

    /// 目的：注入失败：load / save 一律返回该原因（端口契约测试用）。
    pub(crate) fn fail_with(mut self, msg: &str) -> InMemorySettings {
        self.fail = Some(msg.to_string());
        self
    }
    /// 目的：指定登记的外部工具路径（断言"显式登记优先"）。
    pub(crate) fn with_qemu_path(qemu: &str) -> InMemorySettings {
        let s = InMemorySettings::new();
        s.s.lock().expect("锁").app.qemu_path = qemu.to_string();
        s
    }

    /// 目的：指定默认执行档位的登记处（断言虚拟机档下的诊断与工具回执）。
    pub(crate) fn with_tier(tier: Tier) -> InMemorySettings {
        let s = InMemorySettings::new();
        s.s.lock().expect("锁").app.tier = tier;
        s
    }

    /// 目的：指定全局流式与调用预算的登记处（断言"设置是流式的上限、预算全局通用"）。
    pub(crate) fn with_llm(streaming: bool, timeout_secs: u64) -> InMemorySettings {
        let s = InMemorySettings::new();
        {
            let mut g = s.s.lock().expect("锁");
            g.app.streaming = streaming;
            g.app.llm_timeout_secs = timeout_secs;
        }
        s
    }

    /// 目的：一个模型都没登记（建会话必须被拒）。
    pub(crate) fn without_models() -> InMemorySettings {
        let s = InMemorySettings::new();
        {
            let mut g = s.s.lock().expect("锁");
            g.models.clear();
            g.core = None;
        }
        s
    }

    /// 目的：模型在册但核心默认没设（无显式模型的 agent / 代理 / 协作必须被拒）。
    pub(crate) fn without_core() -> InMemorySettings {
        let s = InMemorySettings::new();
        s.s.lock().expect("锁").core = None;
        s
    }
}

impl SettingsStore for InMemorySettings {
    fn load(&self) -> Result<Settings, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        Ok(self.s.lock().expect("锁").clone())
    }
    fn save(&self, s: &Settings) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        *self.s.lock().expect("锁") = s.clone();
        Ok(())
    }
}

/// 目的：内存工作区：准备/写入都不碰盘，只记录（测试用）。
#[derive(Default)]
pub(crate) struct InMemoryWorkspace {
    pub(crate) files: Mutex<BTreeMap<String, Vec<u8>>>,
    pub(crate) fail: Option<String>,
}
