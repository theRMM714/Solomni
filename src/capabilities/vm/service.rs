//! 目的：vm 能力的用例与端口持有者。
//! 管：持 PackageSource / VmEngine / ShareBackend（R12），按租约交出运行包库与 guest 生命周期
//!   （guest 与共享凭据同生共死；每次执行前后走共享的同步钩子）。
//! 不管：纯逻辑（domain/）；适配器（detail/）由组合根构造。
//! 联动：端口在 ports.rs；被 api::Vm 的调用方消费。

use crate::capabilities::vm::api::{Library, Vm};
use crate::capabilities::vm::domain::guest::{GuestMount, GuestOs, GuestRun, GuestSpec};
use crate::capabilities::vm::ports::{GuestVm, PackageSource, ShareBackend, ShareGuard, VmEngine};
use crate::kernel::api::ToolOutcome;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// 目的：一个租约的活体——guest、共享后端与它的凭据、它要同步的挂载清单。
struct Lease {
    vm: Box<dyn GuestVm>,
    /// 这**一台** guest 实际用的共享后端：同步走它（不同租约可以是不同的档）。
    share: Arc<dyn ShareBackend + Send + Sync>,
    guard: Box<dyn ShareGuard>,
    mounts: Vec<GuestMount>,
    /// 启动时实际装载的运行包（id@version）：新子会话据此判断要不要重建。
    packages: Vec<String>,
    /// 这份 guest 是什么 OS：命令路径按它译成 guest 拼法。
    os: GuestOs,
}

/// 目的：虚拟机档能力——持运行包库来源、VM 引擎与**已实现的全部共享后端**，按会话租约管理 guest。
pub struct VmService {
    packages: Arc<dyn PackageSource + Send + Sync>,
    engine: Arc<dyn VmEngine + Send + Sync>,
    /// 已实现的共享后端（按优先序持有）：起 guest 时按 GuestSpec.shares 挑第一个落得上的。
    shares: Vec<Arc<dyn ShareBackend + Send + Sync>>,
    /// 租约 → 该租约的活体。每个活体自己一把锁：表锁只用来取/插，执行在**该租约自己的锁**下跑。
    guests: Mutex<BTreeMap<String, Arc<Mutex<Lease>>>>,
}

impl VmService {
    /// 目的：组合根专用——把已实现的共享后端都交进来（选哪一个不在这里定）。
    pub fn new(
        packages: Arc<dyn PackageSource + Send + Sync>,
        engine: Arc<dyn VmEngine + Send + Sync>,
        shares: Vec<Arc<dyn ShareBackend + Send + Sync>>,
    ) -> VmService {
        VmService {
            packages,
            engine,
            shares,
            guests: Mutex::new(BTreeMap::new()),
        }
    }

    /// 目的：这次启动的共享后端候选：GuestSpec.shares 有就按它（按 id 认），没有就按本 service 持有的用户档。
    /// 约束：候选只收本 service 真持有的那些——传进来的 id 认不出就跳过（不 new 别人的适配器）。
    /// 约束：空候选只返回**非 emergency** 档；copy 只有在 GuestSpec.shares 显式写出时才可能被选中。
    fn candidates(&self, want: &[&'static str]) -> Vec<Arc<dyn ShareBackend + Send + Sync>> {
        if want.is_empty() {
            return self
                .shares
                .iter()
                .filter(|s| !s.is_emergency())
                .cloned()
                .collect();
        }
        want.iter()
            .filter_map(|id| self.shares.iter().find(|s| s.id() == *id).cloned())
            .collect()
    }
}

impl VmService {
    /// 目的：一次 guest 执行（可带 per-agent 围栏）——共享同步与命令通道在同一把租约锁下。
    fn run_guest(
        &self,
        lease: &str,
        run: &GuestRun,
        fence: Option<&crate::kernel::api::FenceSpec>,
    ) -> ToolOutcome {
        let entry = match self.guests.lock() {
            Ok(guests) => guests.get(lease).cloned(),
            Err(_) => return fail("guest 表锁坏了"),
        };
        let Some(entry) = entry else {
            return fail(&format!("这次会话没有活着的 guest（租约 {}）", lease));
        };
        // 全局表锁只用来取 entry；执行在该租约自己的锁下跑——跨租约因此真并行（同一租约仍串行）。
        let mut held = entry.lock().unwrap_or_else(|e| e.into_inner());
        let held = &mut *held;
        let translated =
            crate::capabilities::vm::domain::guest::translate_run(run, held.os, &held.mounts);
        if let Err(why) = held.share.sync_in(held.vm.channel(), &held.mounts) {
            return fail(&format!("共享同步（进去）失败：{}", why));
        }
        let out = match fence {
            Some(fence) => {
                let access = crate::capabilities::vm::domain::guest::GuestAccess::from_fence(
                    fence,
                    held.os,
                    &held.mounts,
                );
                held.vm.channel().exec_fenced(&translated, &access)
            }
            None => held.vm.channel().exec(&translated),
        };
        if let Err(why) = held.share.sync_out(held.vm.channel(), &held.mounts) {
            return fail(&format!("共享同步（回来）失败：{}", why));
        }
        out
    }
}

impl Vm for VmService {
    fn library(&self) -> Library {
        self.packages.scan()
    }

    fn runtimes_dir(&self) -> std::path::PathBuf {
        self.packages.dir()
    }

    fn guest_start(&self, spec: &GuestSpec) -> Result<(), String> {
        if !self.engine.supports(&spec.image) {
            return Err(format!("引擎 {} 承载不了这份 guest 镜像", self.engine.id()));
        }
        // 选档在**这一刻**做：候选里第一个落得到这台引擎 + 这份 guest 上的就用它。
        // 因此同一进程里不同租约可以用不同的档，补齐作业装完共享工具后下一次启动就用上（不必重启）。
        let candidates = self.candidates(&spec.shares);
        let mut tried: Vec<&'static str> = Vec::new();
        let mut chosen: Option<Arc<dyn ShareBackend + Send + Sync>> = None;
        for share in &candidates {
            tried.push(share.id());
            if share.supports(self.engine.id(), &spec.image) {
                chosen = Some(Arc::clone(share));
                break;
            }
        }
        let Some(share) = chosen else {
            return Err(format!(
                "这份 guest 落不到引擎 {} 的任何共享后端上（试过：{}）",
                self.engine.id(),
                if tried.is_empty() {
                    "没有候选".to_string()
                } else {
                    tried.join("、")
                }
            ));
        };
        let (plan, guard) =
            share.prepare(self.engine.id(), &spec.mounts, spec.share_path.as_deref())?;
        // 拉起放在表锁之外：boot 要等 agent 起来（有界但可能要几十秒），别把别的租约的执行挡在表锁后面。
        let vm = self.engine.boot(spec, &plan)?;
        let entry = Arc::new(Mutex::new(Lease {
            vm,
            share,
            guard,
            mounts: spec.mounts.clone(),
            packages: Vec::new(),
            os: spec.image.os,
        }));
        let previous = self
            .guests
            .lock()
            .map_err(|_| "guest 表锁坏了".to_string())?
            .insert(spec.lease.clone(), entry);
        // 同一租约重启：等它的在途执行收尾，再销毁旧的（幂等）。
        if let Some(old) = previous {
            stop_lease(&old);
        }
        Ok(())
    }

    fn guest_run(&self, lease: &str, run: &GuestRun) -> ToolOutcome {
        self.run_guest(lease, run, None)
    }

    fn guest_run_fenced(
        &self,
        lease: &str,
        run: &GuestRun,
        fence: &crate::kernel::api::FenceSpec,
    ) -> ToolOutcome {
        self.run_guest(lease, run, Some(fence))
    }

    fn guest_packages(&self, lease: &str) -> Option<Vec<String>> {
        let entry = self.guests.lock().ok()?.get(lease).cloned()?;
        let held = entry.lock().unwrap_or_else(|e| e.into_inner());
        Some(held.packages.clone())
    }

    fn guest_set_packages(&self, lease: &str, packages: Vec<String>) {
        let Some(entry) = self.guests.lock().ok().and_then(|g| g.get(lease).cloned()) else {
            return;
        };
        let mut held = entry.lock().unwrap_or_else(|e| e.into_inner());
        held.packages = packages;
    }

    fn guest_wait_agent(&self, lease: &str, timeout: std::time::Duration) -> bool {
        let entry = match self.guests.lock() {
            Ok(guests) => guests.get(lease).cloned(),
            Err(_) => return false,
        };
        let Some(entry) = entry else {
            return false;
        };
        let deadline = std::time::Instant::now() + timeout;
        loop {
            // 每探一次短拿一次租约锁：探针不长时间挡住同一租约的其它调用。
            {
                let mut held = entry.lock().unwrap_or_else(|e| e.into_inner());
                if held.vm.channel().ping() {
                    return true;
                }
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    }

    fn guest_stop(&self, lease: &str) {
        let entry = match self.guests.lock() {
            Ok(mut guests) => guests.remove(lease),
            Err(_) => None,
        };
        if let Some(entry) = entry {
            stop_lease(&entry);
        }
    }

    fn guest_leases(&self) -> Vec<String> {
        self.guests
            .lock()
            .map(|g| g.keys().cloned().collect())
            .unwrap_or_default()
    }
}

/// 目的：销毁一份租约（杀 guest + 放共享凭据）；等它的在途执行收尾，幂等。
fn stop_lease(entry: &Arc<Mutex<Lease>>) {
    let mut held = entry.lock().unwrap_or_else(|e| e.into_inner());
    held.vm.stop();
    held.guard.release();
}

fn fail(why: &str) -> ToolOutcome {
    ToolOutcome {
        ok: false,
        output: why.to_string(),
    }
}
