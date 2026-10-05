//! 测试脚手架：造会话 / 造名单 / 内存替身 / 脚本通道（原 `tests/core.rs` 的非测试部分）。
//! 归属：这些**不属于任何一个能力**——它们是测试装配，谁都可以用（与 `doubles.rs` 同一角色）。
use super::prelude::*;

/// 从事件流里抽出转录行并**渲染成文本**（行怎么变文本只有 LineView::render 一处）。
pub(crate) fn replay_lines(events: &[serde_json::Value]) -> Vec<String> {
    events
        .iter()
        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("transcript"))
        .flat_map(|e| {
            e.get("lines")
                .and_then(|l| l.as_array())
                .cloned()
                .unwrap_or_default()
        })
        .map(|l| {
            serde_json::from_value::<crate::capabilities::session::api::LineView>(l)
                .map(|v| v.render())
        })
        .filter_map(Result::ok)
        .collect()
}

/// 事件流里的转录行视图：(id, line, 是否是 tool 行)。
pub(crate) fn transcript_rows(events: &[SessionEvent]) -> Vec<(u64, String, bool)> {
    events
        .iter()
        .flat_map(|e| match e {
            SessionEvent::Transcript(ls) => ls
                .iter()
                .map(|l| (l.id, l.render(), l.tool.is_some()))
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect()
}

/// 事件流里的工具调用视图（tool 行携带的那个）。
pub(crate) fn tool_views(
    events: &[SessionEvent],
) -> Vec<crate::capabilities::session::api::ToolCallView> {
    events
        .iter()
        .flat_map(|e| match e {
            SessionEvent::Transcript(ls) => {
                ls.iter().filter_map(|l| l.tool.clone()).collect::<Vec<_>>()
            }
            _ => Vec::new(),
        })
        .collect()
}

/// 事件流里 tool 行的行文本（给人看的那一行）。
pub(crate) fn tool_line_texts(events: &[SessionEvent]) -> Vec<String> {
    events
        .iter()
        .flat_map(|e| match e {
            SessionEvent::Transcript(ls) => ls
                .iter()
                .filter(|l| l.tool.is_some())
                .map(|l| l.render())
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect()
}

/// 测试用的"逐成员跑一遍"：内部走**共享的 converse_with**（工具循环本身仍在生产里，
/// 只是执行阶段的入口从平铺的 Execution::run 换成了链驱动）。
pub(crate) struct ExecLike {
    pub reports: BTreeMap<String, String>,
    pub traces: BTreeMap<String, Vec<crate::capabilities::session::api::ToolCallView>>,
}

pub(crate) fn run_execution(
    members: &mut [crate::capabilities::collab::service::discussion::Member],
    tasks: &str,
    prompt: &dyn Prompt,
) -> ExecLike {
    let mut out = ExecLike {
        reports: BTreeMap::new(),
        traces: BTreeMap::new(),
    };
    for m in members.iter_mut() {
        if !m.present {
            continue;
        }
        let identity = m.params.identity(prompt, m.mode);
        let id = m.id.clone();
        let user = prompt.render(
            Segment::ExecuteUser,
            &[("tasks", tasks.to_string()), ("rework", String::new())],
        );
        let mut views = Vec::new();
        let mut noop = |_c: crate::capabilities::llm::api::Chunk| true;
        let mut sink = |_e: crate::capabilities::session::api::SessionEvent| {};
        let rounds = crate::capabilities::collab::service::round::converse_with(
            m.chat.as_mut().expect("测试通道").as_mut(),
            m.tools.as_mut(),
            &identity,
            vec![crate::capabilities::llm::api::Msg::user(user)],
            Default::default(),
            &id,
            &mut noop,
            &mut |v: &crate::capabilities::session::api::ToolCallView| views.push(v.clone()),
            &mut |_r: &crate::capabilities::collab::service::round::Round,
                  _s: &mut dyn FnMut(crate::capabilities::session::api::SessionEvent)| {},
            &mut sink,
            // 测试不接工具级确认：直接执行（确认路径由 driver 的用例单独钉）。
            &mut |_req: &crate::capabilities::collab::service::tool_loop::ToolConfirm,
                  _sink: &mut dyn FnMut(crate::capabilities::session::api::SessionEvent)|
             -> bool { true },
            &[],
            false,
        );
        let text = rounds.last().map(|r| r.text.clone()).unwrap_or_default();
        out.reports.insert(id.clone(), text);
        out.traces.insert(id, views);
    }
    out
}

// ---------- 讨论引擎 ----------

// ---------- 出站调用的全局参数（流式 / 预算 / 失败中断） ----------

/// 记录调用参数、并能按脚本失败的通道替身。
/// 用来钉两件机器可判的事实：**流式与预算来自全局设置**、**调用失败 = 中断而不是发言**。
pub(crate) struct OptsChat {
    pub(crate) seen: OptsLog,
    /// 每次调用的结果：None = 回一个 agree 信封；Some(原因) = 失败。
    pub(crate) results: Vec<Option<String>>,
}

/// 调用参数账本：(流式, 预算秒)。
pub(crate) type OptsLog = Arc<Mutex<Vec<(bool, u64)>>>;

impl crate::capabilities::llm::api::Chat for OptsChat {
    fn complete(
        &mut self,
        _m: &[crate::capabilities::llm::api::Msg],
        opts: crate::capabilities::llm::api::CompleteOpts<'_>,
        _on: &mut dyn FnMut(crate::capabilities::llm::api::Chunk) -> bool,
    ) -> crate::capabilities::llm::api::Completion {
        self.seen
            .lock()
            .expect("锁")
            .push((opts.stream, opts.timeout_secs));
        let r = if self.results.len() > 1 {
            self.results.remove(0)
        } else {
            self.results.first().cloned().unwrap_or(None)
        };
        match r {
            Some(reason) => crate::capabilities::llm::api::Completion::failure(reason),
            // 默认回**发言**而不是同意：同意是粘住的，开场就同意会让后面几轮被跳过，
            // 那些用例（预算 / 失败中断）要的是"还在讨论中"。
            None => crate::capabilities::llm::api::Completion::text(
                "{\"type\":\"say\",\"text\":\"我先说\"}",
            ),
        }
    }
}

pub(crate) fn opts_discussion(
    results: Vec<Option<String>>,
    llm: crate::capabilities::llm::api::LlmOpts,
) -> (Discussion, OptsLog) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let members = vec![Member::new(
        "m0",
        crate::tests::doubles::test_params("m0"),
        crate::capabilities::llm::api::ToolMode::Envelope,
        Box::new(OptsChat {
            seen: Arc::clone(&seen),
            results,
        }),
    )];
    (
        Discussion::new(
            members,
            false,
            std::sync::Arc::new(test_prompts()),
            test_tools_svc(),
            llm,
            Default::default(),
            String::new(),
        ),
        seen,
    )
}

pub(crate) fn scripted_discussion(scripts: Vec<Vec<String>>, allow: bool) -> Discussion {
    let members: Vec<Member> = scripts
        .into_iter()
        .enumerate()
        .map(|(i, s)| {
            let id = format!("m{}", i);
            Member::new(
                &id,
                crate::tests::doubles::test_params(&id),
                crate::capabilities::llm::api::ToolMode::Envelope,
                scripted(s),
            )
        })
        .collect();
    Discussion::new(
        members,
        allow,
        std::sync::Arc::new(test_prompts()),
        test_tools_svc(),
        Default::default(),
        Default::default(),
        String::new(),
    )
}

pub(crate) fn chain_node(id: &str, deps: &[&str]) -> crate::capabilities::taskchain::api::TaskNode {
    crate::capabilities::taskchain::api::TaskNode {
        id: id.to_string(),
        title: format!("节点{}", id),
        objective: format!("把 {} 做完", id),
        assignee: "甲".to_string(),
        deps: deps.iter().map(|d| d.to_string()).collect(),
        status: crate::capabilities::taskchain::api::NodeStatus::Pending,
        sub_session: None,
        report: None,
        acceptance: None,
        reported: false,
    }
}

pub(crate) fn roster() -> Vec<String> {
    vec!["甲".to_string(), "乙".to_string()]
}

/// 守护 runner：任何调用即失败（守护不该用工具的路径）。
pub(crate) struct SilentRunner;

impl ToolRunner for SilentRunner {
    fn run(
        &self,
        _fence: &crate::capabilities::tools::api::FenceSpec,
        _command: &str,
        _args: &str,
    ) -> ToolOutcome {
        panic!("不应调用工具");
    }
}

/// 记录型 runner：记录 (root, command, args)，回放固定输出。
pub(crate) struct RecordingRunner {
    pub(crate) calls: Mutex<Vec<(PathBuf, String, String)>>,
    pub(crate) out: String,
    pub(crate) ok: bool,
}

impl RecordingRunner {
    pub(crate) fn new(out: &str, ok: bool) -> RecordingRunner {
        RecordingRunner {
            calls: Mutex::new(Vec::new()),
            out: out.to_string(),
            ok,
        }
    }
}

impl ToolRunner for RecordingRunner {
    fn run(
        &self,
        fence: &crate::capabilities::tools::api::FenceSpec,
        command: &str,
        args_json: &str,
    ) -> ToolOutcome {
        // 记下工具进程的工作目录（= 该模块的根）与命令、参数。
        self.calls.lock().expect("锁").push((
            fence.cwd.clone(),
            command.to_string(),
            args_json.to_string(),
        ));
        ToolOutcome {
            ok: self.ok,
            output: self.out.clone(),
        }
    }
}

/// 并发记录型 runner：记录**同时在跑**的调用数峰值（模块工具是否真的并发，只有它能作证）。
pub(crate) struct ParallelRunner {
    pub(crate) active: AtomicUsize,
    pub(crate) peak: AtomicUsize,
    pub(crate) delay_ms: u64,
}

impl ParallelRunner {
    pub(crate) fn new(delay_ms: u64) -> ParallelRunner {
        ParallelRunner {
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            delay_ms,
        }
    }

    /// 同时在跑的峰值（串行恒为 1）。
    pub(crate) fn peak_concurrent(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }
}

impl ToolRunner for ParallelRunner {
    fn run(
        &self,
        _fence: &crate::capabilities::tools::api::FenceSpec,
        command: &str,
        args_json: &str,
    ) -> ToolOutcome {
        let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        if self.delay_ms > 0 {
            std::thread::sleep(Duration::from_millis(self.delay_ms));
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
        ToolOutcome {
            ok: true,
            output: format!("{} 跑完了 {}", command, args_json),
        }
    }
}

pub(crate) const TOOL_CALL: &str =
    "{\"type\":\"tool\",\"name\":\"grep\",\"args\":{\"keyword\":\"x\"}}";

/// 带工具环境的成员：模块 m0 声明 grep → python tools/grep.py（cwd = 该模块目录）。
pub(crate) fn member_with_tools(
    id: &str,
    script: Vec<String>,
    runner: Arc<impl ToolRunner + Send + Sync + 'static>,
) -> Member {
    let mut commands = BTreeMap::new();
    commands.insert("grep".to_string(), "python tools/grep.py".to_string());
    let mut modules = BTreeMap::new();
    modules.insert(
        "m0".to_string(),
        ModuleTools {
            root: abs(&["mods", "root"]),
            commands,
            books: BTreeMap::new(),
            parallel: BTreeSet::new(),
        },
    );
    let mut m = Member::new(
        id,
        crate::tests::doubles::test_params(id),
        crate::capabilities::llm::api::ToolMode::Envelope,
        scripted(script),
    );
    // 该路径走模块声明的外部命令（grep）：空沙箱 + 内存 IO，内置工具不参与。
    m.tools = Some(MemberTools {
        mode: crate::capabilities::llm::api::ToolMode::Envelope,
        modules,
        observations: crate::capabilities::tools::api::Observations::default(),
        llm: test_llm_demo(),
        log: Arc::new(crate::kernel::ports::NoopLog),
        tools: test_tools_svc_with(
            runner,
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        ),
        sandbox: test_sandbox("m0", &[]),
        builtin_tools: test_systools().tools,
        unavailable: BTreeMap::new(),
        fence: crate::capabilities::tools::api::FenceSpec::from_sandbox(
            &test_sandbox("m0", &[]),
            false,
        ),
        reply_seq: 0,
        line: Default::default(),
        // 测试替身按"执行席"发放全部内置工具（角色表的越权校验另有专门用例）。
        allowed: crate::capabilities::tools::api::names(),
        with_modules: true,
        notes: crate::tests::doubles::test_notes(&test_sandbox("m0", &[]), &[]),
        handlers: Vec::new(),
    });
    m
}

/// 真实的坏信封：结尾多了一个 ]（括号不配对）。路径用真实绝对路径（模型写对了路径、写坏了信封）。
/// 坏的形状照真案例：args 先闭合、多一个 ]、外层才闭合（整段不是合法 JSON）。
pub(crate) fn broken_tool(path: &str) -> String {
    format!(
        "{{\"type\":\"tool\",\"name\":\"write\",\"args\":{{\"path\":\"{}\",\"content\":\"hi\"}}]}}",
        path
    )
}

pub(crate) struct ReasoningChat;

impl Chat for ReasoningChat {
    fn complete(
        &mut self,
        _messages: &[Msg],
        _opts: CompleteOpts<'_>,
        _on: &mut dyn FnMut(Chunk) -> bool,
    ) -> Completion {
        Completion {
            raw: "{\"type\":\"say\",\"text\":\"完成\"}".to_string(),
            reasoning: "先思考".to_string(),
            finish: "stop".to_string(),
            calls: Vec::new(),
            error: None,
        }
    }
}

pub(crate) struct ReasoningGateway;

impl ChatGateway for ReasoningGateway {
    fn probe_tools(
        &self,
        _channel: &Channel,
    ) -> Result<crate::capabilities::llm::api::ProbeOutcome, String> {
        Err("测试替身没有真实供应商，测不了工具调用支持".to_string())
    }

    fn member_channel(
        &self,
        _channel: Option<&Channel>,
        _module_id: &str,
    ) -> (BoxedChat, Option<String>) {
        (Box::new(ReasoningChat), None)
    }

    fn core_channel(&self, _channel: Option<&Channel>) -> (BoxedChat, bool) {
        (Box::new(ReasoningChat), false)
    }
}

/// 被供应商按长度截断的通道替身：正文照发，但 finish_reason = length。
pub(crate) struct TruncChat {
    pub(crate) script: Vec<String>,
}

impl Chat for TruncChat {
    fn complete(
        &mut self,
        _m: &[Msg],
        _opts: CompleteOpts<'_>,
        _on: &mut dyn FnMut(Chunk) -> bool,
    ) -> Completion {
        let text = if self.script.len() > 1 {
            self.script.remove(0)
        } else {
            self.script.first().cloned().unwrap_or_default()
        };
        Completion {
            raw: text,
            reasoning: String::new(),
            finish: "length".to_string(),
            calls: Vec::new(),
            error: None,
        }
    }
}

/// 一律回"被截断"的网关（验核心能不能把截断与写错分开）。
pub(crate) struct TruncGateway {
    pub(crate) script: Vec<String>,
}

impl ChatGateway for TruncGateway {
    fn probe_tools(
        &self,
        _c: &Channel,
    ) -> Result<crate::capabilities::llm::api::ProbeOutcome, String> {
        Err("脚本替身没有真实供应商，测不了工具调用支持".to_string())
    }
    fn member_channel(&self, _c: Option<&Channel>, _id: &str) -> (BoxedChat, Option<String>) {
        (
            Box::new(TruncChat {
                script: self.script.clone(),
            }),
            None,
        )
    }
    fn core_channel(&self, _c: Option<&Channel>) -> (BoxedChat, bool) {
        (
            Box::new(TruncChat {
                script: self.script.clone(),
            }),
            false,
        )
    }
}

/// 被用户中止的通道：回调一律返回 false（调用方要求停止），但仍返回一段"只差一个括号"的信封。
pub(crate) struct AbortChat {
    pub(crate) raw: String,
}

impl Chat for AbortChat {
    fn complete(
        &mut self,
        _m: &[Msg],
        _opts: CompleteOpts<'_>,
        on: &mut dyn FnMut(Chunk) -> bool,
    ) -> Completion {
        // 已停止的生成：引擎侧的回调在收到第一片时就返回 false（用户点了「停止」），
        // 但**已经收到的内容照样返回**——半截信封正是这么来的（回调的裁决由引擎读，
        // 替身不替引擎决定要不要中止：中止与否是**取消标志**的事）。
        let _ = on(Chunk::Start);
        let _ = on(Chunk::Text(self.raw.clone()));
        Completion::text(self.raw.clone())
    }
}

pub(crate) struct AbortGateway {
    pub(crate) raw: String,
}

impl ChatGateway for AbortGateway {
    fn probe_tools(
        &self,
        _c: &Channel,
    ) -> Result<crate::capabilities::llm::api::ProbeOutcome, String> {
        Err("脚本替身没有真实供应商，测不了工具调用支持".to_string())
    }
    fn member_channel(&self, _c: Option<&Channel>, _id: &str) -> (BoxedChat, Option<String>) {
        (
            Box::new(AbortChat {
                raw: self.raw.clone(),
            }),
            None,
        )
    }
    fn core_channel(&self, _c: Option<&Channel>) -> (BoxedChat, bool) {
        (
            Box::new(AbortChat {
                raw: self.raw.clone(),
            }),
            false,
        )
    }
}

/// 固定探测结论的网关：专测"结论怎么落到登记处"这一层策略（事实本身由适配器测）。
pub(crate) struct ProbeGateway {
    pub(crate) outcome: Arc<Mutex<crate::capabilities::conductor::api::ProbeOutcome>>,
}

impl ChatGateway for ProbeGateway {
    fn probe_tools(
        &self,
        _c: &Channel,
    ) -> Result<crate::capabilities::llm::api::ProbeOutcome, String> {
        Ok(self.outcome.lock().expect("锁").clone())
    }
    fn member_channel(&self, _c: Option<&Channel>, _id: &str) -> (BoxedChat, Option<String>) {
        (
            scripted(vec!["{\"type\":\"say\",\"text\":\"收到\"}".to_string()]),
            None,
        )
    }
    fn core_channel(&self, _c: Option<&Channel>) -> (BoxedChat, bool) {
        (scripted(vec!["[]".to_string()]), true)
    }
}

/// 原生通道的脚本替身：一步 = 一次"原生工具调用"或一段文本；
/// 同时记录每次请求带过来的工具声明与消息（用来断言"声明真的发出去了、结果真的回填了"）。
pub(crate) enum NativeStep {
    Calls(Vec<crate::capabilities::llm::api::ToolCall>),
    Text(String),
}

/// 每次请求声明的工具（名字 + 参数 Schema）。
pub(crate) type DeclLog = Arc<Mutex<Vec<Vec<(String, serde_json::Value)>>>>;

/// 每次请求看到的消息。
pub(crate) type SeenLog = Arc<Mutex<Vec<Vec<Msg>>>>;

pub(crate) struct NativeChat {
    pub(crate) steps: Vec<NativeStep>,
    pub(crate) declared: DeclLog,
    pub(crate) seen: SeenLog,
}

impl Chat for NativeChat {
    fn complete(
        &mut self,
        messages: &[Msg],
        opts: CompleteOpts<'_>,
        _on: &mut dyn FnMut(Chunk) -> bool,
    ) -> Completion {
        self.seen.lock().expect("锁").push(messages.to_vec());
        let decls: Vec<(String, serde_json::Value)> = opts
            .tools
            .map(|ts| {
                ts.iter()
                    .map(|d| (d.name.clone(), d.parameters.clone()))
                    .collect()
            })
            .unwrap_or_default();
        self.declared.lock().expect("锁").push(decls);
        if self.steps.is_empty() {
            return Completion::text("{\"type\":\"say\",\"text\":\"脚本用完了\"}");
        }
        match self.steps.remove(0) {
            NativeStep::Calls(calls) => Completion {
                raw: String::new(),
                reasoning: String::new(),
                finish: "tool_calls".to_string(),
                calls,
                error: None,
            },
            NativeStep::Text(t) => Completion::text(t),
        }
    }
}

/// 原生形态的成员：沙箱用测试根，模块表为空，工具执行走内存 IO。
pub(crate) fn native_member(
    id: &str,
    io: Arc<InMemorySysIo>,
    steps: Vec<NativeStep>,
    declared: DeclLog,
    seen: SeenLog,
) -> Member {
    let chat: BoxedChat = Box::new(NativeChat {
        steps,
        declared,
        seen,
    });
    let mut m = Member::new(
        id,
        crate::tests::doubles::test_params(id),
        crate::capabilities::llm::api::ToolMode::Native,
        chat,
    );
    let mut modules = BTreeMap::new();
    modules.insert(
        "m0".to_string(),
        ModuleTools {
            root: abs(&["mods", "root"]),
            commands: BTreeMap::new(),
            books: BTreeMap::new(),
            parallel: BTreeSet::new(),
        },
    );
    let sb = test_sandbox(id, &[]);
    m.tools = Some(MemberTools {
        mode: crate::capabilities::llm::api::ToolMode::Native,
        modules,
        observations: crate::capabilities::tools::api::Observations::default(),
        llm: test_llm_demo(),
        log: Arc::new(crate::kernel::ports::NoopLog),
        tools: test_tools_svc_with(Arc::new(SilentRunner), io, Arc::new(NoFenceHost)),
        sandbox: sb.clone(),
        builtin_tools: test_systools().tools,
        unavailable: BTreeMap::new(),
        fence: crate::capabilities::tools::api::FenceSpec::from_sandbox(&sb, false),
        reply_seq: 0,
        line: Default::default(),
        // 测试替身按"执行席"发放全部内置工具（角色表的越权校验另有专门用例）。
        allowed: crate::capabilities::tools::api::names(),
        with_modules: true,
        notes: crate::tests::doubles::test_notes(&sb, &[]),
        handlers: Vec::new(),
    });
    m
}

/// 虚拟机档选型：基础根 + 不联网（定版留空 = 让库自己决定；多版本时报歧义）。
pub(crate) fn vm_spec() -> ExecSpec {
    ExecSpec {
        tier: Tier::Vm,
        base: Some("base-linux".to_string()),
        pins: BTreeMap::new(),
        net: false,
    }
}

/// 造一条 agent 名单记录。
pub(crate) fn agent_meta(name: &str, modules: &[&str], model: Option<&str>) -> AgentMeta {
    AgentMeta {
        name: name.to_string(),
        transient: false,
        modules: modules.iter().map(|s| s.to_string()).collect(),
        model: model.map(|s| s.to_string()),
        permissions: Default::default(),
    }
}

/// 直接把一份 meta 放进内存历史（模拟"重启后从盘上读回该会话"）。
pub(crate) fn seed_session(
    hist: &Arc<InMemoryHistory>,
    name: &str,
    mode: &str,
    agents: Vec<AgentMeta>,
    exec: ExecSpec,
) {
    hist.create(&SessionMeta {
        name: name.to_string(),
        mode: mode.to_string(),
        delegate: false,
        modules: agents.iter().flat_map(|a| a.modules.clone()).collect(),
        task: None,
        ts: 1,
        agents,
        exec,
        parent: None,
        node: None,
        delegation: None,
        run: RunState::Active,
    })
    .unwrap();
}

/// 一次编辑提交（名字 / 模块 / 模型；档位与定版默认本机档）。
pub(crate) fn edit_of(agents: Vec<(&str, &[&str], &str)>) -> SessionEdit {
    SessionEdit {
        agents: agents
            .into_iter()
            .map(|(n, ms, m)| ConfigAgent {
                name: n.to_string(),
                modules: ms.iter().map(|s| s.to_string()).collect(),
                model: m.to_string(),
                permissions: None,
            })
            .collect(),
        tier: "host".to_string(),
        base: None,
        pins: BTreeMap::new(),
        net: false,
    }
}

/// 原生形态的网关：每次要通道就弹出一份脚本（第一份给实时会话，第二份给重建）。
pub(crate) struct NativeGateway {
    pub(crate) scripts: Mutex<Vec<Vec<NativeStep>>>,
}

impl ChatGateway for NativeGateway {
    fn probe_tools(
        &self,
        _c: &Channel,
    ) -> Result<crate::capabilities::llm::api::ProbeOutcome, String> {
        Ok(crate::capabilities::llm::api::ProbeOutcome::Supported {
            detail: "替身".to_string(),
        })
    }
    fn member_channel(&self, _c: Option<&Channel>, _id: &str) -> (BoxedChat, Option<String>) {
        let steps = self.scripts.lock().expect("锁").pop().unwrap_or_default();
        (
            Box::new(NativeChat {
                steps,
                declared: Arc::new(Mutex::new(Vec::new())),
                seen: Arc::new(Mutex::new(Vec::new())),
            }),
            None,
        )
    }
    fn core_channel(&self, _c: Option<&Channel>) -> (BoxedChat, bool) {
        (Box::new(FakeChat::new(vec!["[]".to_string()])), false)
    }
}

/// 指定网关 + 指定落盘历史装配一个核心（重建用例要读同一份转录）。
pub(crate) fn native_core(
    gateway: NativeGateway,
    history: Arc<InMemoryHistory>,
    io: Arc<InMemorySysIo>,
) -> Conductor {
    let gateway: Arc<dyn ChatGateway + Send + Sync> = Arc::new(gateway);
    let llm = test_llm(
        Arc::clone(&gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    Conductor::new(
        registry_service(InMemorySettings::new(), Arc::clone(&llm)),
        test_history_of(history),
        test_workspace(
            Arc::new(VecSource(vec![module_of("a")])),
            Arc::new(InMemoryPackages::empty()),
            Arc::new(InMemoryWorkspace::new()),
        ),
        llm,
        test_tools_svc_with(Arc::new(SilentRunner), io, Arc::new(NoFenceHost)),
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        Arc::new(crate::kernel::detail::HostProbeAdapter),
    )
}
