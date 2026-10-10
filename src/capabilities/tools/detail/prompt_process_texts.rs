//! 目的：把提示词册的收尾文案适配成 kernel 的 `ProcessTexts` 端口（进程机制不认识 prompt）。
//! 管：一组 render 调用——把 `ToolTexts` 的模板按变量渲染成字符串。
//! 不管：文案内容（在 prompts/）；进程机制（在 kernel/detail/process.rs）。
//! 联动：由组合根构造后注入 `ProcTools`；端口见 src/kernel/ports.rs。

use crate::capabilities::prompt::api::ToolTexts;
use crate::kernel::ports::ProcessTexts;
use std::sync::Arc;

/// 目的：`ProcessTexts` 的提示词册实现（组合根注入）。
pub struct PromptProcessTexts {
    texts: Arc<ToolTexts>,
}

impl PromptProcessTexts {
    /// 目的：用提示词册造一个文案适配器。
    pub fn new(texts: Arc<ToolTexts>) -> Self {
        Self { texts }
    }
}

impl ProcessTexts for PromptProcessTexts {
    fn stderr_header(&self) -> String {
        self.texts.tool_stderr_header.clone()
    }
    fn timeout(&self) -> String {
        self.texts.tool_timeout.clone()
    }
    fn fence_failed(&self) -> String {
        self.texts.tool_fence_failed.clone()
    }
    fn truncated(&self, chars: &str, limit: &str) -> String {
        self.texts.render(
            &self.texts.tool_truncated,
            &[("chars", chars.to_string()), ("limit", limit.to_string())],
        )
    }
    fn fence_blocked(&self, part: &str, path: &str, why: &str, fix: &str) -> String {
        self.texts.render(
            &self.texts.tool_fence_blocked,
            &[
                ("part", part.to_string()),
                ("path", path.to_string()),
                ("why", why.to_string()),
                ("fix", fix.to_string()),
            ],
        )
    }
    fn denied_by_user(&self) -> String {
        self.texts.tool_denied_by_user.clone()
    }
    fn no_answerer_defaulted(&self, part: &str, path: &str, option: &str) -> String {
        self.texts.render(
            &self.texts.tool_no_answerer_defaulted,
            &[
                ("part", part.to_string()),
                ("path", path.to_string()),
                ("option", option.to_string()),
            ],
        )
    }
    fn no_answerer_refused(&self, part: &str, path: &str) -> String {
        self.texts.render(
            &self.texts.tool_no_answerer_refused,
            &[("part", part.to_string()), ("path", path.to_string())],
        )
    }
    fn fence_unfenced(&self, part: &str, path: &str, why: &str) -> String {
        self.texts.render(
            &self.texts.tool_fence_unfenced,
            &[
                ("part", part.to_string()),
                ("path", path.to_string()),
                ("why", why.to_string()),
            ],
        )
    }
}
