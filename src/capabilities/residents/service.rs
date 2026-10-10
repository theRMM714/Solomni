//! 目的：常驻服务的注册表与生命周期——统一管理 API 的实现。
//! 管：适配器注册表、运行中的实例、用户开关、租约回收；只认适配器端口与 `Workspace::roster` 的清单事实。
//! 不管：任何协议（全在适配器）；进程与围栏（在 kernel，随真实适配器接入）；
//!   开关的落盘（随设置面接入，当前在内存、缺省取声明里的 enabled）。
//! 联动：api 见 `api.rs`，端口见 `ports.rs`；由 `main.rs` 装配。

use crate::capabilities::residents::api::{
    Operation, Receipt, ResidentOps, ServiceState, ServiceView,
};
use crate::capabilities::residents::ports::{LaunchSpec, ServiceAdapter, ServiceInstance};
use crate::capabilities::secrets::api::SecretOps;
use crate::capabilities::workspace::api::{Module, Roster, ServiceDecl, Workspace};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// 目的：定位键——模块 id 与服务名的组合（两个模块可以声明同名服务）。
fn key(module: &str, name: &str) -> String {
    format!("{}/{}", module, name)
}

/// 一个运行中的实例（连同它的会话租约）。
struct Running {
    instance: Box<dyn ServiceInstance>,
    operations: Vec<Operation>,
    adapter: String,
    lease: String,
}

/// 目的：常驻服务能力——持适配器注册表与运行态，按统一 API 答话。
pub struct ResidentsService {
    adapters: BTreeMap<String, Arc<dyn ServiceAdapter + Send + Sync>>,
    workspace: Arc<dyn Workspace + Send + Sync>,
    /// 隐秘字段面：起服务时解析注入项，回执出口按已知值脱敏。
    secrets: Arc<dyn SecretOps + Send + Sync>,
    /// 用户开关（缺省取声明里的 enabled）；落盘随设置面接入。
    enabled: Mutex<BTreeMap<String, bool>>,
    /// 运行中的实例（key = 模块/服务）。
    running: Mutex<BTreeMap<String, Running>>,
}

impl ResidentsService {
    /// 目的：组合根专用——注入适配器注册表与工作区用例面（只用来取模块清单）。
    pub fn new(
        adapters: Vec<Arc<dyn ServiceAdapter + Send + Sync>>,
        workspace: Arc<dyn Workspace + Send + Sync>,
        secrets: Arc<dyn SecretOps + Send + Sync>,
    ) -> ResidentsService {
        let mut map = BTreeMap::new();
        for a in adapters {
            map.insert(a.id().to_string(), a);
        }
        ResidentsService {
            adapters: map,
            workspace,
            secrets,
            enabled: Mutex::new(BTreeMap::new()),
            running: Mutex::new(BTreeMap::new()),
        }
    }

    /// 目的：适配器名一览（未知适配器时如实列出可用项）。
    fn adapter_ids(&self) -> String {
        self.adapters.keys().cloned().collect::<Vec<_>>().join("、")
    }

    /// 目的：按清单定位模块与服务声明；缺失时如实报错。
    fn locate<'a>(
        roster: &'a Roster,
        module: &str,
        name: &str,
    ) -> Result<(&'a Module, &'a ServiceDecl), String> {
        let m = roster
            .modules
            .iter()
            .find(|m| m.manifest.id == module)
            .ok_or_else(|| format!("清单里没有模块 {}", module))?;
        let d = m
            .manifest
            .services
            .get(name)
            .ok_or_else(|| format!("模块 {} 没有声明服务 {}", module, name))?;
        Ok((m, d))
    }

    /// 目的：这个服务此刻开没开——用户开关优先，缺省取声明。
    fn is_enabled(&self, k: &str, default: bool) -> bool {
        *self.enabled.lock().expect("锁").get(k).unwrap_or(&default)
    }
}

impl ResidentOps for ResidentsService {
    fn services(&self) -> Result<Vec<ServiceView>, String> {
        let roster = self.workspace.roster();
        let running = self.running.lock().expect("锁");
        let mut out = Vec::new();
        for m in &roster.modules {
            for (name, decl) in &m.manifest.services {
                let k = key(&m.manifest.id, name);
                let mut reason = String::new();
                if !self.adapters.contains_key(&decl.adapter) {
                    reason = format!(
                        "未知适配器：{}（可用：{}）",
                        decl.adapter,
                        self.adapter_ids()
                    );
                }
                let (state, operations) = match running.get(&k) {
                    Some(run) => (ServiceState::Ready, run.operations.clone()),
                    None if !self.is_enabled(&k, decl.enabled) => {
                        (ServiceState::Disabled, Vec::new())
                    }
                    None => (ServiceState::Stopped, Vec::new()),
                };
                out.push(ServiceView {
                    module: m.manifest.id.clone(),
                    name: name.clone(),
                    adapter: decl.adapter.clone(),
                    state,
                    operations,
                    reason,
                });
            }
        }
        Ok(out)
    }

    fn start(&self, module: &str, name: &str, lease: &str) -> Result<Vec<Operation>, String> {
        let roster = self.workspace.roster();
        let (m, decl) = Self::locate(&roster, module, name)?;
        let k = key(module, name);
        if !self.is_enabled(&k, decl.enabled) {
            return Err(format!("服务 {} 已禁用：先启用再启动", k));
        }
        {
            let running = self.running.lock().expect("锁");
            if let Some(run) = running.get(&k) {
                return Ok(run.operations.clone());
            }
        }
        let adapter = self.adapters.get(&decl.adapter).ok_or_else(|| {
            format!(
                "未知适配器：{}（可用：{}）",
                decl.adapter,
                self.adapter_ids()
            )
        })?;
        // 该模块已配置的隐秘字段：只经环境变量注入该服务进程（值不进命令行）。
        let env = self.secrets.resolve(module)?;
        let spec = LaunchSpec {
            module: module.to_string(),
            name: name.to_string(),
            command: decl.command.clone(),
            cwd: m.root.clone(),
            options: decl.options.clone(),
            env,
        };
        let (instance, operations) = adapter.start(&spec)?;
        self.running.lock().expect("锁").insert(
            k,
            Running {
                instance,
                operations: operations.clone(),
                adapter: decl.adapter.clone(),
                lease: lease.to_string(),
            },
        );
        Ok(operations)
    }

    fn stop(&self, module: &str, name: &str) -> Result<(), String> {
        let k = key(module, name);
        if let Some(mut run) = self.running.lock().expect("锁").remove(&k) {
            run.instance.stop();
        }
        Ok(())
    }

    fn set_enabled(&self, module: &str, name: &str, enabled: bool) -> Result<(), String> {
        let roster = self.workspace.roster();
        Self::locate(&roster, module, name)?;
        let k = key(module, name);
        self.enabled.lock().expect("锁").insert(k, enabled);
        if !enabled {
            self.stop(module, name)?;
        }
        Ok(())
    }

    fn call(
        &self,
        module: &str,
        name: &str,
        op: &str,
        args: &serde_json::Value,
    ) -> Result<Receipt, String> {
        let k = key(module, name);
        let mut running = self.running.lock().expect("锁");
        let run = running
            .get_mut(&k)
            .ok_or_else(|| format!("服务 {} 没在跑：先启动", k))?;
        if !run.operations.iter().any(|o| o.name == op) {
            let names: Vec<&str> = run.operations.iter().map(|o| o.name.as_str()).collect();
            return Err(format!(
                "服务（适配器 {}）没有操作 {}（可用：{}）",
                run.adapter,
                op,
                names.join("、")
            ));
        }
        let receipt = match run.instance.call(op, args) {
            Ok(output) => Receipt { ok: true, output },
            Err(e) => Receipt {
                ok: false,
                output: e,
            },
        };
        // 回执出口按该模块的已知值脱敏（尽力而为；编码 / 变形挡不住，如实写在文档里）。
        let output = self.secrets.redact(module, &receipt.output)?;
        Ok(Receipt {
            ok: receipt.ok,
            output,
        })
    }

    fn reap(&self, lease: &str) -> Result<(), String> {
        let mut running = self.running.lock().expect("锁");
        let keys: Vec<String> = running
            .iter()
            .filter(|(_, r)| r.lease == lease)
            .map(|(k, _)| k.clone())
            .collect();
        for k in keys {
            if let Some(mut run) = running.remove(&k) {
                run.instance.stop();
            }
        }
        Ok(())
    }
}
