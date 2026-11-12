//! 目的：会话与名单的测试脚手架——造成员、造名单、造会话与一次编辑提交。
//! 管：工具声明样本、坏信封、成员装配、名单与 meta 落盘、编辑提交。
//! 不管：事件视图（`events.rs`）、通道（`chat.rs`）。
//! 联动：`src/capabilities/conductor/api.rs` 与 `src/capabilities/modules/api.rs`。

use super::*;

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

pub(crate) const TOOL_CALL: &str =
    "{\"type\":\"tool\",\"name\":\"grep\",\"args\":{\"keyword\":\"x\"}}";

/// 目的：带工具环境的成员：模块 m0 声明 grep → python tools/grep.py（cwd = 该模块目录）。
pub(crate) fn member_with_tools(
    id: &str,
    script: Vec<String>,
    runner: Arc<impl ProcessRunner + 'static>,
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
            services: BTreeMap::new(),
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
        role: "solo".to_string(),
        modules,
        observations: crate::capabilities::tools::api::Observations::default(),
        llm: test_llm_demo(),
        log: Arc::new(crate::kernel::ports::NoopLog),
        tools: test_tools_svc_with(
            runner,
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        ),
        tier: crate::kernel::api::Tier::Host,
        guest_lease: String::new(),
        vm: test_vm(),
        sandbox: test_sandbox("m0", &[]),
        tools_this_turn: test_prompts()
            .text(crate::capabilities::prompt::api::Segment::ToolsThisTurn)
            .to_string(),
        builtin_tools: test_systools().tools,
        unavailable: BTreeMap::new(),
        fence: crate::kernel::api::FenceSpec::for_session(&test_sandbox("m0", &[]), &[], false),
        module_env: Default::default(),
        runtime_path: None,
        runtime_roots: Vec::new(),
        shell_live: None,
        residents: Arc::new(crate::capabilities::residents::api::NoResidents),
        // 这些用例只跑工具面：写工具落地后不自动提交（没有工作区）。
        workspace: None,
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

/// 目的：真实的坏信封：结尾多了一个 ]（括号不配对）。路径用真实绝对路径（模型写对了路径、写坏了信封）。 坏的形状照真案例：args 先闭合、多一个 ]、外层才闭合（整段不是合法 JSON）。
pub(crate) fn broken_tool(path: &str) -> String {
    format!(
        "{{\"type\":\"tool\",\"name\":\"write\",\"args\":{{\"path\":\"{}\",\"content\":\"hi\"}}]}}",
        path
    )
}

/// 目的：造一条 agent 名单记录。
pub(crate) fn agent_meta(name: &str, modules: &[&str], model: Option<&str>) -> AgentMeta {
    AgentMeta {
        name: name.to_string(),
        transient: false,
        modules: modules.iter().map(|s| s.to_string()).collect(),
        model: model.map(|s| s.to_string()),
        chara: String::new(),
    }
}

/// 目的：直接把一份 meta 放进内存历史（模拟"重启后从盘上读回该会话"）。
pub(crate) fn seed_session(
    hist: &Arc<InMemoryHistory>,
    name: &str,
    mode: &str,
    agents: Vec<AgentMeta>,
    exec: ExecSpec,
) {
    hist.create(&SessionMeta {
        context_freshness: false,
        name: name.to_string(),
        mode: mode.to_string(),
        delegate: false,
        modules: agents.iter().flat_map(|a| a.modules.clone()).collect(),
        task: None,
        ts: 1,
        agents,
        exec,
        sandbox: Default::default(),
        parent: None,
        node: None,
        delegation: None,
        thinking: None,
        run: RunState::Active,
        workspace: None,
    })
    .unwrap();
}

/// 目的：一次编辑提交（名字 / 模块 / 模型；档位与定版默认本机档）。
pub(crate) fn edit_of(agents: Vec<(&str, &[&str], &str)>) -> SessionEdit {
    SessionEdit {
        agents: agents
            .into_iter()
            .map(|(n, ms, m)| ConfigAgent {
                name: n.to_string(),
                modules: ms.iter().map(|s| s.to_string()).collect(),
                model: m.to_string(),
                chara: String::new(),
            })
            .collect(),
        tier: "host".to_string(),
        guest: None,
        pins: BTreeMap::new(),
        permission: None,
    }
}
