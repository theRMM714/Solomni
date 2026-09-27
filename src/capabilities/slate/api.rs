//! 入站能力面：**其它能力只准用这里**（本业务没有端口、也没有状态，所以只有 DTO 与用例签名）。

use crate::capabilities::llm::api::{Chat, CompleteOpts, ToolMode};
use crate::capabilities::prompt::api::Prompt;
use crate::capabilities::registry::api::{Agents, ModelEntry};
use crate::capabilities::session::api::{AgentMeta, MemberTools, SessionEvent};
use crate::capabilities::tools::api::Tools;
use crate::capabilities::workspace::api::Roster;
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub use crate::capabilities::slate::service::propose;

/// 编排模式：决定给模型的说明（提示词册的 `mode_*`）与名单的**收束方式**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 单 agent：一个 AI 可带多个模块；核心给多条时收成一条。
    Single,
    /// 协作：若干 agent，各自独立、各自沙箱。
    Collab,
}

/// 名单里的一项：一个 agent 实例 + 它入选的理由。
#[derive(Debug, Clone)]
pub struct Pick {
    pub agent: AgentMeta,
    /// 核心给的那句入选理由（照实给用户看）。
    pub why: String,
}

/// 一次拟名单的产出。
#[derive(Debug, Clone)]
pub struct Proposal {
    /// 已按模式收束的名单（单 agent 模式最多一条）。
    pub picks: Vec<Pick>,
    /// 被拒收的条目（逐条的理由；原样如实告知，不静默改写）。
    pub rejected: Vec<String>,
}

/// 拟名单要用的**参与方事实**（调用方从各能力取来，本业务不持句柄）。
pub struct Parties<'a> {
    /// 提示词册（段与文案都在它的书里）。
    pub prompt: &'a dyn Prompt,
    /// 工具总表（按角色发放核心的工具面）。
    pub tools: &'a dyn Tools,
    /// 模块公地（本次工作的清单）。
    pub roster: &'a Roster,
    /// 登记处事实：在册 agent。
    pub agents: &'a Agents,
    /// 登记处事实：在册模型。
    pub models: &'a BTreeMap<String, ModelEntry>,
}

/// 这一次拟名单的输入与出口。
pub struct Request<'a> {
    /// 本次需求（用户那句话）。
    pub task: &'a str,
    pub mode: Mode,
    /// 核心通道（调用方建好：命令队列 / 协作会话各持一条）。
    pub chat: &'a mut dyn Chat,
    /// 这条通道的工具调用形态。
    pub tool_mode: ToolMode,
    /// 这一次调用的通道参数（流式 / 预算）。
    pub opts: CompleteOpts<'static>,
    /// 取消标志（`None` = 这一趟不听取消）。
    pub cancel: Option<&'a Arc<AtomicBool>>,
    /// 只读核实回路（协作会话有工具面；一次性推荐没有）。
    pub verify: Option<&'a mut MemberTools>,
    /// 核心这一轮的事实行推给谁（落不落盘由调用方按会话种类定）。
    pub sink: &'a mut dyn FnMut(SessionEvent),
}
