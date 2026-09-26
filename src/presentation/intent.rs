//! 共享意图层（呈现层内部）：把「用户想做什么」翻成一次或几次**入站能力**调用。
//! CLI 与 Web 都经这里，于是这些规则只有一份，不会两边各写一遍再慢慢分叉：
//! - agent 点名的解析与「登记处为空」的引导；
//! - 单模式下「点名多个 agent = 把它们的模块并成一个临时组合」；
//! - 工作名的缺省与唯一化；
//! - 会话动作的分发，以及「生成中不许改配置」的前置判断。
//!
//! 这一层**只编排**：不做业务决策（那是 core 的），也不碰 HTTP/argv（那是各呈现自己的传输）。

use crate::core::api::{AgentInstance, CollabStep};
use crate::core::api::{Ops, Output};

/// 登记处为空时的引导文案（CLI 与 Web 同源）。
pub const NO_AGENTS: &str = "登记处还没有 agent：请先到 Web 界面「设置 → agent 管理」建一个";

/// 把用户输入的名字串切成名字列表（逗号 / 中文逗号 / 空白分隔）。
pub fn split_names(arg: &str) -> Vec<String> {
    arg.split(|c: char| c == ',' || c == '，' || c.is_whitespace())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// 点名：按名字取登记处存档 → 用例输入。
/// **规则归登记处**（`pick_agents`）；这里只补一句呈现层的引导文案（空登记处该怎么说，是前端的事）。
pub fn pick_agents(ops: &Ops, names: &[String]) -> Result<Vec<AgentInstance>, String> {
    if ops.registry.agents()?.is_empty() {
        return Err(NO_AGENTS.to_string());
    }
    let views = ops.registry.pick_agents(names)?;
    Ok(views.iter().map(AgentInstance::from_view).collect())
}

/// 一次会话动作：CLI 与 Web 共用同一分发（新增动作只改这里）。
pub enum Action<'a> {
    /// 单 agent 说一句。
    Say(&'a str),
    /// 继续一次会话。
    Continue,
    /// 协作推进到下一阶段。
    Step(CollabStep, &'a str),
    /// 撤回同意。
    Withdraw(&'a str),
    /// 回档到某行之前。
    Rewind(u64),
    /// 改需求。
    UpdateTask(&'a str),
    /// 压缩上下文（AI 自己压成摘要；此后此前内容不再发给模型，用户仍可查看）。
    Compact,
}

/// 动作结果：生成类只回**事件台头部序号**（事实在事件台上，订阅者自己按 since 取）；
/// 回档/改需求给完整重放（那是快照，不是增量事实）。
pub enum Acted {
    Advanced(crate::core::api::Advance),
    Replayed(Vec<serde_json::Value>),
}

/// 执行一次动作。`out` 只管生成类的输出方式——「怎么显示」是呈现层的选择。
pub fn act(ops: &Ops, sid: &str, action: Action<'_>, out: Output) -> Result<Acted, String> {
    match action {
        Action::Say(text) => ops.sessions.say(sid, text, out).map(Acted::Advanced),
        Action::Continue => ops.sessions.continue_flow(sid, out).map(Acted::Advanced),
        Action::Step(step, text) => ops
            .sessions
            .collab_step(sid, step, text)
            .map(Acted::Advanced),
        Action::Withdraw(agent) => ops.sessions.withdraw_agree(sid, agent).map(Acted::Advanced),
        Action::Rewind(keep_id) => ops.sessions.rewind(sid, keep_id).map(Acted::Replayed),
        Action::UpdateTask(text) => ops.sessions.update_task(sid, text).map(Acted::Replayed),
        Action::Compact => ops.sessions.compact(sid).map(Acted::Advanced),
    }
}
