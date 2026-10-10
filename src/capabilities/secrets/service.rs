//! 目的：隐秘字段的存储与解析——统一管理 API 的实现。
//! 管：值缓存与落盘、按模块解析注入项、按已知值脱敏；只认 SecretStore 端口与 Workspace 的清单事实。
//! 不管：实际注入（消费方按 resolve 取）；通道密钥（registry）；字段声明解析（workspace）。
//! 联动：api 见 `api.rs`，端口见 `ports.rs`；由 `main.rs` 装配。
#![allow(dead_code)] // declared 已由 CLI 消费；set/clear/resolve/redact 随设置面与起进程时的 env 注入接入。

use crate::capabilities::secrets::api::{SecretOps, SecretView};
use crate::capabilities::secrets::ports::SecretStore;
use crate::capabilities::workspace::api::{Roster, SecretDecl, Workspace};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// 目的：定位键——模块 id 与字段名的组合。
fn key(module: &str, name: &str) -> String {
    format!("{}/{}", module, name)
}

/// 目的：隐秘字段能力——持值缓存与存储端口，按统一 API 答话。
pub struct SecretsService {
    workspace: Arc<dyn Workspace + Send + Sync>,
    store: Arc<dyn SecretStore + Send + Sync>,
    values: Mutex<BTreeMap<String, String>>,
}

impl SecretsService {
    /// 目的：组合根专用——注入工作区清单来源与存储端口；加载失败 = 装配失败（不静默空表）。
    pub fn new(
        workspace: Arc<dyn Workspace + Send + Sync>,
        store: Arc<dyn SecretStore + Send + Sync>,
    ) -> Result<SecretsService, String> {
        let values = store.load()?;
        Ok(SecretsService {
            workspace,
            store,
            values: Mutex::new(values),
        })
    }

    /// 目的：字段必须已被某个模块声明；缺失时如实报错。
    fn locate<'a>(roster: &'a Roster, module: &str, name: &str) -> Result<&'a SecretDecl, String> {
        let m = roster
            .modules
            .iter()
            .find(|m| m.manifest.id == module)
            .ok_or_else(|| format!("清单里没有模块 {}", module))?;
        m.manifest
            .secrets
            .get(name)
            .ok_or_else(|| format!("模块 {} 没有声明隐秘字段 {}", module, name))
    }

    /// 目的：把缓存里该模块的已配置值取一份快照（脱敏与解析共用）。
    fn module_values(&self, module: &str, roster: &Roster) -> Vec<(String, String)> {
        let values = self.values.lock().expect("锁");
        let mut out = Vec::new();
        if let Some(m) = roster.modules.iter().find(|m| m.manifest.id == module) {
            for (name, decl) in &m.manifest.secrets {
                if let Some(v) = values.get(&key(module, name)) {
                    if !v.is_empty() {
                        out.push((decl.env.clone(), v.clone()));
                    }
                }
            }
        }
        out
    }

    /// 目的：写回整表（先改缓存再落盘；落盘失败如实报错）。
    fn persist(&self) -> Result<(), String> {
        let snapshot = self.values.lock().expect("锁").clone();
        self.store.save(&snapshot)
    }
}

impl SecretOps for SecretsService {
    fn declared(&self) -> Result<Vec<SecretView>, String> {
        let roster = self.workspace.roster();
        let values = self.values.lock().expect("锁");
        let mut out = Vec::new();
        for m in &roster.modules {
            for (name, decl) in &m.manifest.secrets {
                out.push(SecretView {
                    module: m.manifest.id.clone(),
                    name: name.clone(),
                    env: decl.env.clone(),
                    desc: decl.desc.clone(),
                    configured: values.contains_key(&key(&m.manifest.id, name)),
                });
            }
        }
        Ok(out)
    }

    fn set(&self, module: &str, name: &str, value: &str) -> Result<(), String> {
        let roster = self.workspace.roster();
        Self::locate(&roster, module, name)?;
        self.values
            .lock()
            .expect("锁")
            .insert(key(module, name), value.to_string());
        self.persist()
    }

    fn clear(&self, module: &str, name: &str) -> Result<(), String> {
        let roster = self.workspace.roster();
        Self::locate(&roster, module, name)?;
        self.values.lock().expect("锁").remove(&key(module, name));
        self.persist()
    }

    fn resolve(&self, module: &str) -> Result<Vec<(String, String)>, String> {
        let roster = self.workspace.roster();
        if !roster.modules.iter().any(|m| m.manifest.id == module) {
            return Err(format!("清单里没有模块 {}", module));
        }
        Ok(self.module_values(module, &roster))
    }

    fn redact(&self, module: &str, text: &str) -> Result<String, String> {
        let roster = self.workspace.roster();
        let mut out = text.to_string();
        for (_env, value) in self.module_values(module, &roster) {
            out = out.replace(&value, "***");
        }
        Ok(out)
    }
}
