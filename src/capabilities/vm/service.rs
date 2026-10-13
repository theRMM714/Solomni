//! 目的：vm 能力的用例与端口持有者。
//! 管：持 PackageSource / VmEngine / ShareBackend（R12），按租约交出运行包库与 guest 生命周期。
//! 不管：纯逻辑（domain/）；适配器（detail/）由组合根构造。
//! 联动：端口在 ports.rs；被 api::Vm 的调用方消费。

use crate::capabilities::vm::api::{Library, Vm};
use crate::capabilities::vm::domain::guest::{GuestRun, GuestSpec};
use crate::capabilities::vm::ports::{GuestVm, PackageSource, ShareBackend, VmEngine};
use crate::kernel::api::ToolOutcome;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// 目的：虚拟机档能力——持运行包库来源、VM 引擎与共享后端，按会话租约管理 guest。
pub struct VmService {
    packages: Arc<dyn PackageSource + Send + Sync>,
    engine: Arc<dyn VmEngine + Send + Sync>,
    share: Arc<dyn ShareBackend + Send + Sync>,
    guests: Mutex<BTreeMap<String, Box<dyn GuestVm>>>,
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
        let plan = self.share.plan(self.engine.id(), &spec.mounts)?;
        let mut guests = self
            .guests
            .lock()
            .map_err(|_| "guest 表锁坏了".to_string())?;
        if let Some(mut old) = guests.remove(&spec.lease) {
            old.stop();
        }
        let started = self.engine.boot(spec, &plan)?;
        guests.insert(spec.lease.clone(), started);
        Ok(())
    }

    fn guest_run(&self, lease: &str, run: &GuestRun) -> ToolOutcome {
        let mut guests = match self.guests.lock() {
            Ok(g) => g,
            Err(_) => return fail("guest 表锁坏了"),
        };
        match guests.get_mut(lease) {
            Some(g) => g.channel().exec(run),
            None => fail(&format!("这次会话没有活着的 guest（租约 {}）", lease)),
        }
    }

    fn guest_stop(&self, lease: &str) {
        if let Ok(mut guests) = self.guests.lock() {
            if let Some(mut g) = guests.remove(lease) {
                g.stop();
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
