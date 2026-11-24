//! 目的：通道替身——记录调用参数、按脚本回话或失败，覆盖流式、截断、中止与原生调用。
//! 管：Reasoning/Trunc/Abort/Probe/Native 各类网关与脚本。
//! 不管：内存替身库（`doubles/`）、runner（`runners.rs`）。
//! 联动：`src/capabilities/llm/ports.rs` 的 `ChatGateway` 与 `Chat`。

use super::*;

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
        Default::default(),
        String::new(),
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
            cancelled: false,
            usage: None,
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

    fn member_channel(&self, _channel: &Channel, _module_id: &str) -> BoxedChat {
        Box::new(ReasoningChat)
    }

    fn core_channel(&self, _channel: &Channel) -> BoxedChat {
        Box::new(ReasoningChat)
    }
}

/// 目的：被供应商按长度截断的通道替身：正文照发，但 finish_reason = length。
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
            cancelled: false,
            usage: None,
        }
    }
}

/// 目的：一律回"被截断"的网关（验核心能不能把截断与写错分开）。
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
    fn member_channel(&self, _c: &Channel, _id: &str) -> BoxedChat {
        Box::new(TruncChat {
            script: self.script.clone(),
        })
    }
    fn core_channel(&self, _c: &Channel) -> BoxedChat {
        Box::new(TruncChat {
            script: self.script.clone(),
        })
    }
}

/// 目的：被用户中止的通道：回调一律返回 false（调用方要求停止），但仍返回一段"只差一个括号"的信封。
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
        // 已停止的生成：引擎侧的回调在收到第一片时就返回 false（用户点了「停止」），但**已经收到的内容照样返回**——半截信封正是这么来的（回调的裁决由引擎读，替身不替引擎决定要不要中止：中止与否是**取消标志**的事）。
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
    fn member_channel(&self, _c: &Channel, _id: &str) -> BoxedChat {
        Box::new(AbortChat {
            raw: self.raw.clone(),
        })
    }
    fn core_channel(&self, _c: &Channel) -> BoxedChat {
        Box::new(AbortChat {
            raw: self.raw.clone(),
        })
    }
}

/// 目的：固定探测结论的网关：专测"结论怎么落到登记处"这一层策略（事实本身由适配器测）。
pub(crate) struct ProbeGateway {
    pub(crate) outcome: Arc<Mutex<crate::capabilities::runtime::api::ProbeOutcome>>,
}

impl ChatGateway for ProbeGateway {
    fn probe_tools(
        &self,
        _c: &Channel,
    ) -> Result<crate::capabilities::llm::api::ProbeOutcome, String> {
        Ok(self.outcome.lock().expect("锁").clone())
    }
    fn member_channel(&self, _c: &Channel, _id: &str) -> BoxedChat {
        scripted(vec!["{\"type\":\"say\",\"text\":\"收到\"}".to_string()])
    }
    fn core_channel(&self, _c: &Channel) -> BoxedChat {
        scripted(vec!["[]".to_string()])
    }
}

/// 目的：原生通道的脚本替身：一步 = 一次"原生工具调用"或一段文本； 同时记录每次请求带过来的工具声明与消息（用来断言"声明真的发出去了、结果真的回填了"）。
pub(crate) enum NativeStep {
    Calls(Vec<crate::capabilities::llm::api::ToolCall>),
    Text(String),
}

/// 目的：每次请求声明的工具（名字 + 参数 Schema）。
pub(crate) type DeclLog = Arc<Mutex<Vec<Vec<(String, serde_json::Value)>>>>;

/// 目的：每次请求看到的消息。
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
                cancelled: false,
                usage: None,
            },
            NativeStep::Text(t) => Completion::text(t),
        }
    }
}

/// 目的：原生形态的成员：沙箱用测试根，模块表为空，工具执行走内存 IO。
pub(crate) fn native_member(
    id: &str,
    io: Arc<InMemorySysIo>,
    steps: Vec<NativeStep>,
    declared: DeclLog,
    seen: SeenLog,
) -> WithTools {
    let chat: BoxedChat = Box::new(NativeChat {
        steps,
        declared,
        seen,
    });
    let m = Member::new(
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
            services: BTreeMap::new(),
        },
    );
    let sb = test_sandbox(id, &[]);
    WithTools {
        member: m,
        tools: Some(MemberTools {
            face: crate::capabilities::conversation::session::api::ToolFace {
                mode: crate::capabilities::llm::api::ToolMode::Native,
                seat: "solo".to_string(),
                modules,
                tier: crate::kernel::api::types::Tier::Host,
                guest_lease: String::new(),
                sandbox: sb.clone(),
                tools_this_turn: test_prompts()
                    .text(crate::capabilities::prompt::api::Segment::ToolsThisTurn)
                    .to_string(),
                builtin_tools: test_systools().tools,
                unavailable: BTreeMap::new(),
                fence: crate::kernel::api::fence::FencePlan::for_session(&sb, &[], false),
                module_env: Default::default(),
                runtime_path: None,
                runtime_roots: Vec::new(),
                // 测试替身按"执行席"发放全部内置工具（角色表的越权校验另有专门用例）。
                allowed: crate::capabilities::tools::api::names(),
                with_modules: true,
                notes: crate::tests::doubles::test_notes(&sb, &[]),
            },
            llm: test_llm_demo(),
            log: Arc::new(crate::kernel::ports::NoopLog),
            tools: test_tools_svc_with(Arc::new(SilentRunner), io, Arc::new(NoFenceHost)),
            vm: test_vm(),
            residents: Arc::new(crate::capabilities::residents::api::NoResidents),
            // 这些用例只跑工具面：写工具落地后不自动提交（没有工作区）。
            workspace: None,
            observations: crate::capabilities::tools::api::Observations::default(),
            shell_live: None,
            reply_seq: 0,
            line: Default::default(),
            handlers: Vec::new(),
        }),
    }
}

/// 目的：原生形态的网关：每次要通道就弹出一份脚本（第一份给实时会话，第二份给重建），并把每次请求看到的消息记进 `seen`。
/// 约束：`seen` 由调用方持有并读；同一个网关弹出的所有通道共这一本账。
pub(crate) struct NativeGateway {
    pub(crate) scripts: Mutex<Vec<Vec<NativeStep>>>,
    pub(crate) seen: SeenLog,
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

    /// 目的：保存闸门要同时看回放形状：这个替身固定报「协议形状被接受」。
    fn probe_replay(
        &self,
        _c: &Channel,
    ) -> Result<crate::capabilities::llm::api::ReplayReport, String> {
        Ok(crate::capabilities::llm::api::ReplayReport {
            shapes: vec![crate::capabilities::llm::api::ReplayShape {
                name: "content-empty".to_string(),
                accepted: true,
                understood: true,
                detail: "替身".to_string(),
            }],
        })
    }
    fn member_channel(&self, _c: &Channel, _id: &str) -> BoxedChat {
        let steps = self.scripts.lock().expect("锁").pop().unwrap_or_default();
        Box::new(NativeChat {
            steps,
            declared: Arc::new(Mutex::new(Vec::new())),
            seen: Arc::clone(&self.seen),
        })
    }
    fn core_channel(&self, _c: &Channel) -> BoxedChat {
        Box::new(FakeChat::new(vec!["[]".to_string()]))
    }
}

/// 目的：指定网关 + 指定落盘历史装配一个核心（重建用例要读同一份转录）。
pub(crate) fn native_core(
    gateway: NativeGateway,
    history: Arc<InMemoryHistory>,
    io: Arc<InMemorySysIo>,
) -> Core {
    let gateway: Arc<dyn ChatGateway + Send + Sync> = Arc::new(gateway);
    let llm = test_llm(
        Arc::clone(&gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    Core::new(
        crate::tests::root_dir(),
        registry_service(InMemorySettings::new(), Arc::clone(&llm)),
        test_history_of(history),
        test_workspace(Arc::new(InMemoryWorkspace::new())),
        test_vm(),
        test_packages(Arc::new(InMemoryPackages::empty())),
        test_provision(),
        test_modules(Arc::new(VecSource(vec![module_of("a")]))),
        Arc::new(crate::capabilities::maintenance::builds::api::NoBuilds),
        llm,
        test_tools_svc_with(Arc::new(SilentRunner), io, Arc::new(NoFenceHost)),
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        crate::tests::doubles::test_host_probe(),
        Arc::new(crate::capabilities::residents::api::NoResidents),
        Arc::new(crate::capabilities::catalog::secrets::api::NoSecrets),
        crate::tests::doubles::test_profiles(),
    )
}
