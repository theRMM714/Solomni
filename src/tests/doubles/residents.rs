//! 目的：常驻服务、日志与隐秘的替身——注入服务面、记录日志、内存隐秘表。
//! 管：服务启动与调用、停不下来的适配器、日志记录、隐秘读写。
//! 不管：真实服务进程（`src/capabilities/residents/detail/`）。
//! 联动：`src/capabilities/residents/ports.rs` 与 `src/capabilities/secrets/ports.rs`。

use super::*;

impl FakeServiceAdapter {
    pub(crate) fn new() -> FakeServiceAdapter {
        FakeServiceAdapter {
            calls: Arc::new(Mutex::new(Vec::new())),
            envs: Mutex::new(Vec::new()),
            paths: Mutex::new(Vec::new()),
            roots: Mutex::new(Vec::new()),
        }
    }
    /// 目的：start 时收到的注入项（断言 secrets 注入用）。
    pub(crate) fn envs(&self) -> Vec<(String, String)> {
        self.envs.lock().expect("锁").clone()
    }
    /// 目的：start 时收到的运行包 PATH 与围栏只读根。
    pub(crate) fn paths(&self) -> Vec<Option<std::ffi::OsString>> {
        self.paths.lock().expect("锁").clone()
    }
    pub(crate) fn roots(&self) -> Vec<Vec<std::path::PathBuf>> {
        self.roots.lock().expect("锁").clone()
    }
}

struct FakeServiceInstance {
    calls: Arc<Mutex<Vec<String>>>,
    env: Vec<(String, String)>,
}

impl crate::capabilities::residents::ports::ServiceInstance for FakeServiceInstance {
    fn call(&mut self, op: &str, args: &serde_json::Value) -> Result<String, String> {
        self.calls.lock().expect("锁").push(format!("call:{}", op));
        let env: Vec<String> = self
            .env
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        Ok(format!("{} {} {}", op, args, env.join(",")))
    }
    fn stop(&mut self) {
        self.calls.lock().expect("锁").push("stop".to_string());
    }
}

impl crate::capabilities::residents::ports::ServiceAdapter for FakeServiceAdapter {
    fn id(&self) -> &str {
        "fake"
    }
    fn start(
        &self,
        spec: &crate::capabilities::residents::ports::LaunchSpec,
    ) -> Result<
        (
            Box<dyn crate::capabilities::residents::ports::ServiceInstance>,
            Vec<crate::capabilities::residents::api::Operation>,
        ),
        String,
    > {
        self.calls
            .lock()
            .expect("锁")
            .push(format!("start:{}", spec.name));
        self.envs.lock().expect("锁").extend(spec.env.clone());
        self.paths.lock().expect("锁").push(spec.path.clone());
        self.roots
            .lock()
            .expect("锁")
            .push(spec.fence.ro_tree.clone());
        Ok((
            Box::new(FakeServiceInstance {
                calls: Arc::clone(&self.calls),
                env: spec.env.clone(),
            }),
            vec![crate::capabilities::residents::api::Operation {
                name: "echo".to_string(),
                description: "回显参数".to_string(),
                // 带一份 JSON Schema：工具面"原样声明"的断言要用它。
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": { "x": { "type": "integer", "description": "一个整数" } },
                    "required": ["x"]
                })),
            }],
        ))
    }
}

/// 目的：一启动就"死"的假适配器——实例调用恒失败且 is_alive = false（崩溃检测用例用）。
pub(crate) struct DyingServiceAdapter;

impl crate::capabilities::residents::ports::ServiceAdapter for DyingServiceAdapter {
    fn id(&self) -> &str {
        "dying"
    }
    fn start(
        &self,
        _spec: &crate::capabilities::residents::ports::LaunchSpec,
    ) -> Result<
        (
            Box<dyn crate::capabilities::residents::ports::ServiceInstance>,
            Vec<crate::capabilities::residents::api::Operation>,
        ),
        String,
    > {
        Ok((
            Box::new(DyingServiceInstance),
            vec![crate::capabilities::residents::api::Operation {
                name: "boom".to_string(),
                description: String::new(),
                parameters: None,
            }],
        ))
    }
}

struct DyingServiceInstance;

impl crate::capabilities::residents::ports::ServiceInstance for DyingServiceInstance {
    fn call(&mut self, _op: &str, _args: &serde_json::Value) -> Result<String, String> {
        Err("服务进程已结束（EOF）".to_string())
    }
    fn stop(&mut self) {}
    fn is_alive(&self) -> bool {
        false
    }
}

/// 目的：记录型日志 Spy——按顺序记下 (级别, 位置, 文案)，供"事实被记进日志"的断言。
#[derive(Default)]
pub(crate) struct RecordingLog {
    pub(crate) lines: std::sync::Mutex<Vec<(String, String, String)>>,
}

impl RecordingLog {
    /// 目的：空记录。
    pub(crate) fn new() -> RecordingLog {
        RecordingLog::default()
    }
    /// 目的：全部记录里是否有一条文案包含给定片段。
    pub(crate) fn has(&self, needle: &str) -> bool {
        self.lines
            .lock()
            .expect("锁")
            .iter()
            .any(|(_, _, m)| m.contains(needle))
    }
}

impl crate::kernel::ports::Log for RecordingLog {
    fn info(&self, at: &str, msg: &str) {
        self.lines
            .lock()
            .expect("锁")
            .push(("info".to_string(), at.to_string(), msg.to_string()));
    }
    fn warn(&self, at: &str, msg: &str) {
        self.lines
            .lock()
            .expect("锁")
            .push(("warn".to_string(), at.to_string(), msg.to_string()));
    }
    fn error(&self, at: &str, msg: &str) {
        self.lines
            .lock()
            .expect("锁")
            .push(("error".to_string(), at.to_string(), msg.to_string()));
    }
}

// ---------- 隐秘字段（secrets）的内存存储替身 ----------
/// 目的：SecretStore 的内存替身——可观察写入次数与内容。
pub(crate) struct InMemorySecretStore {
    pub(crate) values: Mutex<BTreeMap<String, String>>,
    pub(crate) saves: Mutex<usize>,
}

impl InMemorySecretStore {
    pub(crate) fn new() -> InMemorySecretStore {
        InMemorySecretStore {
            values: Mutex::new(BTreeMap::new()),
            saves: Mutex::new(0),
        }
    }
    pub(crate) fn with(values: BTreeMap<String, String>) -> InMemorySecretStore {
        InMemorySecretStore {
            values: Mutex::new(values),
            saves: Mutex::new(0),
        }
    }
    pub(crate) fn saves(&self) -> usize {
        *self.saves.lock().expect("锁")
    }
}

impl crate::capabilities::secrets::ports::SecretStore for InMemorySecretStore {
    fn load(&self) -> Result<BTreeMap<String, String>, String> {
        Ok(self.values.lock().expect("锁").clone())
    }
    fn save(&self, values: &BTreeMap<String, String>) -> Result<(), String> {
        *self.values.lock().expect("锁") = values.clone();
        *self.saves.lock().expect("锁") += 1;
        Ok(())
    }
}

/// 目的：已构建的产品可执行文件（守门进程就是它自己）；没有就 None（跳过真实进程用例）。
pub(crate) fn built_exe() -> Option<std::path::PathBuf> {
    let me = std::env::current_exe().ok()?;
    let profile_dir = me.parent()?.parent()?;
    let name = if cfg!(windows) {
        "solomni.exe"
    } else {
        "solomni"
    };
    let p = profile_dir.join(name);
    p.is_file().then_some(p)
}

/// 目的：本机可用的 python（没有就 None，如实跳过需要解释器的用例）。
pub(crate) fn python() -> Option<&'static str> {
    for name in ["python", "python3"] {
        if let Ok(o) = std::process::Command::new(name)
            .arg("-c")
            .arg("print(1)")
            .output()
        {
            if o.status.success() {
                return Some(name);
            }
        }
    }
    None
}

/// 目的：一个带给定常驻服务操作的最小成员工具环境（模型侧工具面用例用）。
pub(crate) fn member_with_service_tools(
    module: &str,
    residents: Arc<dyn crate::capabilities::residents::api::ResidentOps + Send + Sync>,
    op: crate::capabilities::session::api::ServiceOp,
) -> crate::capabilities::session::api::MemberTools {
    let sb = test_sandbox(module, &[]);
    let mut modules = BTreeMap::new();
    modules.insert(
        module.to_string(),
        crate::capabilities::session::domain::tools::ModuleTools {
            root: abs(&["mods", module]),
            commands: BTreeMap::new(),
            books: BTreeMap::new(),
            parallel: std::collections::BTreeSet::new(),
            services: [(op.tool.clone(), op)].into_iter().collect(),
        },
    );
    crate::capabilities::session::api::MemberTools {
        mode: crate::capabilities::llm::api::ToolMode::Envelope,
        modules,
        observations: Default::default(),
        llm: test_llm_demo(),
        log: Arc::new(crate::kernel::ports::NoopLog),
        tools: test_tools_svc_with(
            Arc::new(SilentRunner),
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        ),
        tier: crate::kernel::api::Tier::Host,
        guest_lease: String::new(),
        vm: test_vm(),
        sandbox: sb.clone(),
        tools_this_turn: test_prompts()
            .text(crate::capabilities::prompt::api::Segment::ToolsThisTurn)
            .to_string(),
        builtin_tools: test_systools().tools,
        unavailable: BTreeMap::new(),
        fence: crate::kernel::api::FenceSpec::for_session(&sb, &[], false),
        module_env: Default::default(),
        runtime_path: None,
        runtime_roots: Vec::new(),
        shell_live: None,
        reply_seq: 0,
        line: Default::default(),
        allowed: crate::capabilities::tools::api::names(),
        role: "solo".to_string(),
        with_modules: true,
        notes: Default::default(),
        handlers: Vec::new(),
        residents,
        // 工具面用例不接工作区：写工具落地后不自动提交。
        workspace: None,
    }
}

/// 目的：给进程机制用的测试文案（代码实现，与生产同一份）。
pub(crate) fn test_process_texts() -> Arc<dyn crate::kernel::ports::ProcessTexts> {
    Arc::new(crate::kernel::detail::process::CodeProcessTexts)
}

// ---------- 常驻服务（MCP）的脚本化长驻会话替身 ----------
/// 目的：脚本化的长驻会话替身——按脚本逐行应答，并记录收到的每一行与关闭调用。
pub(crate) struct ScriptedHost {
    pub sent: Arc<Mutex<Vec<String>>>,
    pub(crate) replies: Mutex<Vec<String>>,
}
