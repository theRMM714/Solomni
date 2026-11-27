//! 目的：回收状态落盘——.vm/storage.json 只记上次回收摘要。
//! 管：读 / 写 LastGc（写走内核的原子发布），坏文件当没有。
//! 不管：策略（settings.yaml）与业务决策。
//! 联动：storage::service 每次 GC 后写；报告与 doctor 读。

use crate::kernel::api::fs;
use crate::units::business::maintenance::storage::api::LastGc;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct StateFile {
    version: u32,
    #[serde(default)]
    last_gc: Option<LastGc>,
}

/// 目的：状态文件路径。
pub fn path(root: &Path) -> std::path::PathBuf {
    crate::units::resource::vmstore::api::paths(root).storage_state()
}

/// 目的：读上次回收；没有或坏文件给 None。
pub fn read(root: &Path) -> Option<LastGc> {
    let text = std::fs::read_to_string(path(root)).ok()?;
    let f: StateFile = serde_json::from_str(&text).ok()?;
    f.last_gc
}

/// 目的：写上次回收（原子发布交内核；失败不阻断回收结果）。
pub fn write(root: &Path, last: &LastGc) -> Result<(), String> {
    let p = path(root);
    let text = serde_json::to_string_pretty(&StateFile {
        version: 1,
        last_gc: Some(last.clone()),
    })
    .map_err(|e| e.to_string())?;
    fs::write(None, &p, text.as_bytes(), fs::Expect::Any)
        .map(|_| ())
        .map_err(|fault| format!("写回收状态失败：{}", fault.message))
}
