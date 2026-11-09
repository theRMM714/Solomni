//! 目的：压缩通道工厂——全局设置指向的压缩模型在这里解析，会话触发压缩时现取。
//! 管：`CompactChoice`（同模型 / 指定模型 / 不可用）与 `CompactRuntime` 的构建与更新。
//! 不管：压缩本身（session 能力）、全局设置怎么读（registry）。
//! 联动：Conductor 持一份 `CompactRuntime`，设置变更时更新选择；会话经 `CompactChatFactory` 取通道。

use super::*;

/// 目的：压缩模型的选择（全局设置；只读触发时解析，不参与每个回合）。
#[derive(Debug, Clone)]
pub(super) enum CompactChoice {
    /// 空设置 = 会话同模型。
    Session,
    /// 指定模型：解析好的通道 + 窗口。
    Model {
        id: String,
        channel: Channel,
        window: u64,
    },
    /// 设置指向的模型 / 供应商不可用：压缩触发时如实报错。
    Unavailable(String),
}

/// 目的：压缩通道工厂：设置 / 模型 / 供应商变更时由 conductor 更新选择，会话在触发时读。
pub(super) struct CompactRuntime {
    llm: Arc<dyn Llm + Send + Sync>,
    choice: RwLock<CompactChoice>,
}

impl CompactRuntime {
    pub(super) fn new(llm: Arc<dyn Llm + Send + Sync>) -> CompactRuntime {
        CompactRuntime {
            llm,
            choice: RwLock::new(CompactChoice::Session),
        }
    }

    pub(super) fn set(&self, choice: CompactChoice) {
        *self.choice.write().unwrap_or_else(|e| e.into_inner()) = choice;
    }
}

impl crate::capabilities::session::api::CompactChatFactory for CompactRuntime {
    fn build(
        &self,
        module: &str,
        session_model: Option<&str>,
    ) -> Result<Option<crate::capabilities::session::api::CompactChatSpec>, String> {
        let guard = self.choice.read().unwrap_or_else(|e| e.into_inner());
        match &*guard {
            CompactChoice::Session => Ok(None),
            CompactChoice::Unavailable(err) => Err(err.clone()),
            CompactChoice::Model {
                id,
                channel,
                window,
            } => {
                if session_model == Some(id.as_str()) {
                    return Ok(None);
                }
                Ok(Some(crate::capabilities::session::api::CompactChatSpec {
                    chat: self.llm.member_channel(channel, module),
                    model: id.clone(),
                    window: *window,
                }))
            }
        }
    }
}
