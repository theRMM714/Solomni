//! 目的：会话编辑：冻结、规则与名单写回。
//! 管：内容先定后冻名、与创建同规则、子会话写回 agent 名单、跨树模块重叠、profile 收窄。
//! 不管：创建与删除。
//! 联动：`tests::builders`；登记处见 docs/registry/。

use super::super::builders::*;
use super::super::prelude::*;

#[test]
pub(crate) fn edit_session_writes_meta_appends_config_record_and_rebuilds() {
    let hist = Arc::new(InMemoryHistory::new());
    let mut a = module_of("a");
    a.manifest
        .tools
        .insert("grep".to_string(), decl("python tools/grep.py"));
    let mut core = core_with_pkgs(
        vec![a, module_of("b")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
        Arc::new(InMemoryPackages::empty()),
    );
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    assert!(
        !core.session_config(&sid).unwrap().started,
        "单 agent 会话在用户开口之前还没内容：名字仍改得动"
    );
    // 改：模块 a → b，模型指定 m，档位换虚拟机档。网络是全局策略，不在会话里改。虚拟机档现在一律不可选
    // （guest 本体尚未接入），所以**先把它做成已存在的虚拟机档会话**——已在 vm 档的会话只校验它自己那几项。
    hist.force_tier(&sid, Tier::Vm);
    let guest = crate::tests::test_guest("edit-session-base");
    core.edit_session(
        &sid,
        SessionEdit {
            agents: vec![ConfigAgent {
                name: "a".to_string(),
                modules: vec!["b".to_string()],
                model: "m".to_string(),
                chara: String::new(),
            }],
            tier: "vm".to_string(),
            guest: Some(guest.clone()),
            pins: BTreeMap::new(),
            permission: Some(crate::capabilities::permission::api::PermissionLayer {
                shared: Some(crate::capabilities::permission::api::SharedEdit {
                    write: Some(crate::capabilities::permission::api::ScopeMode::Paths(
                        vec!["docs".to_string()],
                    )),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        },
    )
    .unwrap();
    let cfg = core.session_config(&sid).unwrap();
    assert_eq!(cfg.agents[0].modules, vec!["b".to_string()]);
    assert_eq!(cfg.tier, "vm");
    assert_eq!(
        cfg.permission.shared.write,
        crate::capabilities::permission::api::ScopeMode::Paths(vec!["docs".to_string()])
    );
    let (meta, events) = hist.load(&sid).unwrap();
    assert_eq!(
        meta.agents[0].modules,
        vec!["b".to_string()],
        "meta.yaml 是名单的唯一真相"
    );
    assert_eq!(meta.exec.tier, Tier::Vm);
    assert_eq!(
        meta.sandbox
            .ceiling
            .as_ref()
            .and_then(|l| l.shared.as_ref())
            .and_then(|s| s.write.clone()),
        Some(crate::capabilities::permission::api::ScopeMode::Paths(
            vec!["docs".to_string()]
        )),
        "会话级权限写回 meta"
    );
    assert_eq!(
        meta.exec.guest.as_deref(),
        Some(guest.as_str()),
        "虚拟根随提交写回"
    );
    let config = events
        .iter()
        .find(|e| e.get("type").and_then(|t| t.as_str()) == Some("config"))
        .unwrap_or_else(|| panic!("每次提交编辑追加一条旁路配置记录：{:?}", events));
    assert!(
        config.get("permission").is_some(),
        "配置记录要带权限层：{}",
        config
    );
    assert!(
        config
            .get("permission_effective")
            .and_then(|p| p.get("profile"))
            .is_some(),
        "配置记录要带生效档位：{}",
        config
    );
    // 内存里的会话按旧配置装过：丢掉后下次访问按新配置从转录重建（内容不丢）。
    assert!(!core.session_exists(&sid));
    // 下一次访问按新配置从转录重建（单 agent 会话还没轮到用户：它只提醒，不硬发请求）。
    with_live(|l| core.continue_flow(&sid, l)).unwrap();
    assert!(core.session_exists(&sid), "访问会话即按新配置重建");
    // 重建后：身份块照样现渲染（不在对话里），对话里的内容也还在。
    let identity = core.single_identity(&sid).expect("重建后身份块");
    assert!(!identity.is_empty(), "重建后身份块还在");
    let _ = core.single_history(&sid).unwrap();
}

#[test]
pub(crate) fn edit_session_freezes_names_only_after_content() {
    let hist = Arc::new(InMemoryHistory::new());
    seed_session(
        &hist,
        "raw",
        "single",
        vec![agent_meta("a", &["a"], None)],
        ExecSpec::default(),
    );
    let mut core = core_with_pkgs(
        vec![module_of("a"), module_of("b")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
        Arc::new(InMemoryPackages::empty()),
    );
    // 没内容：名字与模块都能换。
    core.edit_session("raw", edit_of(vec![("新名", &["b"], "")]))
        .unwrap();
    assert_eq!(core.session_config("raw").unwrap().agents[0].name, "新名");
    // 有内容之后：名字冻结，模块与模型照旧可改。
    hist.append(
        "raw",
        &[serde_json::json!({"type":"transcript","lines":[{"id":0,"line":"[用户] 你好"}]})],
    )
    .unwrap();
    let err = core
        .edit_session("raw", edit_of(vec![("再改", &["b"], "")]))
        .unwrap_err();
    assert!(err.contains("冻结"), "{}", err);
    core.edit_session("raw", edit_of(vec![("新名", &["a"], "m")]))
        .unwrap();
    assert_eq!(
        core.session_config("raw").unwrap().agents[0].modules,
        vec!["a".to_string()]
    );
}

#[test]
pub(crate) fn edit_session_enforces_the_same_rules_as_creation() {
    let hist = Arc::new(InMemoryHistory::new());
    seed_session(
        &hist,
        "w",
        "single",
        vec![agent_meta("a", &["a"], None)],
        ExecSpec::default(),
    );
    seed_session(
        &hist,
        "c",
        "collab",
        vec![agent_meta("a", &["a"], None), agent_meta("b", &["b"], None)],
        ExecSpec::default(),
    );
    let mut core = core_with_pkgs(
        vec![module_of("a"), module_of("b")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
        Arc::new(InMemoryPackages::empty()),
    );
    let e = core
        .edit_session("w", edit_of(vec![("a", &["没有这个模块"], "")]))
        .unwrap_err();
    assert!(e.contains("无此模块"), "{}", e);
    core.edit_session("c", edit_of(vec![("x", &["a"], ""), ("y", &["a"], "")]))
        .expect("同一模块可被多个 agent 共用");
    let shared = core.session_config("c").unwrap();
    assert!(
        shared
            .agents
            .iter()
            .all(|a| a.modules == vec!["a".to_string()])
    );
    let mut bad_tier = edit_of(vec![("x", &["a"], "")]);
    bad_tier.tier = "docker".to_string();
    assert!(
        core.edit_session("c", bad_tier)
            .unwrap_err()
            .contains("未知执行档位")
    );
    let mut two_agents = edit_of(vec![("x", &["a"], ""), ("y", &["b"], "")]);
    two_agents.tier = "host".to_string();
    assert!(
        core.edit_session("w", two_agents)
            .unwrap_err()
            .contains("只接受一个 agent")
    );

    // 虚拟机档选型不成立（同一能力多版本未定版）：编辑与「开始」同一把尺子，如实拒绝。
    let hist2 = Arc::new(InMemoryHistory::new());
    // 已经在虚拟机档上的会话（"留在 vm 档"这一路：只校验它自己那几项，不拿"本机能不能提供 vm 档"拦它）。
    // 虚拟根给一个真实在场的那一格：留在 vm 档时仍然要校验它自描述得对不对。
    let vm_guest = crate::tests::test_guest("edit-rules-vm");
    seed_session(
        &hist2,
        "c",
        "collab",
        vec![agent_meta("x", &["a"], None)],
        ExecSpec {
            tier: Tier::Vm,
            guest: Some(vm_guest.clone()),
            ..ExecSpec::default()
        },
    );
    let mut core2 = core_with_pkgs(
        vec![module_with_runtimes("a", &["python"])],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist2),
        Arc::new(InMemorySysIo::new()),
        Arc::new(InMemoryPackages::with(&[
            "id: python\nversion: 3.12.4\nprefix: opt/rt/py312",
            "id: python\nversion: 3.11.9\nprefix: opt/rt/py311",
        ])),
    );
    let mut vm_edit = edit_of(vec![("x", &["a"], "")]);
    vm_edit.tier = "vm".to_string();
    vm_edit.guest = Some(vm_guest.clone());
    let e = core2.edit_session("c", vm_edit.clone()).unwrap_err();
    assert!(e.contains("多个版本"), "{}", e);
    // 定版之后可以提交（缺包不拦：那只是该模块工具不可用）。
    vm_edit.pins = BTreeMap::from([("python".to_string(), "3.12.4".to_string())]);
    core2.edit_session("c", vm_edit).unwrap();
    assert_eq!(
        core2
            .session_config("c")
            .unwrap()
            .pins
            .get("python")
            .map(String::as_str),
        Some("3.12.4")
    );
}

#[test]
pub(crate) fn deleting_a_session_releases_its_fence_lease() {
    use crate::tests::doubles::{InMemorySysIo, RecordingFence};
    let hist = Arc::new(InMemoryHistory::new());
    let fence = Arc::new(RecordingFence::new());
    seed_session(
        &hist,
        "w",
        "single",
        vec![agent_meta("甲", &["a"], None)],
        ExecSpec::default(),
    );
    let gateway: Arc<dyn ChatGateway + Send + Sync> =
        Arc::new(gw(BTreeMap::new(), vec!["[]".into()]));
    let llm = test_llm(
        Arc::clone(&gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    let mut core = Conductor::new(
        crate::tests::root_dir(),
        registry_service(InMemorySettings::new(), Arc::clone(&llm)),
        test_history_of(Arc::clone(&hist)),
        test_workspace(Arc::new(InMemoryWorkspace::new())),
        test_vm(),
        test_packages(Arc::new(InMemoryPackages::empty())),
        test_provision(),
        test_modules(Arc::new(VecSource(vec![module_of("a")]))),
        Arc::new(crate::capabilities::builds::api::NoBuilds),
        llm,
        test_tools_svc_with(
            Arc::new(SilentRunner),
            Arc::new(InMemorySysIo::new()),
            Arc::clone(&fence) as Arc<dyn crate::kernel::ports::FenceHost + Send + Sync>,
        ),
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        crate::tests::doubles::test_host_probe(),
        Arc::new(crate::capabilities::residents::api::NoResidents),
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    );
    assert!(core.history_delete("w").unwrap(), "会话目录该被删掉");
    assert!(
        fence
            .lease_releases
            .lock()
            .expect("锁")
            .iter()
            .any(|l| l == "w"),
        "删除会话要按会话租约撤销围栏授权（调用结束不释放，生命周期归会话）"
    );
}

#[test]
pub(crate) fn session_meta_exec_section_roundtrips_and_defaults_to_host() {
    let meta = SessionMeta {
        context_freshness: false,
        name: "w".to_string(),
        mode: "single".to_string(),
        delegate: false,
        modules: vec!["a".to_string()],
        task: None,
        ts: 1,
        agents: Vec::new(),
        exec: ExecSpec {
            tier: Tier::Vm,
            guest: Some(crate::tests::test_guest("rt-guest")),
            pins: BTreeMap::from([("python".to_string(), "3.12.4".to_string())]),
        },
        sandbox: Default::default(),
        parent: None,
        node: None,
        delegation: None,
        thinking: None,
        run: RunState::Active,
        workspace: None,
    };
    let text = yaml_serde::to_string(&meta).expect("序列化");
    let back: SessionMeta = yaml_serde::from_str(&text).expect("反序列化");
    assert_eq!(back.exec.tier, Tier::Vm);
    assert_eq!(back.exec.guest.as_deref(), Some("rt-guest"));
    assert_eq!(
        back.exec.pins.get("python").map(String::as_str),
        Some("3.12.4")
    );
    assert_eq!(back.run, RunState::Active, "缺省运行态 = 正常运行");
    // 运行态是**持久事实**：停止 / 关闭写进 meta 后重启照样成立。
    let stopped = SessionMeta {
        context_freshness: false,
        thinking: None,
        run: RunState::Stopped,
        workspace: None,
        ..meta.clone()
    };
    let back: SessionMeta =
        yaml_serde::from_str(&yaml_serde::to_string(&stopped).expect("序列化")).expect("反序列化");
    assert_eq!(back.run, RunState::Stopped);
    // 缺 exec 段按当前默认读回：本机档、无虚拟根、无定版、正常运行。
    let defaulted: SessionMeta =
        yaml_serde::from_str("name: w\nmode: single\nmodules: [a]\nts: 1\n")
            .expect("缺 exec 段的 meta 必须可读");
    assert_eq!(defaulted.exec.tier, Tier::Host);
    assert!(defaulted.exec.guest.is_none());
    assert!(defaulted.exec.pins.is_empty());
    assert_eq!(defaulted.run, RunState::Active);
}
