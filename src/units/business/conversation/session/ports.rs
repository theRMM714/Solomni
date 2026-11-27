//! 目的：会话单元的出站端口——历史落盘（机制在适配层 `detail/fs_history.rs`）。
//! 管：`HistoryStore` 的契约：建 / 存元信息 / 追加 / 整体替换 / 列 / 读元信息 / 载入 / 删。
//! 不管：目录布局与原子写（在适配层）；会话状态派生（在 `domain/`）。
//! 联动：由 `service.rs` 的 `SessionService` 持有（R12），组合根注入具体实现。
use crate::units::business::session_model::api::{HistoryView, SessionMeta};
use crate::units::business::transcript::api::Content;

/// 目的：会话历史端口：一个会话一个目录（meta + 内容日志）。
///   日志只追加（留档只在日志里追加一条 rewind 事实）；只有版本控制的删除 / 恢复会整体重写。
///   线格式（一行一条事件 JSON）由适配层自己认，端口只交 `Content`。
pub trait HistoryStore {
    fn create(&self, meta: &SessionMeta) -> Result<(), String>;
    /// 写回会话元信息（配置界面的编辑：会话身份唯一真相在 meta.yaml）。
    fn save_meta(&self, meta: &SessionMeta) -> Result<(), String>;
    fn append(&self, name: &str, content: &Content) -> Result<(), String>;
    /// **整体重写**一个会话的流水（截断/恢复用）：留档只追加，删除与恢复会真的落到这里。
    /// 机制必须原子（先写临时文件再替换），失败时原文件保持可用。
    fn replace(&self, name: &str, content: &Content) -> Result<(), String>;
    fn list(&self) -> Result<Vec<HistoryView>, String>;
    /// **只读元信息**（不回放流水）：派发前的运行态判定等"只看身份与运行态"的场合用它，
    /// 免得为查一个字段把整份转录读一遍。
    fn meta(&self, name: &str) -> Result<SessionMeta, String>;
    fn load(&self, name: &str) -> Result<(SessionMeta, Content), String>;
    fn delete(&self, name: &str) -> Result<bool, String>;
}
