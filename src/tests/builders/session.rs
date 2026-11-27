//! 目的：会话与名单的测试脚手架——造成员、造名单、造会话与一次编辑提交。
//! 管：工具声明样本、坏信封、成员装配、名单与 meta 落盘、编辑提交。
//! 不管：事件视图（`events.rs`）、通道（`chat.rs`）。
//! 联动：`src/units/business/runtime/api.rs` 与 `src/units/modules/api.rs`。

use super::*;
use crate::units::business::session_model::api::SessionSpec;

pub(crate) fn chain_node(id: &str, deps: &[&str]) -> crate::units::pure::taskchain::api::TaskNode {
    crate::units::pure::taskchain::api::TaskNode {
        id: id.to_string(),
        title: format!("节点{}", id),
        objective: format!("把 {} 做完", id),
        assignee: "甲".to_string(),
        deps: deps.iter().map(|d| d.to_string()).collect(),
        status: crate::units::pure::taskchain::api::NodeStatus::Pending,
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
    "{\"type\":\"tool\",\"name\":\"scan\",\"args\":{\"keyword\":\"x\"}}";

/// 目的：带工具环境的测试席位：生产里工具面归会话（`AgentSession`），测试里跟成员放在一起，便于逐席改 face、通道与执行服务。
pub(crate) struct WithTools {
    pub member: Member,
    pub tools: Option<MemberTools>,
}

/// 目的：测试替身的放行名单：全部内置工具 + 模块表里的限定 id（`<模块>.<工具>`）。
/// 约束：模块工具按限定 id 发放——与真装配机的口径一致；只算名单，不造模块表。
pub(crate) fn face_allowed(
    modules: &BTreeMap<String, crate::units::business::agentloop::api::ModuleTools>,
) -> Vec<String> {
    let mut out = crate::units::business::tools::api::names();
    for (id, tools) in modules {
        for name in tools.commands.keys().chain(tools.services.keys()) {
            let qualified = format!("{}.{}", id, name);
            if !out.contains(&qualified) {
                out.push(qualified);
            }
        }
    }
    out
}

/// 目的：带工具环境的成员：模块 m0 声明 scan → python tools/scan.py（cwd = 该模块目录）。
pub(crate) fn member_with_tools(
    id: &str,
    script: Vec<String>,
    runner: Arc<impl ProcessRunner + 'static>,
) -> WithTools {
    let mut commands = BTreeMap::new();
    commands.insert("scan".to_string(), "python tools/scan.py".to_string());
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
    let allowed = face_allowed(&modules);
    let m = Member::new(
        id,
        crate::tests::doubles::test_params(id),
        crate::units::pure::registry_model::api::ToolMode::Envelope,
        scripted(script),
    );
    // 该路径走模块声明的外部命令（scan）：空沙箱 + 内存 IO，内置工具不参与。
    WithTools {
        member: m,
        tools: Some(MemberTools {
            face: crate::units::business::agentloop::api::ToolFace {
                mode: crate::units::pure::registry_model::api::ToolMode::Envelope,
                loop_id: "standard".to_string(),
                role: "solo".to_string(),
                modules,
                tier: crate::units::pure::exec_model::api::Tier::Host,
                guest_lease: String::new(),
                sandbox: test_sandbox("m0", &[]),
                tools_this_turn: test_prompts()
                    .text(crate::units::business::prompt::api::Segment::ToolsThisTurn)
                    .to_string(),
                builtin_tools: test_systools().tools,
                unavailable: BTreeMap::new(),
                fence: crate::kernel::api::fence::FencePlan::for_session(
                    &test_sandbox("m0", &[]),
                    &[],
                    false,
                ),
                module_env: Default::default(),
                runtime_path: None,
                runtime_roots: Vec::new(),
                // 测试替身按"执行席"发放全部内置工具与模块的限定 id（机制席位的越权校验另有专门用例）。
                allowed,
                source_ids: vec!["agent".to_string(), "module".to_string()],
                system_tools: Vec::new(),
                abilities: crate::units::pure::registry_model::api::DEFAULT_ABILITIES
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                profile: "single".to_string(),
                with_modules: true,
                notes: crate::tests::doubles::test_notes(&test_sandbox("m0", &[]), &[]),
            },
            llm: test_llm_demo(),
            log: Arc::new(crate::kernel::ports::NoopLog),
            tools: test_tools_svc_with(
                runner,
                Arc::new(InMemorySysIo::new()),
                Arc::new(NoFenceHost),
            ),
            vm: test_vm(),
            residents: Arc::new(crate::units::business::residents::api::NoResidents),
            // 这些用例只跑工具面：写工具落地后不自动提交（没有工作区）。
            workspace: None,
            observations: crate::units::business::tools::api::Observations::default(),
            shell_live: None,
            reply_seq: 0,
            line: Default::default(),
            handlers: Vec::new(),
            // 替身不换席：模块表与提示词面只备空值（换席另有专门用例走真装配机）。
            modules: Vec::new(),
            prompt: Arc::new(test_prompts()),
            policy: crate::units::business::agentloop::api::default_policy(),
            decls: crate::tests::doubles::test_decls(),
        }),
    }
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
        abilities: None,
    }
}

/// 目的：按会话类型表造一份落盘声明（夹具与产品同一条路：会议类型自带流程、代理类型坐核心席）。
/// 参数：`kind` = 会话类型 id；表里没有就退回一个没有流程的单 agent 声明。
pub(crate) fn seeded_spec(kind: &str) -> SessionSpec {
    crate::tests::doubles::test_decls()
        .spec_of(kind)
        .unwrap_or_else(|| SessionSpec::plain(kind, kind, "solo"))
}

/// 目的：直接把一份 meta 放进内存历史（模拟"重启后从盘上读回该会话"）。
pub(crate) fn seed_session(
    hist: &Arc<InMemoryHistory>,
    name: &str,
    kind: &str,
    agents: Vec<AgentMeta>,
    exec: ExecSpec,
) {
    hist.create(&SessionMeta {
        usage_totals: Default::default(),
        context_freshness: false,
        freshness_keep_percent: 80,
        name: name.to_string(),
        spec: seeded_spec(kind),
        delegate: false,
        modules: agents.iter().flat_map(|a| a.modules.clone()).collect(),
        task: None,
        ts: 1,
        agents,
        exec,
        sandbox: Default::default(),
        shared: Default::default(),
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
        shared: None,
    }
}
