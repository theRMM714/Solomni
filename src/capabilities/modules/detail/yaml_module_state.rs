//! 目的：启停覆盖的落盘实现（`.home/modules.yaml`）。
//! 管：一份 `EnabledState` 的读写；文件不存在 = 空；文件非法 = 如实报错。
//! 不管：有效值判定（在 `service.rs`）；模块清单与契约（在 `domain/module.rs`）。
//! 联动：实现 ports::ModuleStateStore；路径由组合根给。

use crate::capabilities::modules::domain::state::EnabledState;
use crate::capabilities::modules::ports::ModuleStateStore;
use std::path::{Path, PathBuf};

/// 目的：启停覆盖的文件存储（组合根构造）。
pub struct YamlModuleState {
    path: PathBuf,
}

impl YamlModuleState {
    /// 目的：用文件路径造一个存储（组合根给 .home/modules.yaml）。
    pub fn new(path: PathBuf) -> YamlModuleState {
        YamlModuleState { path }
    }

    /// 目的：写盘：建目录 → 写。
    fn write(path: &Path, text: &str) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("建模块状态目录失败：{}", e))?;
        }
        std::fs::write(path, text).map_err(|e| format!("写模块状态失败：{}", e))
    }
}

impl ModuleStateStore for YamlModuleState {
    fn load(&self) -> Result<EnabledState, String> {
        match std::fs::read_to_string(&self.path) {
            Ok(t) => yaml_serde::from_str::<EnabledState>(&t)
                .map_err(|e| format!("modules.yaml 非法：{}", e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(EnabledState::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    fn save(&self, state: &EnabledState) -> Result<(), String> {
        let text = yaml_serde::to_string(state).map_err(|e| e.to_string())?;
        Self::write(&self.path, &text)
    }
}
