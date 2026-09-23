//! 协作会话状态机：建组 → 讨论 → 整理（出任务链）→ **审查关卡** → 链驱动（节点各跑在子会话里）
//! → 节点验收 → 总验收（拉模式）。见 docs/architecture/task-chain.md。
//! 前端经 Core 门面按 pending 驱动（set_task → confirm_slate? → begin → answer…），泵式收事件。
//! 发言席只有 agent：名单是 Vec<AgentMeta>（名字 / 模块 / 模型），member id = agent 实例名。
//! 名单的权威来源是会话 meta.agents（代拟确认后由 Core 写回 meta）；转录只用来恢复讨论进度。
//! 依赖全部为端口与核心数据；无 IO，无具体适配器。

use crate::core::agents::{self, RosterPick};
use crate::core::engine::{Discussion, Execution, Member, MemberTools, TurnOut, MAX_ROUNDS};
use crate::core::envelope;
use crate::core::events::{CheckView, LineView, Pending, SessionEvent};
use crate::core::exec::{self, ExecSpec};
use crate::core::history::{AgentMeta, SessionMeta};
use crate::core::module::{self, Module};
use crate::core::ports::{
    Chat, ChatGateway, CompleteOpts, ModuleSource, Msg, PackageSource, SysIo, ToolRunner,
};
use crate::core::prompt::Prompts;
use crate::core::providers::Settings;
use crate::core::workspace::Sandboxes;
use std::sync::Arc;

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
    fn llm_opts(&self) -> crate::core::ports::LlmOpts {
        crate::core::ports::LlmOpts {
            stream: self.settings.app.streaming,
            timeout_secs: self.settings.app.llm_timeout_secs,
        }
    }
}

impl CollabSession {
    /// 成员一轮之后的处置：**策略在 Discussion**（两条驱动共用同一份判定，
    /// 见 docs/architecture/session-model.md 二）。用户主动中止时计数不再工作。
    pub fn after_member_turn(
        &mut self,
        i: usize,
        has_verb: bool,
        user_stopped: bool,
        remind_cap: u32,
    ) -> crate::core::engine::AfterTurn {
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
        self.prompts.core.tool_texts.discuss_reminder.clone()
    }
}

/// 用户显式授权的只读根（`settings.yaml` 的 `fence_read`）：空 = 一个都不放行。
/// 与 `Core::fence_read_roots` 同义——两处都在 core 内，读的是同一份设置事实。
fn read_only_roots(app: &crate::core::providers::AppSettings) -> Vec<std::path::PathBuf> {
    app.fence_read
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .collect()
}

pub struct CollabSession {
    /// 是否代拟：显式传入（WorkSpec.delegate / meta.delegate），不从名单是否为空推断。
    delegated: bool,
    /// 在组名单（agent 实例；代拟确认前为空）。
    roster: Vec<AgentMeta>,
    task: String,
    /// 代拟拟好的名单（已逐条校验），确认后落到 roster。
    slate_picks: Vec<AgentMeta>,
    /// 登记处快照：agent 的模型解析与核心通道在此进行（策略在 core）。
    settings: Settings,
    /// 当前用户介入请求。
    pub pending: Option<Pending>,
    allow: bool,
    /// 已记录在案的执行方案（回档/重启后沿用，未整理则为 None）。
    plan: Option<String>,
    /// 核心给出的**任务链**（与方案一起出；审查关卡把它交用户看）。
    chain: Option<crate::core::chain::TaskChain>,
    disc: Option<Discussion>,
    /// 回合 id 计数器（整场工作单调递增）：agent 会话的回合标记用它。
    turns: u64,
    /// 上一次成员回合失败的原因：下一次泵推一步时如实交回（TurnOut::Interrupted）。
    turn_error: Option<String>,
    /// 泵让出的那一步：该问哪个成员、给它什么上下文。
    /// 泵**不自己调模型**——由核心取该 agent 的会话跑完再 feed 回来（见 session-model.md 二之二）。
    pending_ask: Option<(usize, Vec<crate::core::ports::Msg>)>,
    /// 已发出的转录行数（增量事件用）。
    emitted: usize,
    /// 下一条转录行的 id（会话内稳定序号）。
    next_line: u64,
    /// 回复 id 计数器（转录行按它分组；按落盘重建时从转录里的最大值续号）。
    reply_seq: u64,
    core_chat: crate::core::ports::BoxedChat,
    core_is_demo: bool,
    prompts: Prompts,
    gateway: Arc<dyn ChatGateway + Send + Sync>,
    source: Arc<dyn ModuleSource + Send + Sync>,
    /// 外部工具执行端口（策略在核心按模块清单放行，机制在适配层）。
    tools: Arc<dyn ToolRunner + Send + Sync>,
    /// 内置文件工具读写端口。
    io: Arc<dyn SysIo + Send + Sync>,
    /// 信封修复端口（手写信封不合法时的无歧义补救）。
    repair: Arc<dyn crate::core::ports::EnvelopeRepair + Send + Sync>,
    /// 运行日志（工具循环里"输出被长度截断"这类事实落盘）。
    log: Arc<dyn crate::core::ports::Log + Send + Sync>,
    /// 运行包库来源（工具可用性按它判定）。
    packages: Arc<dyn PackageSource + Send + Sync>,
    /// 本会话的执行选型（档位 + 运行包定版）。
    spec: ExecSpec,
    /// 本工作的沙箱清单（按 agent 实例名取）。
    sandboxes: Sandboxes,
    done: bool,
    /// 「停止」标志：由 CoreHandle 在派发时把任务登记处的取消标志注入（见 set_cancel）。
    cancel: Arc<std::sync::atomic::AtomicBool>,
    /// 方案是否已过审（审查关卡）：没过审不开工。由转录里的 [用户:同意方案] 派生。
    plan_approved: bool,
}

impl CollabSession {
    /// 装配会话：roster = 本次工作的 agent 名单（代拟时为空，等 draft_slate 填）。
    // 组合根注入的构造函数：参数天然多，收口成参数对象只是把参数挪个地方、并让装配更难读。
    // 这是有意的设计取舍（见 docs/testing/quality-isolation.md 的 allow 清单），不是没修。
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        gateway: Arc<dyn ChatGateway + Send + Sync>,
        source: Arc<dyn ModuleSource + Send + Sync>,
        settings: Settings,
        prompts: Prompts,
        tools: Arc<dyn ToolRunner + Send + Sync>,
        io: Arc<dyn SysIo + Send + Sync>,
        repair: Arc<dyn crate::core::ports::EnvelopeRepair + Send + Sync>,
        log: Arc<dyn crate::core::ports::Log + Send + Sync>,
        packages: Arc<dyn PackageSource + Send + Sync>,
        spec: ExecSpec,
        roster: Vec<AgentMeta>,
        delegated: bool,
        sandboxes: Sandboxes,
    ) -> Result<CollabSession, String> {
        let core_channel = settings.core_channel();
        let (core_chat, core_is_demo) = gateway.core_channel(core_channel.as_ref());
        Ok(CollabSession {
            delegated,
            roster,
            task: String::new(),
            slate_picks: Vec::new(),
            settings,
            pending: None,
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
            gateway,
            source,
            tools,
            io,
            repair,
            log,
            packages,
            spec,
            sandboxes,
            done: false,
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            plan_approved: false,
        })
    }

    /// 用户点「同意」：方案过关，可以开工。
    /// 记录在**转录**里（重启/回档后按它派生），不是内存里的临时状态。
    pub fn approve_plan(&mut self, sink: &mut dyn FnMut(SessionEvent)) {
        let line = self.view("[用户:同意方案]".to_string());
        sink(SessionEvent::Transcript(vec![line]));
        self.plan_approved = true;
        self.pending = None;
    }

    /// 接上「停止」：CoreHandle 在派发时注入任务登记处的取消标志。
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
    fn exec_note(&self, exec: &Execution) -> Option<String> {
        if exec.stopped || self.cancelled() {
            return Some(crate::core::events::stopped_note());
        }
        exec.error
            .clone()
            .map(|err| crate::core::events::interrupted_note(&err))
    }

    /// 任务链（未整理 = None）。
    pub fn chain(&self) -> Option<&crate::core::chain::TaskChain> {
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
                n.status = crate::core::chain::NodeStatus::Running;
            }
        }
    }

    /// **节点级验收**：核心 AI 按各节点**当前目标**判它的产出，返回逐节点结论。
    /// 一次调用判完整条链（比逐节点各调一次省得多，也便于横向比较）。
    /// 取字段而不是 &mut self：调用点在泵里，core_chat 要被可变借用。
    fn review_nodes(
        prompts: &Prompts,
        cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
        chain: Option<&crate::core::chain::TaskChain>,
        opts: crate::core::ports::CompleteOpts<'static>,
        core_chat: &mut dyn Chat,
    ) -> Result<Vec<(String, bool, String)>, String> {
        let nodes = chain.map(|c| c.nodes.clone()).unwrap_or_default();
        let listed = nodes
            .iter()
            .map(|n| {
                format!(
                    "- {}（{}）：目标「{}」\n  产出：{}",
                    n.id,
                    n.title,
                    n.objective,
                    n.report
                        .clone()
                        .unwrap_or_else(|| "（没有产出）".to_string())
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let user = prompts.render(&prompts.core.node_review.user, &[("nodes", listed)]);
        let msgs = vec![
            Msg::system(prompts.core.node_review.system.clone()),
            Msg::user(user),
        ];
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("已停止".to_string());
        }
        let stop = std::sync::Arc::clone(cancel);
        let mut keep =
            move |_c: crate::core::ports::Chunk| !stop.load(std::sync::atomic::Ordering::Relaxed);
        let done = core_chat.complete(&msgs, opts, &mut keep);
        if let Some(err) = done.error {
            return Err(err);
        }
        let arr = envelope::extract_json_array(&done.raw)
            .ok_or_else(|| format!("节点验收没有给出 JSON 数组：{}", done.raw))?;
        let parsed: Vec<NodeVerdict> = serde_json::from_str(&arr)
            .map_err(|e| format!("节点验收清单不合法（{}）：{}", e, arr))?;
        Ok(parsed.into_iter().map(|v| (v.node, v.ok, v.note)).collect())
    }

    /// 记下节点完成（产出先存下来，**验收由核心 AI 判**，见 review_nodes）。
    pub fn mark_node_done(&mut self, node: &str, report: &str) {
        if let Some(chain) = self.chain.as_mut() {
            if let Some(n) = chain.nodes.iter_mut().find(|n| n.id == node) {
                n.status = crate::core::chain::NodeStatus::Done;
                n.report = Some(report.to_string());
            }
        }
    }

    /// 记下一个节点的验收结论。
    pub fn set_node_acceptance(&mut self, node: &str, ok: bool, note: &str) {
        if let Some(chain) = self.chain.as_mut() {
            if let Some(n) = chain.nodes.iter_mut().find(|n| n.id == node) {
                n.acceptance = Some(crate::core::chain::Acceptance {
                    ok,
                    note: note.to_string(),
                });
            }
        }
    }

    /// 正在等用户（请教 / 方案待审 / 节点没过）：泵**不再往下推**，直到用户回应。
    pub fn awaiting_user(&self) -> bool {
        matches!(
            self.pending,
            Some(Pending::Ask { .. })
                | Some(Pending::PlanReview)
                | Some(Pending::NodeBlocked { .. })
        )
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
                    && matches!(n.status, crate::core::chain::NodeStatus::Running)
            })
            .map(|n| n.id.clone())
    }

    /// 把节点退回待办（验收没过 → 用户点「继续」→ 重派它）。
    pub fn reset_node(&mut self, node: &str) {
        if let Some(chain) = self.chain.as_mut() {
            if let Some(n) = chain.nodes.iter_mut().find(|n| n.id == node) {
                n.status = crate::core::chain::NodeStatus::Pending;
                n.sub_session = None;
                n.report = None;
                n.acceptance = None;
            }
        }
    }

    /// 在组名单（agent 实例）。
    pub fn roster(&self) -> &[AgentMeta] {
        &self.roster
    }

    /// 代拟拟好的名单（待用户确认；确认后落到 roster）。
    pub fn slate(&self) -> Vec<AgentMeta> {
        self.slate_picks.clone()
    }

    /// 代拟确认后由 Core 补上沙箱清单（名单刚定下来时才有）。
    pub fn set_sandboxes(&mut self, sandboxes: Sandboxes) {
        self.sandboxes = sandboxes;
    }

    /// 生成一条带 id 的转录行（工具行另走 tool_line，带调用视图）。
    fn view(&mut self, line: String) -> LineView {
        // 协作的讨论行各自成一条回复（协作的模型上下文不是从转录重建的，这个号只用于显示与分组一致）。
        let v = LineView {
            id: self.next_line,
            reply: self.next_line,
            line,
            ..Default::default()
        };
        self.next_line += 1;
        v
    }

    /// 名单里的 agent 名（顺序即名单）。
    fn names(&self) -> Vec<String> {
        self.roster.iter().map(|a| a.name.clone()).collect()
    }

    /// 拟名单给模型看的三份清单（已存 agent / 模块公地 / 可用模型）。
    fn briefing(&self, roster: &module::Roster) -> (String, String, String) {
        (
            agents::listing(&self.prompts, &self.settings.agents),
            module::listing(roster, &self.prompts.core.tool_texts),
            agents::model_listing(&self.settings.models, &self.prompts.core.tool_texts),
        )
    }

    /// 提交需求（总是第一步）。需求入转录（用户看到的与进上下文的一致）。
    /// 协作里用户不属任何 agent 的沙箱：@ 引用按 speaker = None 改写（共读同一段文字）。
    pub fn set_task(&mut self, task: &str, sink: &mut dyn FnMut(SessionEvent)) {
        let roots = crate::core::refs::RefRoots {
            work: self.sandboxes.shared.clone(),
            private: None,
        };
        let task = crate::core::refs::rewrite(task, None, &roots, &self.prompts.core.refs);
        if task.trim().is_empty() {
            sink(SessionEvent::Notice("[取消] 需求为空".into()));
            sink(SessionEvent::Ended);
            self.done = true;
            return;
        }
        self.task = task.clone();
        let user = self.view(format!("[用户:需求] {}", task));
        sink(SessionEvent::Transcript(vec![user]));
        if self.delegated {
            self.draft_slate(sink);
        } else {
            sink(SessionEvent::Notice(format!(
                "[建组] {}",
                self.names().join(" + ")
            )));
            self.pending = Some(Pending::ConfirmBegin);
        }
    }

    /// 委托代拟：核心拟发言名单（优先复用登记处的 agent，否则组装新的并给出模型），交用户确认。
    fn draft_slate(&mut self, sink: &mut dyn FnMut(SessionEvent)) {
        let roster = self.source.scan();
        let (agent_listing, module_listing, model_listing) = self.briefing(&roster);
        let user = self.prompts.render(
            &self.prompts.core.slate.user,
            &[
                ("agents", agent_listing),
                ("modules", module_listing),
                ("models", model_listing),
                ("task", self.task.clone()),
            ],
        );
        let msgs = vec![
            Msg::system(self.prompts.core.slate.system.clone()),
            Msg::user(user),
        ];
        let raw = self
            .core_chat
            .complete(&msgs, CompleteOpts::plain(false), &mut |_| true)
            .raw;
        let parsed = envelope::extract_json_object(&raw)
            .and_then(|obj| serde_json::from_str::<SlateReply>(&obj).ok());
        let Some(slate) = parsed else {
            sink(SessionEvent::Notice(
                "[错误] 代拟失败（模型无响应格式）。请直接点名 agent。".into(),
            ));
            sink(SessionEvent::Ended);
            self.done = true;
            return;
        };
        // 逐条校验（存在性、模型真实、整份名单内模块不重复）；拒收项如实告知。
        let (picks, rejected) = agents::resolve_picks(
            slate.picks,
            &self.settings.agents,
            &roster,
            &self.settings.models,
        );
        for r in rejected {
            sink(SessionEvent::Notice(format!("[代拟] {}，拒收", r)));
        }
        if picks.is_empty() {
            sink(SessionEvent::Notice("[错误] 代拟名单无可用 agent".into()));
            sink(SessionEvent::Ended);
            self.done = true;
            return;
        }
        let line = self.view(format!(
            "[代拟] {}",
            picks
                .iter()
                .map(|(a, why)| slate_item(a, why))
                .collect::<Vec<_>>()
                .join("；")
        ));
        sink(SessionEvent::Transcript(vec![line]));
        self.slate_picks = picks.into_iter().map(|(a, _)| a).collect();
        self.pending = Some(Pending::ConfirmSlate);
    }

    /// 回应代拟名单确认（仅 ConfirmSlate 挂起时有效）。
    pub fn confirm_slate(&mut self, ok: bool, sink: &mut dyn FnMut(SessionEvent)) {
        let line = self.view(format!("[用户:名单] {}", if ok { "确认" } else { "取消" }));
        sink(SessionEvent::Transcript(vec![line]));
        if !ok {
            sink(SessionEvent::Notice("[取消] 已按用户意愿取消".into()));
            sink(SessionEvent::Ended);
            self.done = true;
            return;
        }
        if self.slate_picks.is_empty() {
            // 名单只活在内存里（落档发生在确认之后）；重启后回来会空手。
            // 与其拿着空名单开工，不如如实告知并重新拟一份（名单本来就是要用户过目的提案）。
            sink(SessionEvent::Notice(
                "[提示] 上次拟的名单未落档（重启会丢），重新拟一份，请再确认。".into(),
            ));
            self.draft_slate(sink);
            return;
        }
        self.roster = self.slate_picks.clone();
        sink(SessionEvent::Notice(format!(
            "[建组] {}",
            self.names().join(" + ")
        )));
        self.pending = Some(Pending::ConfirmBegin);
    }

    /// 确认开始讨论（allow = yes,allow 自裁授权）；开聊并一路泵到暂停或交付。
    pub fn begin(&mut self, allow: bool, sink: &mut dyn FnMut(SessionEvent)) {
        if self.done || self.disc.is_some() {
            return;
        }
        self.allow = allow;
        let line = self.view(format!(
            "[用户:开始] {}",
            if allow { "yes,allow" } else { "yes" }
        ));
        sink(SessionEvent::Transcript(vec![line]));
        let prompts = self.prompts.clone();
        let (members, notes) = match self.assemble_members() {
            Ok(x) => x,
            Err(e) => {
                sink(SessionEvent::Notice(format!("[装配失败] {}", e)));
                return;
            }
        };
        for n in notes {
            sink(SessionEvent::Notice(n));
        }
        if self.core_is_demo {
            sink(SessionEvent::Notice(
                "[提示] 核心未配置供应商：整理/验收使用内置假模型（演示）".into(),
            ));
        }
        // 讨论也走**全局设置**（流式 + 预算），与单 agent 共用同一份。
        let llm = crate::core::ports::LlmOpts {
            stream: self.settings.app.streaming,
            timeout_secs: self.settings.app.llm_timeout_secs,
        };
        // 本席位的可用表态清单**由角色表渲染**（不是提示词里另写一遍）：漂不了。
        // 讨论席的"协议"= **机制说明 + 讨论约定 + 角色表渲染的表态清单**（三样缺一不可）：
        // 只说约定不说机制，AI 就不知道自己在什么流程里、该干什么（真机上就是空转）。
        let protocol = match self.prompts.systools.render_face("discussant") {
            Ok(face) => format!(
                "{}\n{}\n{}",
                self.prompts.core.mechanism, self.prompts.core.chat_protocol, face
            ),
            Err(e) => {
                sink(SessionEvent::Notice(format!(
                    "[装配失败] 角色表不可用：{}",
                    e
                )));
                return;
            }
        };
        let mut disc = Discussion::new(
            members,
            self.allow,
            prompts,
            llm,
            std::sync::Arc::clone(&self.cancel),
            protocol,
        );
        // 开场**不在这里跑**：核心驱动（见 session-model.md 二之二）——这里只渲染提示词、置游标，
        // 下一步由核心取该 agent 的会话跑第一个回合（逐成员外送在 feed_with 里）。
        disc.start(&self.task);
        self.disc = Some(disc);
        self.pump_with(sink);
    }

    /// 核心把某个成员回合的结果**交回来**：吸收、外送、继续泵（驱动权在核心，见 session-model.md 二之二）。
    /// 调用前核心应把该回合的核实行落进**该 agent 自己的会话**（它们不属于主会话）。
    pub fn feed_with(
        &mut self,
        i: usize,
        turn: crate::core::engine::MemberTurn,
        turn_id: u64,
        sink: &mut dyn FnMut(SessionEvent),
    ) {
        let next_line = std::cell::Cell::new(self.next_line);
        let handed = std::cell::Cell::new(0usize);
        let mut on_lines = |lines: &[crate::core::engine::DiscLine],
                            s: &mut dyn FnMut(SessionEvent)| {
            emit_new_lines(lines, &next_line, &handed, s);
        };
        let out = self.disc.as_mut().expect("disc 已确认存在").feed(
            i,
            turn,
            turn_id,
            &mut on_lines,
            sink,
        );
        self.next_line = next_line.get();
        self.emitted += handed.get();
        // 兜底：feed 提前返回时把剩下的行补齐；已交出去过的不会再出。
        if let Some(d) = self.disc.as_ref() {
            push_delta(d, &mut self.emitted, &mut self.next_line, sink);
        }
        if let Some(TurnOut::AskUser { member, question }) = out {
            self.pending = Some(Pending::Ask { member, question });
            return;
        }
        self.pump_with(sink);
    }

    /// 泵让出的那一步（该问谁、给它什么上下文）——由核心取走并驱动。
    pub fn take_ask(&mut self) -> Option<(usize, Vec<crate::core::ports::Msg>)> {
        self.pending_ask.take()
    }

    /// 第 i 个成员的 agent 名（核心据此拼出它的会话名 <工作>--<agent>）。
    pub fn member_id(&self, i: usize) -> Option<String> {
        self.disc.as_ref()?.member_id(i).map(|s| s.to_string())
    }

    /// 讨论回合的**工具面**（动词 + 只读核实）：核心驱动时交给 turn_with。
    pub fn systools(&self) -> &crate::core::roles::SystemTools {
        &self.prompts.systools
    }

    /// 「停止」标志：与核心共享同一个（停止能在一个模型调用内收尾）。
    pub fn disc_cancel(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        self.disc
            .as_ref()
            .map(|d| d.cancel_flag())
            .unwrap_or_else(|| std::sync::Arc::clone(&self.cancel))
    }

    /// 本回合的调用选项（流式 + 预算，取全局设置）。
    pub fn disc_opts(&self) -> crate::core::ports::CompleteOpts<'static> {
        crate::core::ports::CompleteOpts::plain(self.settings.app.streaming)
            .with_timeout(self.settings.app.llm_timeout_secs)
    }

    /// 开场还没开始过就先开始（核心驱动的第一步）。
    pub fn start_if_needed(&mut self) {
        if let Some(d) = self.disc.as_mut() {
            if d.not_started() {
                d.start(&self.task);
            }
        }
    }

    /// 回答 ask（仅 Ask 挂起时有效）；回答转达后继续泵。用户回答同样先改写 @ 引用。
    pub fn answer(&mut self, text: &str, sink: &mut dyn FnMut(SessionEvent)) {
        if matches!(self.pending, Some(Pending::Ask { .. })) {
            self.pending = None;
            let roots = crate::core::refs::RefRoots {
                work: self.sandboxes.shared.clone(),
                private: None,
            };
            let text = crate::core::refs::rewrite(text, None, &roots, &self.prompts.core.refs);
            if let Some(disc) = self.disc.as_mut() {
                disc.pending_user_answers.push(text);
            }
            self.pump_with(sink);
        }
    }

    /// 泵：推进讨论直至暂停（ask）或收敛并走完整理/执行/验收/交付；事件逐条经 sink 外送。
    pub fn pump_with(&mut self, sink: &mut dyn FnMut(SessionEvent)) {
        if self.done || self.disc.is_none() {
            return;
        }
        let prompts = self.prompts.clone();
        // 讨论阶段：只在未收敛时步进（回档/重启后可从中途接着走）。
        if !self.disc.as_ref().expect("disc 已确认存在").closed {
            loop {
                // 已在等用户（请教 / 待审 / 待继续）：泵不再往下推——驱动循环据此停下。
                if self.awaiting_user() {
                    return;
                }
                // 逐成员外送：一个成员说完就出它那一行（以前是整轮问完才一次性出，界面因此整轮不动）。
                // 回调里不能借 self（disc 正被可变借用），所以用 Cell/RefCell 暂存，调用后并回会话。
                // 上一个成员回合失败 / 被停：如实交回（不静默吞掉，也不当发言吸收）。
                let outcome = if let Some(err) = self.turn_error.take() {
                    TurnOut::Interrupted(err)
                } else {
                    // 泵只推**一步**：该问谁就存下并让出——驱动权在核心（它同时看得到协作会话与各 agent 的会话）。
                    match self.disc.as_mut().expect("disc 已确认存在").advance() {
                        crate::core::engine::Adv::Ask { i, msgs } => {
                            self.pending_ask = Some((i, msgs));
                            return;
                        }
                        // 开场刚问完：接着进轮次。
                        crate::core::engine::Adv::Opened => continue,
                        crate::core::engine::Adv::Out(out) => out,
                    }
                };
                match outcome {
                    TurnOut::Round => {}
                    TurnOut::Interrupted(err) => {
                        // 讨论中调用失败 / 被停：**不**把它当发言吸收，如实告知并中断本轮。
                        // 停止与失败用不同文案（用户看得到"是我停的"还是"它断了"）。
                        let note = if self.cancelled() {
                            crate::core::events::stopped_note()
                        } else {
                            crate::core::events::interrupted_note(&err)
                        };
                        sink(SessionEvent::Notice(note));
                        return;
                    }
                    TurnOut::Stopped => {
                        // 用户点了「停止」：被中断的那条发言没有吸收（半截 say/agree 会把状态算歪），
                        // 讨论保持可继续——点「继续」从断点接着推进。
                        sink(SessionEvent::Notice(crate::core::events::stopped_note()));
                        return;
                    }
                    TurnOut::AskUser { member, question } => {
                        self.pending = Some(Pending::Ask { member, question });
                        return;
                    }
                    TurnOut::Done => {
                        let round = self.disc.as_ref().expect("disc 存在").round;
                        let over_cap = round > MAX_ROUNDS;
                        if over_cap {
                            sink(SessionEvent::Notice(
                                "[上限] 讨论轮次超限，交用户裁决。".into(),
                            ));
                        }
                        sink(SessionEvent::DiscussionDone { round, over_cap });
                        break;
                    }
                }
            }
        }
        // 整理：只在还没有方案（或没有链）时做——回档/重启后沿用已记的，不重复花钱。
        if self.plan.is_none() || self.chain.is_none() {
            let made = self
                .disc
                .as_ref()
                .expect("disc 存在")
                .synthesize(self.core_chat.as_mut());
            match made {
                Ok((plan, chain)) => {
                    // **装配期门禁**：链必须自洽（悬空依赖 / 环 / 未知负责人 / 空目标）——
                    // 不静默开工；挡下时如实说明，用户点「继续」会重新整理。
                    let roster: Vec<String> = self
                        .disc
                        .as_ref()
                        .expect("disc 存在")
                        .members
                        .iter()
                        .filter(|m| m.present)
                        .map(|m| m.id.clone())
                        .collect();
                    let problems = chain.problems(&roster);
                    if !problems.is_empty() {
                        sink(SessionEvent::Notice(format!(
                            "[错误] 核心给出的任务链不自洽：{}。点「继续」会重新整理。",
                            problems.join("；")
                        )));
                        return;
                    }
                    self.plan = Some(plan.clone());
                    self.chain = Some(chain);
                    sink(SessionEvent::Plan(plan));
                }
                // 整理被停止 / 失败 / 回执不合法：都不落方案、不往下走，如实告知并交回用户。
                Err(err) => {
                    let note = if self.cancelled() {
                        crate::core::events::stopped_note()
                    } else {
                        crate::core::events::interrupted_note(&err)
                    };
                    sink(SessionEvent::Notice(note));
                    return;
                }
            }
        }
        let plan = self.plan.clone().unwrap_or_default();
        // **审查关卡**：整理完不自动开工——方案与链先交用户审查，点「同意」才推进。
        // 为什么闸门在这里：整理之后就是花钱的执行（每个成员一轮工具循环），让用户先看一眼最省事。
        if !self.plan_approved {
            sink(SessionEvent::PlanReview {
                plan: plan.clone(),
                chain: self.chain.clone().unwrap_or_default(),
            });
            self.pending = Some(Pending::PlanReview);
            return;
        }
        // 用户点「继续」= 重派没过的节点：先退回待办，再让核心重新派发（新起一轮子会话）。
        if let Some(Pending::NodeBlocked { nodes }) = self.pending.clone() {
            self.pending = None;
            for n in &nodes {
                self.reset_node(n);
            }
            return;
        }
        // **链驱动**：每个节点跑在它自己的子会话里（核心负责建会话与派发）。
        // 本会话在此**让出**——全部节点落定后才回来做节点验收与总验收。
        let settled = self
            .chain
            .as_ref()
            .map(|c| {
                !c.nodes.is_empty()
                    && c.nodes.iter().all(|n| {
                        matches!(
                            n.status,
                            crate::core::chain::NodeStatus::Done
                                | crate::core::chain::NodeStatus::Failed
                        )
                    })
            })
            .unwrap_or(false);
        if !settled {
            return;
        }
        // **节点级验收**：核心 AI 按各节点**当前目标**判它的产出；没过就暂停并交用户。
        let verdicts = match Self::review_nodes(
            &self.prompts,
            &self.cancel,
            self.chain.as_ref(),
            crate::core::ports::CompleteOpts::plain(self.settings.app.streaming)
                .with_timeout(self.settings.app.llm_timeout_secs),
            self.core_chat.as_mut(),
        ) {
            Ok(v) => v,
            Err(err) => {
                sink(SessionEvent::Notice(crate::core::events::interrupted_note(
                    &err,
                )));
                return;
            }
        };
        for (node, ok, note) in &verdicts {
            self.set_node_acceptance(node, *ok, note);
        }
        let bad: Vec<String> = verdicts
            .iter()
            .filter(|(_, ok, _)| !ok)
            .map(|(n, _, _)| n.clone())
            .collect();
        if !bad.is_empty() {
            sink(SessionEvent::Notice(format!(
                "[验收] 这些节点没通过：{}。点「继续」会重派它们。",
                bad.join("、")
            )));
            self.pending = Some(Pending::NodeBlocked { nodes: bad });
            return;
        }
        // 总验收：核心 AI 按**各节点的产出**核对（复用执行阶段的验收机制）→ 交付。
        let llm = self.llm_opts();
        let mut exec = Execution::new();
        for n in self.chain.as_ref().expect("链存在").nodes.iter() {
            let note = n
                .report
                .clone()
                .unwrap_or_else(|| "（该节点没有产出）".to_string());
            exec.reports.insert(n.id.clone(), note.clone());
            sink(SessionEvent::Report {
                id: n.id.clone(),
                text: note,
                rework: 0,
            });
        }
        exec.review(self.core_chat.as_mut(), &plan, &prompts, llm);
        if let Some(note) = self.exec_note(&exec) {
            sink(SessionEvent::Notice(note));
            return;
        }
        sink(review_event(&exec));
        let ok = exec.all_pass();
        sink(SessionEvent::Delivery {
            ok,
            over_rework: !ok,
        });
        sink(SessionEvent::Ended);
        self.done = true;
    }

    /// 从在组名单装配成员通道（带回落告知；策略在 core：该 agent 的模型 > 核心默认）。
    /// 一个 agent = 一个成员：system 由它全部模块合成，工具 = 各模块外部工具的并集。
    fn assemble_members(&self) -> Result<(Vec<Member>, Vec<String>), String> {
        let prompts = self.prompts.clone();
        let roster = self.source.scan();
        let library = self.packages.scan();
        let mut members = Vec::new();
        let mut notes = Vec::new();
        for a in &self.roster {
            // 该 agent 的模块：清单即事实，缺了就如实报错（不静默跳过）。
            let modules: Vec<Module> = a
                .modules
                .iter()
                .filter_map(|id| {
                    roster
                        .modules
                        .iter()
                        .find(|m| &m.manifest.id == id)
                        .cloned()
                })
                .collect();
            if modules.len() != a.modules.len() {
                let missing: Vec<String> = a
                    .modules
                    .iter()
                    .filter(|id| !roster.modules.iter().any(|m| &&m.manifest.id == id))
                    .cloned()
                    .collect();
                return Err(format!(
                    "agent {} 的模块已不在清单：{}",
                    a.name,
                    missing.join("、")
                ));
            }
            let channel = a
                .model
                .as_deref()
                .and_then(|id| self.settings.resolve(id).ok())
                .or_else(|| self.settings.core_channel());
            // 通道本身不再由成员持有（回合跑在各自的 agent 会话里）；这里只取它的如实告知。
            let (_chat, note) = self.gateway.member_channel(channel.as_ref(), &a.name);
            if let Some(n) = note {
                notes.push(n);
            }
            let sandbox = self.sandboxes.for_agent(&a.name).cloned().ok_or_else(|| {
                format!("agent {} 没有被分配沙箱（工作区未记录该 agent）", a.name)
            })?;
            let guide = crate::core::systool::guide(&prompts, &sandbox);
            // 形态按该 agent 的模型（或核心默认）解析：系统提示与实际协议必须一致
            let mode = if channel.is_some() {
                self.settings.tool_mode_for(a.model.as_deref())
            } else {
                crate::core::providers::ToolMode::Envelope
            };
            let system = module::agent_system(&prompts, &a.name, &modules, &guide, mode);
            let mut member = Member::plain(&a.name, system);
            // 围栏：可达范围 + 断网，由该 agent 的沙箱与 exec 段派生（机制在 adapters）；
            // 只读根来自用户显式授权（`fence_read`），默认空。
            let fence = crate::core::fence::FenceSpec::from_sandbox(&sandbox, self.spec.net)
                .with_read_only(read_only_roots(&self.settings.app));
            member.tools = Some(MemberTools {
                mode,
                // 模块 id → 该模块的（目录, 工具表）：多模块 agent 靠信封里的 module 消歧。
                modules: crate::core::engine::tool_table(&modules),
                observations: crate::core::systool::Observations::default(),
                repair: Arc::clone(&self.repair),
                log: Arc::clone(&self.log),
                runner: Arc::clone(&self.tools),
                sandbox,
                io: Arc::clone(&self.io),
                reply_seq: self.reply_seq,
                // 本档位下不能执行工具的模块（缺运行包）：机制侧据此拒绝执行。
                unavailable: exec::unavailable(&self.spec, &modules, &library),
                fence,
                // 讨论席的系统工具面**由角色表发放**（越权校验的唯一判据）。
                allowed: self
                    .prompts
                    .systools
                    .tool_face("discussant")
                    .map(|f| f.into_iter().map(|(id, _)| id.to_string()).collect())
                    .unwrap_or_default(),
            });
            members.push(member);
        }
        Ok((members, notes))
    }

    /// 继续：从断点推进（协作不需要用户发言）。未开始时由用户经裁决门确认，不由继续代劳。
    pub fn resume(&mut self, sink: &mut dyn FnMut(SessionEvent)) {
        if self.done {
            return;
        }
        if self.disc.is_some() {
            self.pump_with(sink);
            return;
        }
        if self.delegated && self.slate_picks.is_empty() && self.roster.is_empty() {
            self.draft_slate(sink);
        } else {
            sink(SessionEvent::Notice(
                "[提示] 等待你在裁决门确认名单 / 开始讨论。".into(),
            ));
        }
    }

    /// 撤回某 agent 的同意：转录追加一条撤回行（用户可见、也进上下文），并就地复位本轮表态。
    pub fn withdraw_agree(&mut self, agent: &str, sink: &mut dyn FnMut(SessionEvent)) {
        let line = self.view(format!("[用户:撤回] {}", agent));
        sink(SessionEvent::Transcript(vec![line]));
        if let Some(disc) = self.disc.as_mut() {
            for m in disc.members.iter_mut() {
                if m.id == agent {
                    m.agreed = false;
                }
            }
            disc.closed = false;
        }
    }

    /// 从落盘事件重建协作会话：名单取 meta.agents（权威），讨论进度由转录派生。
    /// 通道是可重建的机制，不是状态：按会话来时记住的 agent 名单重新装配。
    // 组合根注入的构造函数：参数天然多，收口成参数对象只是把参数挪个地方、并让装配更难读。
    // 这是有意的设计取舍（见 docs/testing/quality-isolation.md 的 allow 清单），不是没修。
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        gateway: Arc<dyn ChatGateway + Send + Sync>,
        source: Arc<dyn ModuleSource + Send + Sync>,
        settings: Settings,
        prompts: Prompts,
        tools: Arc<dyn ToolRunner + Send + Sync>,
        io: Arc<dyn SysIo + Send + Sync>,
        repair: Arc<dyn crate::core::ports::EnvelopeRepair + Send + Sync>,
        log: Arc<dyn crate::core::ports::Log + Send + Sync>,
        packages: Arc<dyn PackageSource + Send + Sync>,
        meta: &SessionMeta,
        events: &[serde_json::Value],
        sandboxes: Sandboxes,
    ) -> Result<CollabSession, String> {
        let names: Vec<String> = meta.agents.iter().map(|a| a.name.clone()).collect();
        let st = crate::core::collab_state::derive(events, &names);
        // 全部已发出的转录行（按 id 顺序），连降级标记一起读回（样式靠它，不靠文案）。
        let mut all_lines: Vec<crate::core::engine::DiscLine> = Vec::new();
        for ev in events {
            if ev.get("type").and_then(|t| t.as_str()) == Some("transcript") {
                if let Some(lines) = ev.get("lines").and_then(|l| l.as_array()) {
                    for l in lines {
                        if let Some(s) = l.get("line").and_then(|x| x.as_str()) {
                            all_lines.push(crate::core::engine::DiscLine {
                                text: s.to_string(),
                                degraded: l
                                    .get("degraded")
                                    .and_then(|d| d.as_bool())
                                    .unwrap_or(false),
                                // 核实行重建后仍是核实行（呈现层不因重启换样式）。
                                tool: l.get("tool").and_then(|t| {
                                    serde_json::from_value::<crate::core::events::ToolCallView>(
                                        t.clone(),
                                    )
                                    .ok()
                                }),
                                // 回合 id 随行落档：回档时两边按它对上（见 session-model.md 五）。
                                turn: l.get("turn").and_then(|t| t.as_u64()).unwrap_or(0),
                                system: l.get("system").and_then(|s| s.as_bool()).unwrap_or(false),
                            });
                        }
                    }
                }
            }
        }
        let total = all_lines.len() as u64;
        let core_channel = settings.core_channel();
        let (core_chat, core_is_demo) = gateway.core_channel(core_channel.as_ref());
        let mut s = CollabSession {
            delegated: meta.delegate,
            roster: meta.agents.clone(),
            task: st.task.clone().unwrap_or_default(),
            slate_picks: Vec::new(),
            settings,
            pending: None,
            allow: st.allow,
            plan: st.plan.clone(),
            // 链随 plan_review 事件落档：重建后按它还原，不重新整理（省一次模型调用）。
            chain: if st.chain.nodes.is_empty() {
                None
            } else {
                Some(st.chain.clone())
            },
            disc: None,
            turns: 0,
            turn_error: None,
            pending_ask: None,
            emitted: 0,
            next_line: total,
            reply_seq: crate::core::engine::max_reply(events),
            core_chat,
            core_is_demo,
            prompts: prompts.clone(),
            gateway,
            source,
            tools,
            io,
            repair,
            log,
            packages,
            spec: meta.exec.clone(),
            sandboxes,
            done: st.ended,
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            plan_approved: st.plan_approved,
        };
        if st.begun {
            // 讨论转录 = 最后一条 [用户:开始] 之后的行。
            let start = all_lines
                .iter()
                .rposition(|l| l.text.starts_with("[用户:开始]"))
                .map(|i| i + 1)
                .unwrap_or(all_lines.len());
            let disc_lines = all_lines[start..].to_vec();
            let (members, _) = s.assemble_members()?;
            let llm = s.llm_opts();
            let protocol = match s.prompts.systools.render_face("discussant") {
                Ok(face) => format!(
                    "{}\n{}\n{}",
                    s.prompts.core.mechanism, s.prompts.core.chat_protocol, face
                ),
                Err(_) => format!(
                    "{}\n{}",
                    s.prompts.core.mechanism, s.prompts.core.chat_protocol
                ),
            };

            let mut disc = Discussion::new(
                members,
                st.allow,
                prompts,
                llm,
                std::sync::Arc::clone(&s.cancel),
                protocol,
            );
            disc.round = st.round.max(1);
            disc.closed = st.closed;
            for m in disc.members.iter_mut() {
                m.present = st.present.get(&m.id).copied().unwrap_or(true);
                m.agreed = st.agreed.get(&m.id).copied().unwrap_or(false);
            }
            s.emitted = disc_lines.len();
            disc.transcript = disc_lines;
            s.disc = Some(disc);
        }
        s.pending = derive_pending(&st);
        Ok(s)
    }

    /// 会话是否已终结。
    pub fn is_done(&self) -> bool {
        self.done
    }
}

/// 代拟行里的一项（只给人看）：复用项标出来，组装项带上模块与模型。
fn slate_item(a: &AgentMeta, why: &str) -> String {
    if a.transient {
        format!(
            "{}〈{}〉→ {}（{}）",
            a.name,
            a.modules.join(","),
            a.model.clone().unwrap_or_default(),
            why
        )
    } else {
        format!("{}（复用；{}）", a.name, why)
    }
}

/// 从派生状态推出当前挂起（None = 没有待用户处理的门）。
fn derive_pending(st: &crate::core::collab_state::CollabState) -> Option<Pending> {
    if st.ended {
        return None;
    }
    if !st.begun {
        if st.slate.is_some() && !st.slate_confirmed {
            return Some(Pending::ConfirmSlate);
        }
        if st.task.is_some() {
            return Some(Pending::ConfirmBegin);
        }
        return None;
    }
    st.pending_ask.as_ref().map(|(m, q)| Pending::Ask {
        member: m.clone(),
        question: q.clone(),
    })
}

/// 代拟/推荐共用的应答形状：名单项（复用或组装）。
#[derive(serde::Deserialize)]
struct SlateReply {
    picks: Vec<RosterPick>,
}

/// 逐成员外送：把刚定稿的讨论行变成带**会话内稳定 id** 的转录事件交出去。
/// 为什么要 Cell/RefCell：回调在 `Discussion::step/open` 内部被调用，那时 `self` 正被可变借用，
/// 碰不到 `self.next_line` 与 `sink`——所以调用前后各并回一次，行只构造一次。
fn emit_new_lines(
    lines: &[crate::core::engine::DiscLine],
    next_line: &std::cell::Cell<u64>,
    handed: &std::cell::Cell<usize>,
    sink: &mut dyn FnMut(SessionEvent),
) {
    let views: Vec<LineView> = lines
        .iter()
        .map(|l| {
            let id = next_line.get();
            next_line.set(id + 1);
            LineView {
                id,
                reply: id,
                line: l.text.clone(),
                degraded: l.degraded,
                // 讨论回合里的核实行带着工具视图（与单 agent 的工具行同一形态）。
                tool: l.tool.clone(),
                // 回合 id 落进线格式：回档时两边按它对上（见 session-model.md 五）。
                turn: l.turn,
                // 系统消息标记落进线格式：前端据此显示系统行，不靠匹配文本。
                system: l.system,
                ..Default::default()
            }
        })
        .collect();
    handed.set(handed.get() + views.len());
    if !views.is_empty() {
        sink(SessionEvent::Transcript(views));
    }
}

/// 发出自上次以来的新转录行（增量），逐行分配会话内稳定 id。
fn push_delta(
    disc: &Discussion,
    emitted: &mut usize,
    next_line: &mut u64,
    sink: &mut dyn FnMut(SessionEvent),
) {
    if disc.transcript.len() > *emitted {
        let views: Vec<LineView> = disc.transcript[*emitted..]
            .iter()
            .map(|l| {
                let v = LineView {
                    id: *next_line,
                    reply: *next_line,
                    line: l.text.clone(),
                    degraded: l.degraded,
                    ..Default::default()
                };
                *next_line += 1;
                v
            })
            .collect();
        *emitted = disc.transcript.len();
        sink(SessionEvent::Transcript(views));
    }
}

fn review_event(exec: &Execution) -> SessionEvent {
    let items = exec
        .items
        .iter()
        .map(|i| CheckView {
            item: i.item.clone(),
            status: i.status.clone(),
            note: i
                .reason
                .clone()
                .or_else(|| i.evidence.clone())
                .unwrap_or_default(),
        })
        .collect();
    SessionEvent::Review {
        items,
        raw: exec.checklist_raw.clone(),
    }
}
