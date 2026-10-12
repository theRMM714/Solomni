//! 目的：vm 能力的用例与端口持有者。
//! 管：持 PackageSource（R12），按用例交出运行包库与包库目录。
//! 不管：纯逻辑（domain/）；装配（new 出适配器）在组合根。
//! 联动：端口在 ports.rs；被 api::Vm 的调用方消费。

use crate::capabilities::vm::api::{Library, Vm};
use crate::capabilities::vm::ports::PackageSource;
use std::sync::Arc;

/// 目的：虚拟机档能力——持运行包库来源，按用例答话。
pub struct VmService {
    packages: Arc<dyn PackageSource + Send + Sync>,
}

impl VmService {
    /// 目的：组合根专用。
    pub fn new(packages: Arc<dyn PackageSource + Send + Sync>) -> VmService {
        VmService { packages }
    }
}

impl Vm for VmService {
    fn library(&self) -> Library {
        self.packages.scan()
    }

    fn runtimes_dir(&self) -> std::path::PathBuf {
        self.packages.dir()
    }
}
