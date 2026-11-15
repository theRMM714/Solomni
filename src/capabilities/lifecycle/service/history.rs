//! 目的：会话历史的读侧——列出（树序）与打开（只读回放）。
//! 管：读失败记日志并返回空表；打开时按 rewind 截断事件流。
//! 不管：写与删（`session` 的 `History` 端口）；删除收尾（`service/delete.rs`）。
//! 联动：`conductor` 的历史用例经本能力 `api` 取用；端口实现由组合根注入。

use crate::capabilities::lifecycle::domain::history::tree_order;
use crate::capabilities::session::api::{History, HistoryView, SessionMeta};
use crate::kernel::ports::Log;

/// 目的：历史列表：**树序**（每个父会话紧跟它的子会话）——侧栏据此缩进，
///  顺序与缩进同源，不会再出现"子会话排到父会话上面"的错位（见 session-model.md 一）。
///  读盘失败如实记日志，返回空表。
pub fn list(
    history: &(dyn History + Send + Sync),
    log: &(dyn Log + Send + Sync),
) -> Vec<HistoryView> {
    match history.list() {
        Ok(v) => tree_order(v),
        Err(e) => {
            log.error("lifecycle::history_list", &format!("会话列表失败：{}", e));
            Vec::new()
        }
    }
}

/// 目的：打开历史会话：返回元信息与事件流（只读回放；是否续跑由用户点「继续」授权）。
///  回放同样应用 rewind 截断——流水保留审计，会话内容以截断后为准。
pub fn open(
    history: &(dyn History + Send + Sync),
    name: &str,
) -> Result<(SessionMeta, Vec<serde_json::Value>), String> {
    let (meta, events) = history.load(name)?;
    Ok((
        meta,
        crate::capabilities::session::api::truncate_events(&events),
    ))
}
