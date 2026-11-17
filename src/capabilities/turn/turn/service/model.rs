//! 目的：一次模型调用的通道参数与思考档——流式与预算取值、按会话解析的思考片段。
//! 管：`llm_opts` / `llm_opts_for` / `thinking_fragment_for`（回合推进与宿主共用这一份判据，见下三个自由函数）。
//! 不管：思考档视图与按根设置（在 work）、通道本身（llm 能力）、会话配置落盘。
//! 联动：turn 的生成入口（单 agent / 节点 / 成员 / 代理）与 work 的建会话都经这里取参数。

use std::sync::Arc;

use crate::capabilities::llm::api::LlmOpts;
use crate::capabilities::catalog::registry::api::Registry;
use crate::capabilities::conversation::session::api::{History, work_root};

use super::session::Turn;

/// 目的：本次模型调用的通道参数：**预算与"能不能流式"都取全局设置**（讨论、执行、验收、单 agent 共用一份）。
///  `want_stream` 是调用方这一次的意愿（呈现层按回包形状给）：**设置是上限，调用方可以在本次放弃流式**；
///  设置关掉时一律非流式。两处各判一次迟早会打架，所以判据只在这里。
pub fn llm_opts(registry: &dyn Registry, want_stream: bool) -> LlmOpts {
    let app = registry.app();
    LlmOpts {
        stream: app.streaming && want_stream,
        timeout_secs: app.llm_timeout_secs,
        thinking: None,
    }
}

/// 目的：这条会话这一次生成要用的调用参数：全局流式/预算 + **按模型解析的思考片段**。
/// 约束：只用于对话生成（单 agent / 代理 / 协作成员会话）；核心内部编排不走它。
pub fn llm_opts_for(
    history: &dyn History,
    registry: &dyn Registry,
    sid: &str,
    want_stream: bool,
) -> Result<LlmOpts, String> {
    let mut opts = llm_opts(registry, want_stream);
    opts.thinking = thinking_fragment_for(history, registry, sid)?;
    Ok(opts)
}

/// 目的：这条会话的模型在当前工作档位下要并入请求体的思考片段（None = 不发）。
/// 约束：档位取**工作根** meta 的 `thinking`；模型取这条会话第一个 agent 的模型，没有 agent（代理）就用核心默认；
///  模型没登记该档 = 如实报错（不静默降级成供应商默认）。
pub fn thinking_fragment_for(
    history: &dyn History,
    registry: &dyn Registry,
    sid: &str,
) -> Result<Option<Arc<serde_json::Value>>, String> {
    let root = work_root(history, sid)?;
    let level = history.meta(&root)?.thinking;
    let model = history
        .meta(sid)
        .ok()
        .and_then(|m| m.agents.first().and_then(|a| a.model.clone()));
    Ok(registry
        .thinking_fragment(model.as_deref(), level.as_deref())?
        .map(Arc::new))
}

impl Turn {
    /// 目的：本会话的调用参数（`Turn` 已持落盘面，自由函数按它取事实）。
    pub(crate) fn llm_opts_for(
        &self,
        registry: &dyn Registry,
        sid: &str,
        want_stream: bool,
    ) -> Result<LlmOpts, String> {
        llm_opts_for(self.history.as_ref(), registry, sid, want_stream)
    }

    /// 目的：本会话的思考片段（档位取工作根，模型取本会话首个 agent）。
    pub(crate) fn thinking_fragment_for(
        &self,
        registry: &dyn Registry,
        sid: &str,
    ) -> Result<Option<Arc<serde_json::Value>>, String> {
        thinking_fragment_for(self.history.as_ref(), registry, sid)
    }
}
