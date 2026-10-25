//! 目的：packages 能力的用例与端口持有者。
//! 管：持 PackageSource（R12），按调用交出包库与依赖文件夹路径。
//! 不管：纯逻辑（domain/）；适配器（detail/）由组合根构造。
//! 联动：端口在 ports.rs；被 api::Packages 的调用方消费。

use crate::capabilities::packages::api::{BaseFacts, Library, Packages};
use crate::capabilities::packages::ports::PackageSource;
use std::path::PathBuf;
use std::sync::Arc;

pub struct PackagesService {
    source: Arc<dyn PackageSource + Send + Sync>,
    base: BaseFacts,
}

impl PackagesService {
    pub fn new(source: Arc<dyn PackageSource + Send + Sync>) -> PackagesService {
        PackagesService {
            source,
            base: BaseFacts::default(),
        }
    }

    /// 目的：带上组合根算好的基础 profile / 宿主回退事实；测试替身继续用 `new` 的空事实。
    pub fn with_base(
        source: Arc<dyn PackageSource + Send + Sync>,
        base: BaseFacts,
    ) -> PackagesService {
        PackagesService { source, base }
    }
}

impl Packages for PackagesService {
    fn library(&self) -> Library {
        self.source.scan()
    }

    fn runtimes_dir(&self) -> PathBuf {
        self.source.dir()
    }

    fn base(&self) -> BaseFacts {
        self.base.clone()
    }
}
