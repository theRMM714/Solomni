//! 核心层：定义抽象（ports）、编排业务（会话/引擎）、会话中心。
//! 分层纪律：本文件不出现文件读、ureq、stdin/stdout——机制全在各能力的 `detail` 与 `kernel/detail`，
//! 装配（new 适配器）只发生在 main 组合根。前端只见 Conductor 门面、会话句柄与 SessionEvent 流。

use crate::capabilities::conductor::api::{
    AgentInstance, AgentSuggestion, CollabStep, ConfigAgent, FilesAgentRootView, FilesAgentView,
    FilesRootsView, FilesView, RuntimeReport, SessionConfig, SessionEdit, SessionView, TierChoices,
    WorkMode, WorkOpened, WorkSpec,
};
use crate::capabilities::llm::api::Llm;
use crate::capabilities::session::api::History;
#[cfg(test)]
pub(crate) use crate::capabilities::session::api::Live;
use crate::capabilities::session::api::{Pending, SessionEvent};
use crate::capabilities::workspace::api::Workspace;

use crate::capabilities::collab::api::AfterTurn;
use crate::capabilities::collab::api::CollabSession;
use crate::capabilities::llm::api::Channel;
use crate::capabilities::llm::api::Msg;
use crate::capabilities::prompt::api::{Prompt, Segment};
use crate::capabilities::registry::api::Registry;
use crate::capabilities::session::api::{AgentMeta, HistoryView, RunState, SessionMeta};
use crate::capabilities::tools::api::{ToolExec, Tools};
use crate::capabilities::workspace::api::Module;
use crate::kernel::api::SessionId;
use crate::kernel::ports::Log;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// 会话实例：单 agent 会话或协作会话（本体自带端口，可跨线程移动）。
/// 两变体大小差得远（协作会话带整份讨论状态），装箱只换来一次间接寻址、
/// 却把"会话本体可直接移动"这个形状改掉——有意不装箱（见 docs/testing/quality-isolation.md 的 allow 清单）。
#[allow(clippy::large_enum_variant)]
pub enum Session {
    Single(crate::capabilities::session::api::AgentSession),
    Collab(CollabSession),
}

/// 单模式的组合语义：多个 agent 的模块并成一个**临时组合**实例（去重、保序；模型取核心默认）。
fn merge_into_one(picked: &[AgentInstance], name: &str) -> AgentInstance {
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

/// **这个会话要不要留档**：由**会话种类**定，不由调用点定。
/// - `Keep`：定稿事件落盘（单 agent 会话与协作会话都要回放——前者是模型上下文的一部分，
///   后者是派生状态的来源）；
/// - `Drop`：只推不留（**系统会话**：一次性动作，如"让核心推荐 agent"，重启后本来就该是空的）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PersistPolicy {
    Keep,
    Drop,
}

impl PersistPolicy {
    /// 一条事件该不该留：`Drop` 一条都不留；`Keep` 也不留**短暂事件**
    /// （流式增量 / 运行态 / 实时工具卡 / 裁决卡——它们只给在场的前端看）。
    pub(crate) fn keeps(&self, ev: &SessionEvent) -> bool {
        if *self == PersistPolicy::Drop {
            return false;
        }
        !matches!(
            ev,
            SessionEvent::Delta { .. }
                | SessionEvent::Working { .. }
                | SessionEvent::ToolCall(_)
                | SessionEvent::Decision { .. }
        )
    }
}

/// **系统会话的 sid 约定**：以 `#` 开头的是"没有地方落盘"的一次性动作（只推不留）。
/// 见 docs/session/session-model.md 二（会话种类 × 推 / 落）。
pub(crate) fn is_system_session(sid: &str) -> bool {
    sid.starts_with('#')
}

/// 核心推荐（suggest_models）的系统会话 sid：一次性的推荐动作**没有工作区**，
/// 它的行只推给在场的前端看（推荐是弹窗里的一次问答），不落盘，也不进会话列表。
pub(crate) const SYSTEM_SID_SUGGEST: &str = "#suggest";

/// 这个会话的落盘策略（**判定只有这一处**：会话种类 → 策略）。
fn persist_policy_for(sid: &str) -> PersistPolicy {
    if is_system_session(sid) {
        PersistPolicy::Drop
    } else {
        PersistPolicy::Keep
    }
}

/// 事件落盘（按该会话的策略；短暂事件不落盘）：失败如实告知，返回要外送的警告（None = 一切正常）。
///
/// 为什么抽成自由函数：工作线程按**一次模型调用**的粒度增量落盘，必须与核心走同一段逻辑——
/// 两条路径各写一份的话，"什么算定稿、什么不落盘"迟早会不一致。
pub(crate) fn persist_events(
    history: &dyn History,
    log: &dyn Log,
    sid: &str,
    events: &[SessionEvent],
) -> Option<String> {
    // 留不留由**这个会话的策略**说了算（会话种类 → 策略，见 persist_policy_for）。
    let policy = persist_policy_for(sid);
    let jsons: Vec<serde_json::Value> = events
        .iter()
        .filter(|e| policy.keeps(e))
        .map(|e| e.to_json())
        .collect();
    if jsons.is_empty() {
        return None;
    }
    match history.append(sid, &jsons) {
        Ok(()) => None,
        Err(e) => {
            log.error(
                "conductor::history_append",
                &format!("会话 {} 落盘失败：{}", sid, e),
            );
            Some(format!("[警告] 会话记录落盘失败：{}", e))
        }
    }
}

/// 增量落盘手柄：工作线程按"一次模型调用"的粒度把定稿事件落盘。
/// 为什么要它：整段生成跑完才落一次盘，会让中途刷新看不到已经产生的部分。
#[derive(Clone)]
pub(crate) struct Persister {
    history: Arc<dyn History + Send + Sync>,
    log: Arc<dyn Log + Send + Sync>,
    sid: String,
}

impl Persister {
    /// 落盘这一批；返回要外送给用户的警告（None = 正常）。
    pub(crate) fn persist(&self, events: &[SessionEvent]) -> Option<String> {
        persist_events(self.history.as_ref(), self.log.as_ref(), &self.sid, events)
    }
}

/// 一次单 agent 生成的**准备结果**（核心线程上只算到这里，模型调用在工作线程上）。
pub(crate) enum Prepared {
    /// 可以跑：会话已从核心表取出，由工作线程独占。
    Run {
        /// 装箱：这个变体比其它两个大得多（会话本体），而它本来就是**一次性移交**给工作线程的。
        session: Box<crate::capabilities::session::api::AgentSession>,
        /// 本回合的**身份块**：按当前提示词册与登记处现渲染（不进会话的消息列表）。
        identity: String,
        /// 生成前要先给用户的事件（例如工具形态变更的提示）。
        prefix: Vec<SessionEvent>,
        llm: crate::capabilities::llm::api::LlmOpts,
        /// 增量落盘手柄：逐轮外送的同时就落盘（中途刷新页面因此看得到已产生的部分）。
        persister: Persister,
    },
    /// 不用跑模型：直接把这批事件回给调用方（例如"末条是 AI 发言"）。
    Immediate(Vec<SessionEvent>),
    /// 不是单 agent 会话：交回调用方走它自己那条路（协作）。
    NotSingle,
}

/// **应用服务**：持有注入的端口、能力面与会话中心；前端拿不到它（只拿 `conductor::api::Ops`）。
/// 组合根在 `ConductorHandle::spawn` 里把它**移进核心自己的执行线程**——此后状态只被那一个线程碰，
/// 所以这里不加任何锁（并发不变式见 ARCHITECTURE.md §一）。
pub struct Conductor {
    /// 登记处能力：**四份 yaml 的状态在它里面**，conductor 只按 `Registry` 调用（看不见它的字段）。
    registry: Box<dyn Registry>,
    /// 会话历史直连面（**不持它的端口**，R12）：造/读/写/删会话都走它。
    history: Arc<dyn History + Send + Sync>,
    /// 工作区用例面（**不持它的端口**，R12）：清单事实、运行包库与工作区目录都走它。
    workspace: Arc<dyn Workspace + Send + Sync>,
    /// llm 用例面：按解析出来的通道造收发句柄 + 信封的无歧义修复（通道的**解析**在登记处能力）。
    llm: Arc<dyn Llm + Send + Sync>,
    /// 工具执行面（**不持它的端口**，R12）：跑外部/内置工具、释放围栏授权都走它。
    tools: Arc<dyn ToolExec + Send + Sync>,
    log: Arc<dyn crate::kernel::ports::Log + Send + Sync>,
    /// 宿主能力探测（读环境、查路径存在性都在它后面；conductor 因此不碰 std::env 与文件系统）。
    probe: Arc<dyn crate::kernel::ports::HostProbe + Send + Sync>,
    /// 提示词册能力：**册子本体在它里面**（只有一处），conductor 只按名字取段。
    /// 用 `Arc`：协作会话要与核心**共享**这一份（逐处克隆整本册子是白费）。
    prompt: Arc<dyn Prompt>,
    /// 工具总表与角色表的能力面（`systools/` 两张表）：**表本体在工具能力里**，与册子互不依赖。
    systools: Arc<dyn Tools>,
    sessions: HashMap<SessionId, Session>,
    /// 正在生成的会话：对象被工作线程**取走**了，核心表里暂时没有它。
    /// 为什么取出而不是就地生成：生成要跑几十秒到几分钟，占着唯一的命令队列会让
    /// 读接口（历史列表、状态）与其它会话的命令全排在它后面——界面因此"假死"。
    /// 这一态只表示"不在表里是因为在生成"，不是"不存在"。
    running: std::collections::BTreeSet<SessionId>,
}

mod env;
mod flow;
mod history;
pub mod proxy;
mod rewind;
mod turn;
mod work;
impl Conductor {
    /// 组合根专用：main 负责创建适配器并注入；conductor 不自建任何具体实现。
    // 组合根注入的构造函数：参数天然多，收口成参数对象只是把参数挪个地方、并让装配更难读。
    // 这是有意的设计取舍（见 docs/testing/quality-isolation.md 的 allow 清单），不是没修。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        registry: Box<dyn Registry>,
        history: Arc<dyn History + Send + Sync>,
        workspace: Arc<dyn Workspace + Send + Sync>,
        llm: Arc<dyn Llm + Send + Sync>,
        tools: Arc<dyn ToolExec + Send + Sync>,
        prompt: Arc<dyn Prompt>,
        systools: Arc<dyn Tools>,
        log: Arc<dyn crate::kernel::ports::Log + Send + Sync>,
        probe: Arc<dyn crate::kernel::ports::HostProbe + Send + Sync>,
    ) -> Conductor {
        Conductor {
            registry,
            history,
            workspace,
            llm,
            tools,
            log,
            probe,
            prompt,
            systools,
            sessions: HashMap::new(),
            running: std::collections::BTreeSet::new(),
        }
    }

    /// 日志端口句柄：入站手柄（conductor::api）与组合根共用同一份事实记录。
    pub fn log_handle(&self) -> Arc<dyn crate::kernel::ports::Log + Send + Sync> {
        Arc::clone(&self.log)
    }

    /// 登记处能力面（只读）：组合根与测试读登记处的事实走这里。
    pub fn registry(&self) -> &dyn Registry {
        self.registry.as_ref()
    }

    /// 登记处能力面（可写）：登记处的用例只经它调用。
    pub fn registry_mut(&mut self) -> &mut dyn Registry {
        self.registry.as_mut()
    }

    /// 目录保留名（`systools/names.yaml`）：agent 实例名不得占用（与工作区布局同源）。
    pub(crate) fn reserved_names(&self) -> Vec<String> {
        self.systools.reserved_names()
    }

    /// 工具总表的声明书（代理执行者在队列桥那一侧校验参数/拼回执要用）。
    pub(crate) fn systools_book(&self) -> crate::capabilities::tools::api::ToolBook {
        self.systools.book()
    }

    /// 模型侧工具文案（共享一份）：同上。
    pub(crate) fn prompt_texts(
        &self,
    ) -> std::sync::Arc<crate::capabilities::prompt::api::ToolTexts> {
        self.prompt.tools()
    }

    /// 生成期间"会话不在表里"的三种进入点共用这一句（错误文案要一致，别处不再各写一份）。
    fn running_refusal(sid: &str) -> String {
        format!("会话 {} 正在生成中：先「停止」或等它结束，再做这一步", sid)
    }

    /// 把单 agent 会话**交给工作线程**（核心表里留"生成中"）。
    /// 取出的窗口内，核心队列是空的——读接口与其它会话的命令因此照常。
    pub(crate) fn take_single(
        &mut self,
        sid: &str,
    ) -> Result<crate::capabilities::session::api::AgentSession, String> {
        // 运行态闸门：暂停 / 关闭的会话不启动任何生成（唤醒与派发都从这里过）。
        self.dispatch_gate(sid)?;
        if self.running.contains(sid) {
            return Err(Self::running_refusal(sid));
        }
        match self.sessions.remove(sid) {
            Some(Session::Single(s)) => {
                self.running.insert(sid.to_string());
                Ok(s)
            }
            Some(other) => {
                // 不是单 agent：原样放回，交给协作那条路（B-1 只搬单 agent）。
                self.sessions.insert(sid.to_string(), other);
                Err("该会话不是单 agent 模式".to_string())
            }
            None => Err("无此会话".to_string()),
        }
    }

    /// 把协作会话**交给工作线程**（核心表里留"生成中"）。
    /// 会话还没装进内存时先从落盘重建：协作的"继续"可能先于"打开"到达。
    pub(crate) fn take_collab(&mut self, sid: &str) -> Result<CollabSession, String> {
        // 同上：运行态闸门是第一道，先于"取出来"。
        self.dispatch_gate(sid)?;
        if self.running.contains(sid) {
            return Err(Self::running_refusal(sid));
        }
        if !self.sessions.contains_key(sid) {
            self.ensure_session(sid)?;
        }
        match self.sessions.remove(sid) {
            Some(Session::Collab(c)) => {
                self.running.insert(sid.to_string());
                Ok(c)
            }
            Some(other) => {
                self.sessions.insert(sid.to_string(), other);
                Err("该会话不是协作模式".to_string())
            }
            None => Err("无此会话".to_string()),
        }
    }

    /// 协作生成结束**交回**：重新插入 + 解除"生成中"，并**为就绪节点派发子会话**。
    /// 转录已由工作线程按"一次模型调用"的粒度增量落盘（见 `Persister`），这里不重复落。
    /// 返回值：派发产生的事件（调用方负责入台）。
    pub(crate) fn put_collab(
        &mut self,
        sid: &str,
        c: CollabSession,
    ) -> (Vec<SessionEvent>, Vec<(String, String, String)>) {
        self.running.remove(sid);
        self.sessions.insert(sid.to_string(), Session::Collab(c));
        self.spawn_ready_nodes(sid)
    }

    /// 方案过审后：链里**就绪且还没有子会话**的节点各建一个子会话，并如实外送。
    /// 为什么在这里：会话表只有核心能碰（泵不建会话）；派发是核心的职责。
    /// 子会话的产出 = 它**用 submit_report 工具交的回报**（核心操作走工具调用，不认正文里的 JSON）。
    /// 没交回报就退回"最后一条转录行"（如实取到的东西），仍取不到就说"没有产出"。
    fn node_note(&self, child: &str) -> String {
        let evs = match self.history.load(child) {
            Ok((_, evs)) => evs,
            Err(_) => Vec::new(),
        };
        let mut lines: Vec<String> = Vec::new();
        let mut reported: Option<String> = None;
        for ev in &evs {
            let Some(ls) = ev.get("lines").and_then(|l| l.as_array()) else {
                continue;
            };
            for l in ls {
                if let Some(t) = l.get("line").and_then(|x| x.as_str()) {
                    lines.push(t.to_string());
                }
                let Some(tool) = l.get("tool") else { continue };
                let name = tool.get("name").and_then(|x| x.as_str()).unwrap_or("");
                if name != crate::capabilities::tools::api::REPORT {
                    continue;
                }
                let out = tool
                    .get("output")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                if !out.trim().is_empty() {
                    reported = Some(out);
                }
            }
        }
        reported
            .or_else(|| lines.last().cloned())
            .unwrap_or_else(|| "（该节点没有产出）".to_string())
    }

    /// 标记节点完成（记下它的产出），供父会话做总验收。
    fn mark_node_done(&mut self, sid: &str, node: &str, note: &str) {
        if let Some(Session::Collab(c)) = self.sessions.get_mut(sid) {
            c.mark_node_done(node, note);
        }
    }

    /// 压缩一个会话的上下文：**让 AI 自己压**（核心只给 compact 工具），不是系统替它总结。
    /// 返回（提示词, compact 的声明, 压缩点, 本回合的身份块）——模型调用由调用方在工作线程上跑（界面不被阻塞）。
    pub fn compact_plan(
        &self,
        sid: &str,
    ) -> Result<
        (
            String,
            Option<crate::capabilities::llm::api::ToolDecl>,
            u64,
            String,
        ),
        String,
    > {
        let compact_prompt = self.prompt.tools().compact_prompt.clone();
        let decl = self
            .systools
            .book()
            .get("compact")
            .map(|t| t.decl("compact"));
        // 会话不在世或不是单 agent：**报错**，不给出"空身份块 + 压缩点 0"这种假计划
        // （压缩只对单 agent 会话成立；协作会话由各成员会话按阈值自己压）。
        let Some(Session::Single(s)) = self.sessions.get(sid) else {
            return Err(format!(
                "压缩只支持单 agent 会话（{} 不在世或不是单 agent）",
                sid
            ));
        };
        let up_to = s.next_line_id();
        let identity = s.params().identity(&*self.prompt, s.tool_mode());
        Ok((compact_prompt, decl, up_to, identity))
    }

    /// 往某个会话注入一条**系统消息**（讨论的提醒走这条；见 session-model.md 二"系统消息"）。
    /// 返回要外送/落盘的事件。
    pub fn note_system(&mut self, sid: &str, text: &str) -> Result<Vec<SessionEvent>, String> {
        let mut s = self.take_single(sid)?;
        let events = s.note_system(text);
        self.put_single(sid, s);
        self.persister(sid).persist(&events);
        Ok(events)
    }

    /// 给某个会话接下来的行打上**整场工作的下一个回合 id**（节点执行也用同一套编号）。
    /// 一个 agent 一个会话：它的回合计数来自父会话——回档同步靠两边同一套编号。
    pub(crate) fn bump_turn_of_child(&mut self, child: &str) -> u64 {
        let parent = self
            .history
            .load(child)
            .ok()
            .and_then(|(m, _)| m.parent.clone());
        let Some(parent) = parent else {
            return 0;
        };
        let tid = match self.sessions.get_mut(&parent) {
            Some(Session::Collab(c)) => c.next_turn_id(),
            _ => 0,
        };
        if let Some(Session::Single(s)) = self.sessions.get_mut(child) {
            s.set_turn(tid);
        }
        tid
    }

    // ---- 会话历史 ----
}

// 测试访问器：验证职责提示词已种入历史首条（回归：会话曾丢失 system 提示词）。
#[cfg(test)]
impl Conductor {
    pub fn single_history(&self, sid: &str) -> Option<Vec<Msg>> {
        match self.sessions.get(sid) {
            Some(Session::Single(s)) => Some(s.dialogue().to_vec()),
            _ => None,
        }
    }

    /// 测试用：往某个会话的流水追加事件（造「压过之后再重启」这类历史）。
    pub fn history_append(
        &mut self,
        sid: &str,
        events: &[serde_json::Value],
    ) -> Result<(), String> {
        self.history.append(sid, events)
    }

    /// 该会话此刻的**身份块**（按当前提示词册与形态现渲染）：测试用它断言"它被告诉了什么"。
    pub fn single_identity(&self, sid: &str) -> Option<String> {
        match self.sessions.get(sid) {
            Some(Session::Single(s)) => Some(s.params().identity(&*self.prompt, s.tool_mode())),
            _ => None,
        }
    }
}

/// 继续被拦下时的提醒（单 agent 会话：该轮到用户发言）。
const NEED_USER: &str = "最后一条是 AI 发言：部分供应商不允许连续 AI 发言；请先发言再继续。";

/// 会话列表按**树序**排：顶层保持后端给的时间倒序，每个会话后面**紧跟**它的子会话
/// （子会话内部同样按时间倒序），深度优先。
/// 为什么必须在后端做：顺序与缩进是同一件事的两面——前端各排各的，就会出现子会话排在父会话上面。
pub(crate) fn tree_order(list: Vec<HistoryView>) -> Vec<HistoryView> {
    let mut kids: std::collections::BTreeMap<String, Vec<HistoryView>> =
        std::collections::BTreeMap::new();
    let mut roots: Vec<HistoryView> = Vec::new();
    for h in list {
        match h.parent.clone() {
            Some(p) => kids.entry(p).or_default().push(h),
            None => roots.push(h),
        }
    }
    let mut out: Vec<HistoryView> = Vec::new();
    for r in roots {
        let mut stack = vec![r];
        while let Some(cur) = stack.pop() {
            let mut mine = kids.remove(&cur.name).unwrap_or_default();
            mine.sort_by_key(|h| std::cmp::Reverse(h.ts));
            // 逆序压栈 → 弹出时按时间倒序（与顶层同一口径）。
            for k in mine.into_iter().rev() {
                stack.push(k);
            }
            out.push(cur);
        }
    }
    // 父会话已不在（被删）的子会话：如实列在最后，不能凭空消失。
    for (_, v) in kids {
        out.extend(v);
    }
    out
}

/// 会话是否「已经开过」（流水里有内容）：agent 名单与形态据此冻结。配置记录与回档记录不算内容。
fn session_started(events: &[serde_json::Value]) -> bool {
    events
        .iter()
        .any(|ev| match ev.get("type").and_then(|t| t.as_str()) {
            Some("config") | Some("rewind") => false,
            Some("transcript") => ev
                .get("lines")
                .and_then(|l| l.as_array())
                .map(|l| !l.is_empty())
                .unwrap_or(false),
            Some(_) => true,
            None => false,
        })
}

/// 工作形态 → 会话元信息里的标识。
fn mode_str(mode: WorkMode) -> &'static str {
    match mode {
        WorkMode::Single => "single",
        WorkMode::Collab => "collab",
    }
}

/// 当前时间戳（秒）。
fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 工作名即将来的落盘目录名：非空、不含路径分隔符与 Windows 非法字符、不是保留名。
fn validate_work_name(name: &str) -> Result<(), String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("工作名不能为空".to_string());
    }
    if name != n {
        return Err("工作名首尾不能有空白".to_string());
    }
    if n == "." || n == ".." {
        return Err("工作名非法".to_string());
    }
    const BAD: [char; 9] = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    if n.chars().any(|c| BAD.contains(&c) || c.is_control()) {
        return Err("工作名不能包含 \\ / : * ? \" < > | 或控制字符".to_string());
    }
    let upper = n.to_ascii_uppercase();
    let reserved_device = ["CON", "PRN", "AUX", "NUL"].contains(&upper.as_str())
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit());
    if reserved_device {
        return Err("工作名是系统保留名".to_string());
    }
    Ok(())
}
