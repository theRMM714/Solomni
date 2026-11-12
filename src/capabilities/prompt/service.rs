//! 目的：提示词册的能力面实现与装配入口。
//! 管：实现 api::Prompt；load 装载一次后只答不问；持有者全进程只有一处。
//! 不管：文件系统与 yaml 解析（在 detail/yaml_prompts.rs）；渲染细节（在 domain/prompt.rs）。
//! 联动：状态就是 domain 的 Prompts；组合根调用 load 得到 Arc<dyn Prompt> 交给 conductor。

use crate::capabilities::prompt::api::{Prompt, Segment, Vars};
use crate::capabilities::prompt::domain::prompt::{Prompts, render};
use crate::capabilities::prompt::ports::PromptSource;
use std::sync::Arc;

impl Prompt for Prompts {
    fn text(&self, seg: Segment) -> &str {
        self.core.segment(seg)
    }

    fn render(&self, seg: Segment, vars: Vars) -> String {
        render(self.core.segment(seg), vars)
            .expect("提示词渲染失败：变量缺失属于装配错误，须修复 prompts/ 或调用方")
    }

    fn mechanism(&self, session: &str, role: &str) -> String {
        self.core.mechanisms_for(session, role)
    }

    fn env(&self, host: &str, guest: Option<&str>, vars: Vars) -> String {
        render(&self.core.env_for(host, guest), vars)
            .expect("提示词渲染失败：变量缺失属于装配错误，须修复 prompts/ 或调用方")
    }
}

/// 目的：组合根专用：装载册子（缺文件 / 缺键 = 装配错误，如实报错）。
pub fn load(source: &dyn PromptSource) -> Result<Arc<dyn Prompt>, String> {
    Ok(Arc::new(source.load()?))
}
