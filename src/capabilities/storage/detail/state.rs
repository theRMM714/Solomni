//! 目的：回收状态落盘——.vm/storage.json 只记上次回收摘要。
//! 管：读 / 写 LastGc；临时文件 + rename，坏文件当没有。
//! 不管：策略（settings.yaml）与业务决策。
//! 联动：storage::service 每次 GC 后写；报告与 doctor 读。

use crate::capabilities::storage::api::LastGc;
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
    root.join(".vm").join("storage.json")
}

/// 目的：读上次回收；没有或坏文件给 None。
pub fn read(root: &Path) -> Option<LastGc> {
    let text = std::fs::read_to_string(path(root)).ok()?;
    let f: StateFile = serde_json::from_str(&text).ok()?;
    f.last_gc
}

/// 目的：写上次回收（临时文件 + rename；失败不阻断回收结果）。
pub fn write(root: &Path, last: &LastGc) -> Result<(), String> {
    let p = path(root);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("建存储目录失败：{}", e))?;
    }
    let tmp = p.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(&StateFile {
        version: 1,
        last_gc: Some(last.clone()),
    })
    .map_err(|e| e.to_string())?;
    std::fs::write(&tmp, text).map_err(|e| format!("写回收状态失败：{}", e))?;
    std::fs::rename(&tmp, &p).map_err(|e| format!("放回收状态失败：{}", e))
}
