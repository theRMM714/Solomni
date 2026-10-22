//! 目的：常驻服务的注册表与生命周期——统一管理 API 的实现。
//! 管：适配器注册表、运行中的实例、用户开关、租约回收；只认适配器端口与 `Workspace::roster` 的清单事实。
//! 不管：任何协议（全在适配器）；进程与围栏（在 kernel，随真实适配器接入）；
//!   启停开关（归 modules：用户覆盖 ?? 作者默认）；本能力只按有效值跑。
//! 联动：api 见 `api.rs`，端口见 `ports.rs`；由 `main.rs` 装配。

use crate::capabilities::modules::api::{Module, Modules, Roster, ServiceDecl};
use crate::capabilities::residents::api::{
    Operation, Receipt, ResidentOps, ServiceState, ServiceView,
};
use crate::capabilities::residents::ports::{LaunchSpec, ServiceAdapter, ServiceInstance};
use crate::capabilities::secrets::api::SecretOps;
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
    modules: Arc<dyn Modules + Send + Sync>,
    /// 隐秘字段面：起服务时解析注入项，回执出口按已知值脱敏。
    secrets: Arc<dyn SecretOps + Send + Sync>,
    /// 运行中的实例（key = 模块/服务）。
    running: Mutex<BTreeMap<String, Running>>,
    /// 起过、但进程已经结束的服务（key = 模块/服务 → (如实原因, 会话租约)）：
    /// 崩溃检测的落点——如实标失败，不装作还在跑。
    failed: Mutex<BTreeMap<String, (String, String)>>,
    /// 目的：全局网络开关（常驻服务与一次执行同一份口径）。
    network: bool,
}

impl ResidentsService {
    /// 目的：组合根专用——注入适配器注册表与模块能力面（取清单与启停有效值）。
    pub fn new(
        adapters: Vec<Arc<dyn ServiceAdapter + Send + Sync>>,
        modules: Arc<dyn Modules + Send + Sync>,
        secrets: Arc<dyn SecretOps + Send + Sync>,
    ) -> ResidentsService {
        let mut map = BTreeMap::new();
        for a in adapters {
            map.insert(a.id().to_string(), a);
        }
        ResidentsService {
            adapters: map,
            modules,
            secrets,
            running: Mutex::new(BTreeMap::new()),
            failed: Mutex::new(BTreeMap::new()),
            network: false,
        }
    }

    /// 目的：组合根注入全局网络开关（默认 false；重启后按新设置重建）。
    pub fn with_network(mut self, network: bool) -> ResidentsService {
        self.network = network;
        self
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
}

impl ResidentOps for ResidentsService {
    fn services(&self) -> Result<Vec<ServiceView>, String> {
        let roster = self.modules.roster();
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
                let failed = self.failed.lock().expect("锁");
                let (state, operations) = if let Some(run) = running.get(&k) {
                    (ServiceState::Ready, run.operations.clone())
                } else if let Some((why, _)) = failed.get(&k) {
                    (ServiceState::Failed(why.clone()), Vec::new())
                } else if !self.modules.service_enabled(&m.manifest.id, name) {
                    (ServiceState::Disabled, Vec::new())
                } else {
                    (ServiceState::Stopped, Vec::new())
                };
                drop(failed);
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
        let roster = self.modules.roster();
        let (m, decl) = Self::locate(&roster, module, name)?;
        let k = key(module, name);
        if !self.modules.service_enabled(module, name) {
            return Err(format!("服务 {} 未启用：先启用再启动", k));
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
        // 服务围栏：与"人直接跑模块工具"同一条口径（模块根递归只读 + userdata 可写），并挂上会话租约。
        let mut fence = crate::kernel::api::FenceSpec::standalone(&m.root, None, m.has_userdata);
        fence.net = self.network;
        fence.lease = lease.to_string();
        fence.agent = format!("resident/{}/{}", module, name);
        let spec = LaunchSpec {
            module: module.to_string(),
            name: name.to_string(),
            command: decl.command.clone(),
            cwd: m.root.clone(),
            options: decl.options.clone(),
            env,
            fence,
        };
        let (instance, operations) = adapter.start(&spec)?;
        // 重新启动清掉上一次的失败态。
        self.failed.lock().expect("锁").remove(&k);
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
        self.failed.lock().expect("锁").remove(&k);
        if let Some(mut run) = self.running.lock().expect("锁").remove(&k) {
            run.instance.stop();
        }
        Ok(())
    }

    fn start_enabled(&self) -> Vec<String> {
        let roster = self.modules.roster();
        let mut out = Vec::new();
        for m in &roster.modules {
            for name in m.manifest.services.keys() {
                if !self.modules.service_enabled(&m.manifest.id, name) {
                    continue;
                }
                let k = format!("{}.{}", m.manifest.id, name);
                match self.start(&m.manifest.id, name, "") {
                    Ok(_) => out.push(format!("[常驻服务] 已拉起 {}", k)),
                    Err(e) => out.push(format!("[常驻服务] {} 拉起失败：{}", k, e)),
                }
            }
        }
        out
    }

    fn call(
        &self,
        module: &str,
        name: &str,
        op: &str,
        args: &serde_json::Value,
    ) -> Result<Receipt, String> {
        let k = key(module, name);
        // 可用性先判（**锁外**，与 start 的锁序一致：modules → running）：回执要如实说明为什么不可用，
        // 模型据此学会不再调它（事实教学，不靠会话侧缓存或刷新）。
        if !self.modules.module_enabled(module) {
            return Err(format!("服务 {} 不可用：模块 {} 已停用", k, module));
        }
        if !self.modules.service_enabled(module, name) {
            return Err(format!("服务 {} 不可用：已停用（需先启用）", k));
        }
        let mut running = self.running.lock().expect("锁");
        if !running.contains_key(&k) {
            // 没在跑：区分"之前崩过"（failed 里有如实原因）与"从没起过"。
            let last = self.failed.lock().expect("锁").get(&k).cloned();
            return Err(match last {
                Some((why, _)) => format!("服务 {} 不可用：进程已结束（上次：{}）", k, why),
                None => format!("服务 {} 不可用：未启动", k),
            });
        }
        let (receipt, alive, lease) = {
            let run = running.get_mut(&k).expect("刚查过存在");
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
            // 服务报告操作清单变了（如 MCP tools/list_changed）：就地更新运行态，下一次工具面随之反映。
            if let Some(ops) = run.instance.take_refreshed_operations() {
                run.operations = ops;
            }
            // 崩溃检测：进程已结束就如实标失败，并从运行态摘除（再调用要重新启动）。
            (receipt, run.instance.is_alive(), run.lease.clone())
        };
        if !alive {
            if let Some(mut dead) = running.remove(&k) {
                dead.instance.stop();
            }
            self.failed
                .lock()
                .expect("锁")
                .insert(k.clone(), ("服务进程已结束".to_string(), lease));
        }
        drop(running);
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
        // 失败态也按租约回收：会话没了，它的失败记录不该留到下一次会话。
        self.failed
            .lock()
            .expect("锁")
            .retain(|_, (_, lease_of)| lease_of != lease);
        Ok(())
    }
}
