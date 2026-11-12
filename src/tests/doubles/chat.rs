//! 目的：通道脚本与提示词来源替身——按剧本回话、记录请求。
//! 管：文本与原生调用两条回放路径、令牌用量、提示词装载。
//! 不管：模型目录（`history.rs`）、登记处（`registry.rs`）。
//! 联动：`src/capabilities/llm/ports.rs` 的 `ChatGateway` 与 `src/capabilities/prompt/ports.rs`。

use super::*;

impl Chat for SharedScript {
    fn complete(
        &mut self,
        _messages: &[Msg],
        _opts: CompleteOpts<'_>,
        _on: &mut dyn FnMut(Chunk) -> bool,
    ) -> Completion {
        let mut q = self.q.lock().expect("脚本队列锁");
        let text = if q.len() > 1 {
            q.remove(0)
        } else {
            q.first().cloned().unwrap_or_default()
        };
        let mut done = Completion::text(text);
        done.usage = self.usage;
        done
    }

    fn last_usage(&self) -> Option<crate::capabilities::llm::api::TokenUsage> {
        self.usage
    }
}

/// 目的：模拟“信封通道里供应商返回了原生 tool_calls”——第一次给调用，第二次给普通正文，避免测试落进“每轮都返回同一个调用”的死循环。
pub(crate) struct NativeCallChat {
    pub name: String,
    pub args_json: String,
    pub raw: String,
    pub finish: String,
    pub(crate) sent: bool,
}

impl Chat for NativeCallChat {
    fn complete(
        &mut self,
        _messages: &[Msg],
        _opts: CompleteOpts<'_>,
        _on: &mut dyn FnMut(Chunk) -> bool,
    ) -> Completion {
        if self.sent {
            return Completion::text("我改用信封重发。");
        }
        self.sent = true;
        Completion {
            raw: self.raw.clone(),
            reasoning: String::new(),
            finish: self.finish.clone(),
            calls: vec![crate::capabilities::llm::api::ToolCall {
                id: "call-native-1".to_string(),
                name: self.name.clone(),
                args_json: self.args_json.clone(),
            }],
            error: None,
            cancelled: false,
            usage: None,
        }
    }
}

/// 目的：给成员通道发 NativeCallChat 的网关；核心通道不用。
pub(crate) struct NativeCallGateway {
    pub name: String,
    pub args_json: String,
    pub raw: String,
}

impl ChatGateway for NativeCallGateway {
    fn member_channel(&self, _channel: &Channel, _module_id: &str) -> BoxedChat {
        Box::new(NativeCallChat {
            name: self.name.clone(),
            args_json: self.args_json.clone(),
            raw: self.raw.clone(),
            finish: "tool_calls".to_string(),
            sent: false,
        })
    }
    fn core_channel(&self, _channel: &Channel) -> BoxedChat {
        Box::new(FakeChat::new(vec!["[]".to_string()]))
    }
    fn probe_tools(
        &self,
        _channel: &Channel,
    ) -> Result<crate::capabilities::llm::api::ProbeOutcome, String> {
        Err("脚本替身没有真实供应商，测不了工具调用支持".to_string())
    }
}

/// 目的：脚本网关：按 agent 实例名回放各自脚本；核心通道走共享队列。
pub(crate) struct ScriptGateway {
    pub(crate) member: BTreeMap<String, Vec<String>>,
    pub(crate) core: Arc<Mutex<Vec<String>>>,
    // 需要验“精确上下文用量”时注入；None = 这条通道拿不到 usage。
    pub(crate) usage: Option<crate::capabilities::llm::api::TokenUsage>,
}

impl ScriptGateway {
    pub(crate) fn new(member: BTreeMap<String, Vec<String>>, core: Vec<String>) -> ScriptGateway {
        ScriptGateway {
            member,
            core: Arc::new(Mutex::new(core)),
            usage: None,
        }
    }

    /// 目的：让这个脚本网关的每次调用都报同一份 usage（验状态栏 / 压缩阈值用）。
    pub(crate) fn with_usage(
        mut self,
        usage: crate::capabilities::llm::api::TokenUsage,
    ) -> ScriptGateway {
        self.usage = Some(usage);
        self
    }
}

impl ChatGateway for ScriptGateway {
    fn probe_tools(
        &self,
        _c: &Channel,
    ) -> Result<crate::capabilities::llm::api::ProbeOutcome, String> {
        Err("脚本替身没有真实供应商，测不了工具调用支持".to_string())
    }
    fn member_channel(&self, _c: &Channel, id: &str) -> BoxedChat {
        let script =
            self.member.get(id).cloned().unwrap_or_else(|| {
                vec!["{\"type\":\"say\",\"text\":\"（演示）收到。\"}".to_string()]
            });
        match self.usage {
            Some(u) => Box::new(FakeChat::new(script).with_usage(u)),
            None => scripted(script),
        }
    }
    fn core_channel(&self, _c: &Channel) -> BoxedChat {
        Box::new(SharedScript {
            q: Arc::clone(&self.core),
            usage: self.usage,
        })
    }
}

/// 目的：提示词册替身：默认回放内置册子；fail_with 注入加载失败。
pub(crate) struct TestPrompts {
    pub(crate) fail: Option<String>,
}

impl TestPrompts {
    pub(crate) fn ok() -> TestPrompts {
        TestPrompts { fail: None }
    }
    pub(crate) fn fail_with(mut self, msg: &str) -> TestPrompts {
        self.fail = Some(msg.to_string());
        self
    }
}

impl PromptSource for TestPrompts {
    fn load(&self) -> Result<Prompts, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        Ok(test_prompts())
    }
}

pub(crate) fn test_prompts() -> Prompts {
    // 走**与产品同一条**装配路径（目录 + 合并）：替身与真机装配出同一册子，测试才有意义。
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    crate::capabilities::prompt::detail::yaml_prompts::YamlPrompts::new(root.join("prompts"))
        .load()
        .expect("内置提示词册必须合法")
}

/// 目的：测试用工具总表与角色表（走**与产品同一条**装配路径）。它与提示词册**分开**装配：两者互不依赖（见 capabilities/prompt 的 Prompts）。测试用的**会话历史面**（与生产同一条路：端口装进 `SessionService`）。
pub(crate) fn test_history() -> Arc<dyn crate::capabilities::session::api::History + Send + Sync> {
    test_history_of(Arc::new(InMemoryHistory::new()))
}

/// 目的：同上，但用调用方给的替身（要在断言里看落盘内容的用例用它——同一份状态）。
pub(crate) fn test_history_of(
    store: Arc<InMemoryHistory>,
) -> Arc<dyn crate::capabilities::session::api::History + Send + Sync> {
    Arc::new(crate::capabilities::session::service::SessionService::new(
        store,
    ))
}

/// 目的：测试用的 **tools 能力**：三张表 + 三个替身端口（与生产同一条路，R12）。两个面都从这里出：`Arc<dyn Tools>`（表）与 `Arc<dyn ToolExec>`（执行）。
pub(crate) fn test_tools_svc() -> Arc<crate::capabilities::tools::service::ToolsService> {
    test_tools_svc_with(
        Arc::new(SilentRunner),
        Arc::new(InMemorySysIo::new()),
        Arc::new(NoFenceHost),
    )
}

/// 目的：同上，但指定三个端口（断言并发/落盘/撤权的那几条用例用）。
pub(crate) fn test_tools_svc_with(
    runner: Arc<dyn crate::kernel::ports::ProcessRunner>,
    io: Arc<InMemorySysIo>,
    fence: Arc<dyn crate::kernel::ports::FenceHost + Send + Sync>,
) -> Arc<crate::capabilities::tools::service::ToolsService> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source =
        crate::capabilities::tools::detail::yaml_systools::YamlSystools::new(root.join("systools"));
    Arc::new(
        crate::capabilities::tools::service::ToolsService::new(&source, runner, io, fence)
            .expect("内置工具总表必须合法"),
    )
}

pub(crate) fn test_systools() -> crate::capabilities::tools::api::SystemTools {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    crate::capabilities::tools::detail::yaml_systools::YamlSystools::new(root.join("systools"))
        .load()
        .expect("内置工具总表必须合法")
}

/// 目的：宿主探测替身：**只按给定答案回答**，不读真实环境（测试要确定性）。事实由用例显式声明；真实适配器的契约另有 T2 用例（见 docs/testing/doubles.md 端口矩阵）。
#[derive(Default)]
pub(crate) struct FixedProbe {
    pub files: Vec<std::path::PathBuf>,
    pub dirs: Vec<std::path::PathBuf>,
    pub exes: Vec<String>,
    pub hypervisor: bool,
    // 在场的**虚拟根目录**（`.vm/guest/<名>`）——路径落在它下面也算在场（目录内容由用例声明的文件表达）。
    pub guests: Vec<std::path::PathBuf>,
    // `files` 里"在但**不可执行**"的那些（权限位丢了的场景）——真实适配器按可执行位判。
    pub not_executable: Vec<std::path::PathBuf>,
}
