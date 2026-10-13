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

/// 目的：一个租约的活体——guest、共享凭据与它要同步的挂载清单。
struct Lease {
    vm: Box<dyn GuestVm>,
    guard: Box<dyn ShareGuard>,
    mounts: Vec<GuestMount>,
    /// 这份 guest 是什么 OS：命令路径按它译成 guest 拼法。
    os: GuestOs,
}

/// 目的：虚拟机档能力——持运行包库来源、VM 引擎与共享后端，按会话租约管理 guest。
pub struct VmService {
    packages: Arc<dyn PackageSource + Send + Sync>,
    engine: Arc<dyn VmEngine + Send + Sync>,
    share: Arc<dyn ShareBackend + Send + Sync>,
    guests: Mutex<BTreeMap<String, Lease>>,
}

impl VmService {
    /// 目的：组合根专用。
    pub fn new(
        packages: Arc<dyn PackageSource + Send + Sync>,
        engine: Arc<dyn VmEngine + Send + Sync>,
        share: Arc<dyn ShareBackend + Send + Sync>,
    ) -> VmService {
        VmService {
            packages,
            engine,
            share,
            guests: Mutex::new(BTreeMap::new()),
        }
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
        if !self.share.supports(self.engine.id(), &spec.image) {
            return Err(format!(
                "共享后端 {} 落不到引擎 {} 上",
                self.share.id(),
                self.engine.id()
            ));
        }
        let (plan, guard) = self.share.prepare(self.engine.id(), &spec.mounts)?;
        let mut guests = self
            .guests
            .lock()
            .map_err(|_| "guest 表锁坏了".to_string())?;
        if let Some(mut old) = guests.remove(&spec.lease) {
            old.vm.stop();
            old.guard.release();
        }
        let vm = self.engine.boot(spec, &plan)?;
        guests.insert(
            spec.lease.clone(),
            Lease {
                vm,
                guard,
                mounts: spec.mounts.clone(),
                os: spec.image.os,
            },
        );
        Ok(())
    }

    fn guest_run(&self, lease: &str, run: &GuestRun) -> ToolOutcome {
        let mut guests = match self.guests.lock() {
            Ok(g) => g,
            Err(_) => return fail("guest 表锁坏了"),
        };
        let Some(entry) = guests.get_mut(lease) else {
            return fail(&format!("这次会话没有活着的 guest（租约 {}）", lease));
        };
        let translated = crate::capabilities::vm::domain::guest::translate_run(run, entry.os);
        if let Err(why) = self.share.sync_in(entry.vm.channel(), &entry.mounts) {
            return fail(&format!("共享同步（进去）失败：{}", why));
        }
        let out = entry.vm.channel().exec(&translated);
        if let Err(why) = self.share.sync_out(entry.vm.channel(), &entry.mounts) {
            return fail(&format!("共享同步（回来）失败：{}", why));
        }
        out
    }

    fn guest_stop(&self, lease: &str) {
        if let Ok(mut guests) = self.guests.lock() {
            if let Some(mut entry) = guests.remove(lease) {
                entry.vm.stop();
                entry.guard.release();
            }
        }
    }
}

fn fail(why: &str) -> ToolOutcome {
    ToolOutcome {
        ok: false,
        output: why.to_string(),
    }
}
