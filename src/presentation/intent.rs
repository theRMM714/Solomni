//! 共享意图层（呈现层内部）：把「用户想做什么」翻成一次或几次**入站能力**调用。
//! CLI 与 Web 都经这里，于是这些规则只有一份，不会两边各写一遍再慢慢分叉：
//! - agent 点名的解析与「登记处为空」的引导；
//! - 单模式下「点名多个 agent = 把它们的模块并成一个临时组合」；
//! - 工作名的缺省与唯一化；
//! - 会话动作的分发，以及「生成中不许改配置」的前置判断。
//!
//! 这一层**只编排**：不做业务决策（那是 core 的），也不碰 HTTP/argv（那是各呈现自己的传输）。

use crate::core::agents::AgentView;
use crate::core::api::{Ops, Output};
use crate::core::{AgentInstance, CollabStep, SessionEdit, WorkOpened, WorkSpec};

/// 登记处为空时的引导文案（CLI 与 Web 同源）。
pub const NO_AGENTS: &str = "登记处还没有 agent：请先到 Web 界面「设置 → agent 管理」建一个";

/// 登记处的全部 agent 视图（空登记处不是错误——调用方据此给引导）。
pub fn all_views(ops: &Ops) -> Result<Vec<AgentView>, String> {
    ops.registry.agents()
}

/// 视图 → 实例：点名与「无参 = 全部」两条路共用同一转换，避免两处写法分叉。
pub fn as_instances(views: &[AgentView]) -> Vec<AgentInstance> {
    views
        .iter()
        .map(|a| AgentInstance {
            name: a.name.clone(),
            transient: false,
            modules: a.modules.clone(),
            model: a.model.clone(),
        })
        .collect()
}

/// 把用户输入的名字串切成名字列表（逗号 / 中文逗号 / 空白分隔）。
pub fn split_names(arg: &str) -> Vec<String> {
    arg.split(|c: char| c == ',' || c == '，' || c.is_whitespace())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// 点名：按 agent 名取登记处的存档；名字不存在即如实报错（不猜、不代选）。
pub fn pick_agents(ops: &Ops, names: &[String]) -> Result<Vec<AgentInstance>, String> {
    let views = all_views(ops)?;
    if views.is_empty() {
        return Err(NO_AGENTS.to_string());
    }
    if names.is_empty() {
        return Err("没有点名任何 agent".to_string());
    }
    let mut out = Vec::new();
    for n in names {
        match views.iter().find(|a| &a.name == n) {
            Some(a) => out.push(AgentInstance {
                name: a.name.clone(),
                transient: false,
                modules: a.modules.clone(),
                model: a.model.clone(),
            }),
            None => {
                return Err(format!(
                    "无此 agent：{}（现有：{}）",
                    n,
                    views
                        .iter()
                        .map(|a| a.name.as_str())
                        .collect::<Vec<_>>()
                        .join(" · ")
                ))
            }
        }
    }
    Ok(out)
}

/// 单模式归并：多个 agent 的模块并成一个**临时组合**实例（模块去重、保序；模型取核心默认）。
pub fn merge_into_one(picked: &[AgentInstance], name: &str) -> AgentInstance {
    let mut merged: Vec<String> = Vec::new();
    for a in picked {
        for id in &a.modules {
            if !merged.contains(id) {
                merged.push(id.clone());
            }
        }
    }
    AgentInstance {
        name: name.to_string(),
        transient: true,
        modules: merged,
        model: None,
    }
}

/// 工作名的缺省与唯一化：base 为空用 fallback；重名追加 -2 / -3（进程内唯一即可）。
pub fn unique_work_name(ops: &Ops, base: &str, fallback: &str) -> Result<String, String> {
    let seed = if base.trim().is_empty() {
        fallback
    } else {
        base.trim()
    };
    if !ops.sessions.exists(seed)? {
        return Ok(seed.to_string());
    }
    let mut n = 2;
    loop {
        let cand = format!("{}-{}", seed, n);
        if !ops.sessions.exists(&cand)? {
            return Ok(cand);
        }
        n += 1;
    }
}

/// 新建一次工作（名字已定；名字的缺省/唯一化走 `unique_work_name`）。
pub fn open_work(
    ops: &Ops,
    name: String,
    mode: crate::core::WorkMode,
    agents: Vec<AgentInstance>,
    task: Option<String>,
    delegate: bool,
) -> Result<WorkOpened, String> {
    // 命令回包只给头部序号；事实由调用方按 `since` 从事件台订阅（谁发起的都一样）。
    let (opened, _head) = ops.sessions.create_work(WorkSpec {
        name,
        mode,
        agents,
        task,
        delegate,
    })?;
    Ok(opened)
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

/// 配置能否改：生成中一律拒绝（先「停止」或等它结束，避免改到一半的语义）。规则只写这一处。
pub fn ensure_editable(ops: &Ops, sid: &str) -> Result<(), String> {
    if ops.sessions.is_running(sid) {
        return Err("该会话正在生成中：先「停止」或等它结束，再改配置".to_string());
    }
    Ok(())
}

/// 提交配置编辑：前置判断与调用一起收口。
pub fn edit_session(ops: &Ops, sid: &str, edit: SessionEdit) -> Result<(), String> {
    ensure_editable(ops, sid)?;
    ops.sessions.edit(sid, edit)
}
