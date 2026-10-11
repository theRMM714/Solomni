//! 目的：模块能力的用例与端口持有者。
//! 管：按用例答话（当前是清单）；持有清单来源端口。
//! 不管：目录遍历机制（在 `detail/fs_modules.rs`）；清单契约与校验（在 `domain/module.rs`）。
//! 联动：装配（new 出适配器）在组合根；端口只在这里（R12）。

use crate::capabilities::modules::api::{Modules, Roster};
use crate::capabilities::modules::ports::ModuleSource;
use std::sync::Arc;

/// 目的：模块能力——持清单来源端口，按用例答话。
pub struct ModulesService {
    source: Arc<dyn ModuleSource + Send + Sync>,
}

impl ModulesService {
    /// 目的：组合根专用——注入清单来源端口。
    pub fn new(source: Arc<dyn ModuleSource + Send + Sync>) -> ModulesService {
        ModulesService { source }
    }
}

impl Modules for ModulesService {
    fn roster(&self) -> Roster {
        self.source.scan()
    }
}
