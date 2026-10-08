//! 目的：协作会话状态机——建组、讨论、整理（出任务链）、审查关卡、链驱动、节点验收、总验收。
//! 管：会话对象的字段（名单 / 讨论 / 链 / **本会话的裁决队句柄**）与各关卡的登记、处置、快照形态。
//! 不管：队列本身的排队与校验（在 `crate::capabilities::session::api::DecisionDoor`，与工具级确认共用）、
//!   讨论推进（`discussion.rs`）、泵的每一步（`pump.rs`）、回合收发与回答（`turn_io.rs`）、
//!   代拟名单与按转录重建（`slate.rs`）；它不碰会话表，也不写别人的会话。
//! 联动：契约见 docs/session/session-model.md 的「请用户裁决：一条通道，消息 + 选项」与
//!   docs/collab/task-chain.md；前端按快照的 pending 与推的裁决事件驱动，不做第二真相。

use super::pump::*;
use crate::capabilities::collab::service::discussion::Discussion;
use crate::capabilities::collab::service::synthesis::Execution;
use crate::capabilities::llm::api::{Chat, Llm, Msg};
use crate::capabilities::prompt::api::{Prompt, Segment};
use crate::capabilities::registry::api::Settings;
use crate::capabilities::session::api::AgentMeta;
use crate::capabilities::session::api::{DecisionDoor, LineView, Pending, SessionEvent};
use crate::capabilities::tools::api::ToolExec;
use crate::capabilities::workspace::api::ExecSpec;
use crate::capabilities::workspace::api::Sandboxes;
use crate::capabilities::workspace::api::Workspace;
use std::sync::Arc;

/// 节点验收的结论：逐节点 (node, ok, note)。
type NodeVerdicts = Vec<(String, bool, String)>;

/// 节点验收的一条结论（核心 AI 的 JSON 回执，见 prompts/roles/planner.yaml 的 node_review）。
#[derive(Debug, Clone, serde::Deserialize)]
struct NodeVerdict {
    node: String,
    ok: bool,
    #[serde(default)]
    note: String,
}

impl CollabSession {
    /// 本次调用的通道参数（流式 + 预算）：**全局设置**，与单 agent 共用同一份。
    pub(crate) fn llm_opts(&self) -> crate::capabilities::llm::api::LlmOpts {
        crate::capabilities::llm::api::LlmOpts {
            stream: self.settings.app.streaming,
            timeout_secs: self.settings.app.llm_timeout_secs,
        }
    }
}

impl CollabSession {
    /// 成员一轮之后的处置：**策略在 Discussion**（两条驱动共用同一份判定，
    /// 见 docs/session/session-model.md 二）。用户主动中止时计数不再工作。
    pub fn after_member_turn(
        &mut self,
        i: usize,
        has_verb: bool,
        user_stopped: bool,
        remind_cap: u32,
    ) -> crate::capabilities::collab::service::discussion::AfterTurn {
        self.disc.as_mut().expect("disc 已确认存在").after_turn(
            i,
            has_verb,
            user_stopped,
            remind_cap,
        )
    }

    /// 提醒到顶：主会话如实记一行"未回应"（**系统消息**——不是它说的），本轮放过它。
    pub fn pass_over(&mut self, i: usize, sink: &mut dyn FnMut(SessionEvent)) {
        let Some(id) = self.member_id(i) else {
            return;
        };
        let note = format!("[{}] 本轮未回应", id);
        if let Some(d) = self.disc.as_mut() {
            d.note_system(&note);
            // 放过它：游标往后挪一格（本轮不再问它）。
            d.skip(i);
        }
        if let Some(d) = self.disc.as_ref() {
            push_delta(d, &mut self.emitted, &mut self.next_line, sink);
        }
    }

    /// 注入到该成员会话里的提醒文案（核心在轮次边界注入；用户主动中止时不注入）。
    pub fn reminder_text(&self) -> String {
        self.prompts.tools().discuss_reminder.clone()
    }
}

/// 用户显式授权的只读根（`settings.yaml` 的 `fence_read`）：空 = 一个都不放行。
/// 与 `Conductor::fence_read_roots` 同义——两处都读同一份设置事实（协调业务与协作会话各有一份派生）。
pub(crate) fn read_only_roots(
    app: &crate::capabilities::registry::api::AppSettings,
) -> Vec<std::path::PathBuf> {
    app.fence_read
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .collect()
}

pub struct CollabSession {
    /// 是否代拟：显式传入（WorkSpec.delegate / meta.delegate），不从名单是否为空推断。
    pub(crate) delegated: bool,
    /// 在组名单（agent 实例；代拟确认前为空）。
    pub(crate) roster: Vec<AgentMeta>,
    pub(crate) task: String,
    /// 代拟拟好的名单（已逐条校验），确认后落到 roster。
    pub(crate) slate_picks: Vec<AgentMeta>,
    /// 登记处快照：agent 的模型解析与核心通道在此进行（策略在 conductor）。
    pub(crate) settings: Settings,
    /// 目的：本会话的裁决队（**与工具级确认共用同一条队**）：排队与校验归它，这里只管各关卡的处置。
    pub(crate) door: Arc<DecisionDoor>,
    pub(crate) allow: bool,
    /// 已记录在案的执行方案（回档/重启后沿用，未整理则为 None）。
    pub(crate) plan: Option<String>,
    /// 核心给出的**任务链**（与方案一起出；审查关卡把它交用户看）。
    pub(crate) chain: Option<crate::capabilities::taskchain::api::TaskChain>,
    pub(crate) disc: Option<Discussion>,
    /// 回合 id 计数器（整场工作单调递增）：agent 会话的回合标记用它。
    pub(crate) turns: u64,
    /// 上一次成员回合失败的原因：下一次泵推一步时如实交回（TurnOut::Interrupted）。
    pub(crate) turn_error: Option<String>,
    /// 泵让出的那一步：该问哪个成员、给它什么上下文。
    /// 泵**不自己调模型**——由核心取该 agent 的会话跑完再 feed 回来（见 session-model.md 二之二）。
    /// 泵让出的那一步：成员下标 / 身份块 / 本回合提示（身份每回合现渲染，不存进任何人的消息列表）。
    pub(crate) pending_ask: Option<(usize, String, Vec<crate::capabilities::llm::api::Msg>)>,
    /// 已发出的转录行数（增量事件用）。
    pub(crate) emitted: usize,
    /// 下一条转录行的 id（会话内稳定序号）。
    pub(crate) next_line: u64,
    /// 回复 id 计数器（转录行按它分组；按落盘重建时从转录里的最大值续号）。
    pub(crate) reply_seq: u64,
    pub(crate) core_chat: crate::capabilities::llm::api::BoxedChat,
    pub(crate) core_is_demo: bool,
    /// 核心通道的工具调用形态（原生才声明工具；信封通道看提示词里的说明）。
    pub(crate) core_mode: crate::capabilities::llm::api::ToolMode,
    /// 提示词册能力面：**不是册子本体**（持有者只有提示词能力一处），这里只按名字取段。
    pub(crate) prompts: Arc<dyn Prompt>,
    /// 工具总表与角色表的能力面（**表本体在工具能力里**，与册子互不依赖）。
    pub(crate) systools: Arc<dyn crate::capabilities::tools::api::Tools>,
    /// llm 用例面（**不持它的端口**，R12）：按通道造句柄、修信封都走它。
    pub(crate) llm: Arc<dyn Llm + Send + Sync>,
    /// 工作区用例面（**不持它的端口**，R12）：清单、运行包库与工作区目录都走它。
    pub(crate) workspace: Arc<dyn Workspace + Send + Sync>,
    /// 工具执行面（**不持它的端口**，R12）：跑外部/内置工具都走它。
    pub(crate) tools: Arc<dyn ToolExec + Send + Sync>,
    /// 运行日志（工具循环里"输出被长度截断"这类事实落盘）。
    pub(crate) log: Arc<dyn crate::kernel::ports::Log + Send + Sync>,
    /// 本会话的执行选型（档位 + 运行包定版）。
    pub(crate) spec: ExecSpec,
    /// 本工作的沙箱清单（按 agent 实例名取）。
    pub(crate) sandboxes: Sandboxes,
    pub(crate) done: bool,
    /// 「停止」标志：由 ConductorHandle 在派发时把任务登记处的取消标志注入（见 set_cancel）。
    pub(crate) cancel: Arc<std::sync::atomic::AtomicBool>,
    /// 方案是否已过审（审查关卡）：没过审不开工。由转录里的 [用户:同意方案] 派生。
    pub(crate) plan_approved: bool,
    /// 核心 AI 给用户的**建议**（随方案 / 节点验收那一次调用一起产出）：只属于**下一次登记**的那一关，
    /// 登记时搬进队列里的那一关；用掉就清，别漏到下一关。
    pub(crate) gate_advice: String,
}

impl CollabSession {
    /// 装配会话：roster = 本次工作的 agent 名单（代拟时为空，等 draft_slate 填）。
    // 组合根注入的构造函数：参数天然多，收口成参数对象只是把参数挪个地方、并让装配更难读。
    // 这是有意的设计取舍（见 docs/testing/quality-isolation.md 的 allow 清单），不是没修。
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        llm: Arc<dyn Llm + Send + Sync>,
        workspace: Arc<dyn Workspace + Send + Sync>,
        settings: Settings,
        prompts: Arc<dyn Prompt>,
        systools: Arc<dyn crate::capabilities::tools::api::Tools>,
        tools: Arc<dyn ToolExec + Send + Sync>,
        log: Arc<dyn crate::kernel::ports::Log + Send + Sync>,
        spec: ExecSpec,
        roster: Vec<AgentMeta>,
        delegated: bool,
        sandboxes: Sandboxes,
        // 本会话的裁决队（核心按会话 id 给；与工具级确认共用同一条）。
        door: Arc<DecisionDoor>,
    ) -> Result<CollabSession, String> {
        let core_channel = settings.core_channel();
        let (core_chat, core_is_demo) = llm.core_channel(core_channel.as_ref());
        let core_mode = settings.tool_mode_for(None);
        Ok(CollabSession {
            core_mode,
            delegated,
            roster,
            task: String::new(),
            slate_picks: Vec::new(),
            settings,
            door,
            allow: false,
            plan: None,
            chain: None,
            disc: None,
            turns: 0,
            turn_error: None,
            pending_ask: None,
            emitted: 0,
            next_line: 0,
            reply_seq: 0,
            core_chat,
            core_is_demo,
            prompts,
            systools,
            llm,
            workspace,
            tools,
            log,
            spec,
            sandboxes,
            done: false,
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            plan_approved: false,
            gate_advice: String::new(),
        })
    }

    /// 用户点「同意」：方案过关，可以开工。
    /// 记录在**转录**里（重启/回档后按它派生），不是内存里的临时状态。
    pub fn approve_plan(&mut self, sink: &mut dyn FnMut(SessionEvent)) {
        let line = self.view(LineView::user("同意方案", String::new()));
        sink(SessionEvent::Transcript(vec![line]));
        self.plan_approved = true;
        self.gate_advice.clear(); // 这一关解除了，建议不再属于任何挂起的事
    }

    /// 接上「停止」：ConductorHandle 在派发时注入任务登记处的取消标志。
    /// 注入后泵在每次模型调用前与**调用中途**都看它，所以「停止」能在一个调用内收尾。
    pub fn set_cancel(&mut self, cancel: Arc<std::sync::atomic::AtomicBool>) {
        if let Some(d) = self.disc.as_mut() {
            d.set_cancel(Arc::clone(&cancel));
        }
        self.cancel = cancel;
    }

    /// 是否已被要求停止。
    /// 用户是否已**主动中止**（中止时计数不再工作：不注入提醒）。
    pub fn cancelled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// 执行 / 验收阶段该不该收尾：**停止（用户的意图）与失败（故障）分开说**。
    /// 两者都如实收尾并保持会话可继续，但用户看到的话不一样——混成一句会让用户以为出了故障。
    pub(crate) fn exec_note(&self, exec: &Execution) -> Option<String> {
        if exec.stopped || self.cancelled() {
            return Some(crate::capabilities::session::api::stopped_note());
        }
        exec.error
            .clone()
            .map(|err| crate::capabilities::session::api::interrupted_note(&err))
    }

    /// 任务链（未整理 = None）。
    pub fn chain(&self) -> Option<&crate::capabilities::taskchain::api::TaskChain> {
        self.chain.as_ref()
    }

    /// 方案是否已过审。
    pub fn plan_approved(&self) -> bool {
        self.plan_approved
    }

    /// 记下某个节点跑在哪个子会话里，并把它标成"在跑"。
    pub fn mark_node_started(&mut self, node: &str, sub: &str) {
        if let Some(chain) = self.chain.as_mut() {
            if let Some(n) = chain.nodes.iter_mut().find(|n| n.id == node) {
                n.sub_session = Some(sub.to_string());
                n.status = crate::capabilities::taskchain::api::NodeStatus::Running;
            }
        }
    }

    /// 用户对裁决的回应**明确到可以开工 / 放行**了吗：由核心 AI 判（`verdict` 工具）。
    /// 取字段而不是 &mut self：调用点在协作这一侧，core_chat 要被可变借用（同 review_nodes）。
    // 参数是一组"取字段而不是自己"的出口（prompts/cancel/opts/chat/verify/三句输入），
    // 收口成参数对象只会把它们藏起来、让"谁读什么"更难看清（同 engine::converse_with 的取舍）。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn judge_clear(
        prompt: &dyn Prompt,
        systools: &dyn crate::capabilities::tools::api::Tools,
        cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
        opts: crate::capabilities::llm::api::CompleteOpts<'static>,
        mode: crate::capabilities::llm::api::ToolMode,
        core_chat: &mut dyn Chat,
        verify: Option<&mut crate::capabilities::session::api::MemberTools>,
        kind: &str,
        payload: &str,
        text: &str,
        // 核心这一轮的行推给谁（落不落由协作会话定）。
        sink: &mut dyn FnMut(SessionEvent),
    ) -> Result<(bool, String), String> {
        let user = prompt.render(
            Segment::VerdictUser,
            &[
                ("kind", kind.to_string()),
                ("payload", payload.to_string()),
                ("text", text.to_string()),
            ],
        );
        let msgs = vec![
            Msg::system(prompt.text(Segment::VerdictSystem).to_string()),
            Msg::user(user),
        ];
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("已停止".to_string());
        }
        let payload = crate::capabilities::session::api::core_operation(
            systools,
            "planner",
            "verdict",
            mode,
            core_chat,
            &msgs,
            opts,
            Some(cancel),
            verify,
            sink,
        )?;
        let clear = payload
            .get("clear")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let why = payload
            .get("why")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok((clear, why))
    }

    /// **节点级验收**：核心 AI 按各节点**当前目标**判它的产出，返回逐节点结论。
    /// 一次调用判完整条链（比逐节点各调一次省得多，也便于横向比较）。
    /// 取字段而不是 &mut self：调用点在泵里，core_chat 要被可变借用。
    // 参数是一组"取字段而不是自己"的出口（prompts / cancel / opts / mode / chat / verify / 重填说明），
    // 与 Execution::review 同一取舍（见 docs/testing/quality-isolation.md）。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn review_nodes(
        prompt: &dyn Prompt,
        systools: &dyn crate::capabilities::tools::api::Tools,
        cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
        chain: Option<&crate::capabilities::taskchain::api::TaskChain>,
        opts: crate::capabilities::llm::api::CompleteOpts<'static>,
        mode: crate::capabilities::llm::api::ToolMode,
        core_chat: &mut dyn Chat,
        verify: Option<&mut crate::capabilities::session::api::MemberTools>,
        // 上一次填错了要它重填的话（核心据此**一直重填**到合法，不设次数上限）。
        retry: Option<&str>,
        // 核心这一轮的行推给谁。
        sink: &mut dyn FnMut(SessionEvent),
    ) -> Result<(NodeVerdicts, String), String> {
        let nodes = chain.map(|c| c.nodes.clone()).unwrap_or_default();
        let listed = nodes
            .iter()
            .map(|n| {
                format!(
                    "- {}（{}，负责人 {}）：目标「{}」\n  产出：{}",
                    n.id,
                    n.title,
                    n.assignee,
                    n.objective,
                    n.report
                        .clone()
                        .unwrap_or_else(|| "（没有产出）".to_string())
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let user = prompt.render(Segment::NodeReviewUser, &[("nodes", listed)]);
        let mut msgs = vec![
            Msg::system(prompt.text(Segment::NodeReviewSystem).to_string()),
            Msg::user(user),
        ];
        if let Some(again) = retry {
            msgs.push(Msg::user(again.to_string()));
        }
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("已停止".to_string());
        }
        // 核心操作走工具调用：节点验收结论由 node_verdict 工具承载。
        // 带核实回路：模型想先读/查落盘物时，核心执行只读工具再回灌（不再直接判"没调用"）。
        let payload = crate::capabilities::session::api::core_operation(
            systools,
            "orchestrator",
            "node_verdict",
            mode,
            core_chat,
            &msgs,
            opts,
            Some(cancel),
            verify,
            sink,
        )?;
        let parsed: Vec<NodeVerdict> = payload
            .get("verdicts")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .ok_or_else(|| format!("node_verdict 的载荷没有 verdicts 数组：{}", payload))?;
        let advice = payload
            .get("advice")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok((
            parsed.into_iter().map(|v| (v.node, v.ok, v.note)).collect(),
            advice,
        ))
    }

    /// 记下节点完成（产出先存下来，**验收由核心 AI 判**，见 review_nodes）。
    pub fn mark_node_done(&mut self, node: &str, report: &str) {
        if let Some(chain) = self.chain.as_mut() {
            if let Some(n) = chain.nodes.iter_mut().find(|n| n.id == node) {
                n.status = crate::capabilities::taskchain::api::NodeStatus::Done;
                n.report = Some(report.to_string());
            }
        }
    }

    /// 记下一个节点的验收结论。
    pub fn set_node_acceptance(&mut self, node: &str, ok: bool, note: &str) {
        if let Some(chain) = self.chain.as_mut() {
            if let Some(n) = chain.nodes.iter_mut().find(|n| n.id == node) {
                n.acceptance = Some(crate::capabilities::taskchain::api::Acceptance {
                    ok,
                    note: note.to_string(),
                });
            }
        }
    }

    /// 目的：挂起一件等用户裁决的事：**进队尾**，并把它此刻的队列形态推给界面。
    /// 约束：卡号与排队归队列（会话内唯一、跨重启稳定）；一次只有队首那张对用户可见 / 可答。
    pub(crate) fn ask_user(&mut self, p: Pending, sink: &mut dyn FnMut(SessionEvent)) {
        let advice = std::mem::take(&mut self.gate_advice);
        let (_id, evs) = self.door.push(None, p, &advice, None);
        for e in evs {
            sink(e);
        }
    }

    /// 裁决的**背景**：交给核心 AI 判"用户的意图明确了吗"用——把现场说清楚，别让它猜。
    pub(crate) fn decision_brief(&self, p: &Pending) -> String {
        match p {
            Pending::PlanReview => {
                let chain = self
                    .chain
                    .as_ref()
                    .map(|c| {
                        c.nodes
                            .iter()
                            .map(|n| format!("- {}（{}）→ {}", n.id, n.title, n.assignee))
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
                format!(
                    "方案：{}\n任务链：\n{}",
                    self.plan.clone().unwrap_or_default(),
                    chain
                )
            }
            Pending::NodeBlocked { nodes } => format!("没过验收的节点：{}", nodes.join("、")),
            Pending::Ask { member, question } => format!("{} 问：{}", member, question),
            Pending::ConfirmSlate => "代拟名单待用户确认。".to_string(),
            // 工具级确认不经过"判意图明确"这条路（它的处置是放行 / 拒绝，不是开不开工）。
            Pending::ToolApproval { tool, .. } => format!("工具级确认：{} 要不要放行。", tool),
            // 工具层自己发起的那一类（围栏这类）同样不走"判意图明确"：它的处置在发起方那边。
            Pending::ToolAsk(ask) => format!("工具层在问：{}。", ask.title),
            Pending::ConfirmBegin => "名单已定，等用户确认开始讨论。".to_string(),
        }
    }

    /// 正在等用户（队列里还有没答的卡）：泵**不再往下推**，直到用户回答或整队作废。
    /// 判据是队列本身：请教 / 名单确认 / 开工确认 / 方案待审 / 节点没过都排在同一条队上。
    pub fn awaiting_user(&self) -> bool {
        !self.door.is_empty()
    }

    /// 成员回合失败：记下原因，下一次泵推一步时如实交回（不静默吞掉）。
    pub fn note_turn_failure(&mut self, err: String) {
        self.turn_error = Some(err);
    }

    /// 当前轮次（写进 agent 会话的回合标记里）。
    pub fn round(&self) -> usize {
        self.disc.as_ref().map(|d| d.round).unwrap_or(0)
    }

    /// 下一个**回合 id**：整场工作单调递增，写进 agent 会话的回合标记。
    pub fn next_turn_id(&mut self) -> u64 {
        self.turns += 1;
        self.turns
    }

    /// 哪个节点**正跑在这个会话里**（一个 agent 一个会话，可能依次服务多个节点）。
    pub fn running_node_of(&self, sub: &str) -> Option<String> {
        let chain = self.chain.as_ref()?;
        chain
            .nodes
            .iter()
            .find(|n| {
                n.sub_session.as_deref() == Some(sub)
                    && matches!(
                        n.status,
                        crate::capabilities::taskchain::api::NodeStatus::Running
                    )
            })
            .map(|n| n.id.clone())
    }

    /// 把节点退回待办（验收没过 → 用户点「继续」→ 重派它）。
    pub fn reset_node(&mut self, node: &str) {
        if let Some(chain) = self.chain.as_mut() {
            if let Some(n) = chain.nodes.iter_mut().find(|n| n.id == node) {
                n.status = crate::capabilities::taskchain::api::NodeStatus::Pending;
                n.sub_session = None;
                n.report = None;
                n.acceptance = None;
                n.reported = false; // 重派后"完成"要再报一次
            }
        }
    }

    /// 代拟确认后由 Conductor 补上沙箱清单（名单刚定下来时才有）。
    pub fn set_sandboxes(&mut self, sandboxes: Sandboxes) {
        self.sandboxes = sandboxes;
    }

    /// 生成一条带 id 的转录行（说话人/动词/种类都在字段里；工具行走带 tool 视图的那条路）。
    pub(crate) fn view(&mut self, mut line: LineView) -> LineView {
        // 协作的讨论行各自成一条回复（协作的模型上下文不是从转录重建的，这个号只用于显示与分组一致）。
        line.id = self.next_line;
        line.reply = self.next_line;
        let v = line;
        self.next_line += 1;
        v
    }

    /// 名单里的 agent 名（顺序即名单）。
    pub(crate) fn names(&self) -> Vec<String> {
        self.roster.iter().map(|a| a.name.clone()).collect()
    }

    /// 提交需求（总是第一步）。需求入转录（用户看到的与进上下文的一致）。
    /// 协作里用户不属任何 agent 的沙箱：@ 引用按 speaker = None 改写（共读同一段文字）。
    pub fn set_task(&mut self, task: &str, sink: &mut dyn FnMut(SessionEvent)) {
        let roots = crate::capabilities::prompt::api::RefRoots {
            work: self.sandboxes.shared.clone(),
            private: None,
        };
        let task =
            crate::capabilities::prompt::api::rewrite(task, None, &roots, &self.prompts.refs());
        if task.trim().is_empty() {
            sink(SessionEvent::Notice("[取消] 需求为空".into()));
            sink(SessionEvent::Ended);
            self.done = true;
            return;
        }
        self.task = task.clone();
        let user = self.view(LineView::user("需求", task.clone()));
        sink(SessionEvent::Transcript(vec![user]));
        if self.delegated {
            self.draft_slate(sink);
        } else {
            sink(SessionEvent::Notice(format!(
                "[建组] {}",
                self.names().join(" + ")
            )));
            self.ask_user(Pending::ConfirmBegin, sink);
        }
    }
}
