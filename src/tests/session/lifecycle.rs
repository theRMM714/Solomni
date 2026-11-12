//! 目的：会话建立、校验、历史与删除。
//! 管：创建/校验、树形历史、删除级联与运行态拒绝、工作根回溯、身份取机制与角色。
//! 不管：回合生成与回档。
//! 联动：`tests::builders`；落盘见 docs/session/session-model.md。

use super::super::builders::*;
use super::super::prelude::*;

#[test]
pub(crate) fn create_work_persists_and_history_replays() {
    let hist = Arc::new(InMemoryHistory::new());
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
    );
    let sid = core
        .create_work(work("工作一", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    assert_eq!(sid, "工作一");
    with_live(|l| core.single_say(&sid, "你好", l)).unwrap();
    let list = core.history_list();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "工作一");
    assert_eq!(list[0].mode, "single");
    let (meta, events) = core.history_open("工作一").unwrap();
    assert_eq!(meta.mode, "single");
    assert!(
        events
            .iter()
            .any(|e| e.get("type").and_then(|t| t.as_str()) == Some("transcript")),
        "历史里应有转录事件"
    );
    // 重名（磁盘已存在）被拒
    assert!(
        core.create_work(work("工作一", WorkMode::Single, &["a"]))
            .unwrap_err()
            .contains("已存在")
    );
    // 删除会话：内存与磁盘一起删，之后名字可复用
    assert!(core.history_delete("工作一").unwrap());
    assert!(core.history_list().is_empty());
    assert!(
        core.create_work(work("工作一", WorkMode::Single, &["a"]))
            .is_ok()
    );
}

/// 目的：删父带子：整个子树一起消失；子会话不允许单独删（它由核心按节点派生）。
#[test]
pub(crate) fn history_delete_cascades_to_children_and_refuses_child_delete() {
    let hist = Arc::new(InMemoryHistory::new());
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
    );
    let parent = core
        .create_work(collab_work("p", &["a"], false, "需求"))
        .unwrap()
        .sid;
    let child = core.spawn_agent_session(&parent, "a").unwrap();
    // 子会话不允许单独删，理由要明确，且拒绝后它仍在。
    let err = core.history_delete(&child).unwrap_err();
    assert!(err.contains("子会话"), "{}", err);
    assert!(hist.load(&child).is_ok(), "拒绝删除后子会话仍在");
    // 删父会话：子会话随之消失。
    assert!(core.history_delete(&parent).unwrap());
    assert!(hist.load(&parent).is_err());
    assert!(hist.load(&child).is_err(), "子会话随父一起消失");
}

/// 目的：删除 VM 会话时**先停 guest，再删目录**——Windows 上 guest 还占着会话目录里的文件，反过来会 sharing violation。
#[test]
pub(crate) fn session_delete_stops_the_guest_before_removing_directories() {
    use crate::capabilities::conductor::service::SessionDelete;
    use crate::capabilities::conductor::service::guest_job::GuestStop;
    use crate::capabilities::vm::api::Vm;
    use crate::capabilities::vm::domain::guest::{GuestRun, GuestSpec};
    use crate::kernel::api::FenceSpec;

    struct StopVm {
        history: Arc<InMemoryHistory>,
        saw_directory: Arc<Mutex<bool>>,
    }

    impl Vm for StopVm {
        fn guest_start(&self, _spec: &GuestSpec) -> Result<(), String> {
            Ok(())
        }

        fn guest_run(&self, _lease: &str, _run: &GuestRun) -> ToolOutcome {
            unimplemented!("这条用例只验删除顺序")
        }

        fn guest_run_fenced(
            &self,
            _lease: &str,
            _run: &GuestRun,
            _fence: &FenceSpec,
        ) -> ToolOutcome {
            unimplemented!("这条用例只验删除顺序")
        }

        fn stage_exec(
            &self,
            _lease: &str,
            _module: &str,
            _host: &std::path::Path,
            _entry: &str,
        ) -> Result<String, String> {
            unimplemented!("这条用例只验删除顺序")
        }

        fn guest_packages(&self, _lease: &str) -> Option<Vec<String>> {
            None
        }

        fn guest_set_packages(&self, _lease: &str, _packages: Vec<String>) {}

        fn guest_wait_agent(&self, _lease: &str, _timeout: Duration) -> bool {
            true
        }

        fn guest_stop(&self, _lease: &str) {
            *self.saw_directory.lock().expect("锁") = self.history.load("w").is_ok();
        }

        fn guest_leases(&self) -> Vec<String> {
            Vec::new()
        }
    }

    let store = Arc::new(InMemoryHistory::new());
    seed_session(
        &store,
        "w",
        "single",
        vec![agent_meta("a", &[], None)],
        ExecSpec::default(),
    );
    let hist = test_history_of(Arc::clone(&store));
    let saw = Arc::new(Mutex::new(false));
    let vm: Arc<dyn Vm + Send + Sync> = Arc::new(StopVm {
        history: Arc::clone(&store),
        saw_directory: Arc::clone(&saw),
    });
    let delete = SessionDelete::for_test(
        GuestStop::new(vm, vec!["w".to_string()]),
        hist,
        vec!["w".to_string()],
        "w".to_string(),
    );
    assert!(delete.run().expect("删除收尾"), "会话目录该删掉");
    assert!(
        *saw.lock().expect("锁"),
        "停 guest 时必须还没删目录（否则 Windows 会 sharing violation）"
    );
    assert!(store.load("w").is_err(), "停完 guest 后再删目录");
}

/// 目的：工作根沿父链走到顶：代理 → 多 agent 子会话 → 它的节点子会话，第三层也锚在**顶层**那一个 work/ 上。
#[test]
pub(crate) fn work_root_walks_to_the_top_across_nested_sub_sessions() {
    let hist = Arc::new(InMemoryHistory::new());
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
    );
    let top = core
        .create_work(collab_work("p", &["a"], false, "需求"))
        .unwrap()
        .sid;
    let mid = core.spawn_agent_session(&top, "a").unwrap();
    let low = core.spawn_agent_session(&mid, "a").unwrap();
    assert_eq!(core.work_root(&top).unwrap(), top);
    assert_eq!(core.work_root(&mid).unwrap(), top, "第二层锚在顶");
    assert_eq!(core.work_root(&low).unwrap(), top, "第三层也锚在顶");
}

/// 目的：身份块里的机制说明按（**会话使用类型** × **角色**）取：对得上的拿，对不上的不拿。
#[test]
pub(crate) fn identity_takes_the_mechanism_of_its_session_and_role() {
    let prompt = test_prompts();
    let mode = crate::capabilities::llm::api::ToolMode::Envelope;
    let identity = |kind: &str, role: &str| {
        let mut p = test_params("a");
        p.session_kind = kind.to_string();
        p.role = role.to_string();
        p.identity(&prompt, mode)
    };
    let solo = identity("single", "solo");
    let discussant = identity("collab", "discussant");
    let executor = identity("collab", "executor");
    let proxy = identity("proxy", "core_proxy");
    let mismatch = identity("single", "core_proxy");
    assert!(solo.contains("单 agent 工作"), "{}", solo);
    assert!(
        discussant.contains("一个 agent = 一个会话"),
        "{}",
        discussant
    );
    assert!(executor.contains("一个 agent = 一个会话"), "{}", executor);
    assert!(proxy.contains("派完活就让出回合"), "{}", proxy);
    assert!(!solo.contains("一个 agent = 一个会话"), "各拿自己那一份");
    assert!(!proxy.contains("一个 agent = 一个会话"), "各拿自己那一份");
    assert!(
        !mismatch.contains("单 agent 工作"),
        "类型对不上就不发：{}",
        mismatch
    );
}

/// 目的：子树里任一节点在生成中就**整体拒绝**：不能删到一半留下半个状态。
#[test]
pub(crate) fn history_delete_refuses_when_any_node_in_the_subtree_is_running() {
    let hist = Arc::new(InMemoryHistory::new());
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
    );
    let parent = core
        .create_work(collab_work("p", &["a"], false, "需求"))
        .unwrap()
        .sid;
    let child = core.spawn_agent_session(&parent, "a").unwrap();
    core.take_single(&child)
        .expect("把子会话交给工作线程（标记生成中）");
    let err = core.history_delete(&parent).unwrap_err();
    assert!(err.contains("正在生成"), "{}", err);
    assert!(
        hist.load(&parent).is_ok() && hist.load(&child).is_ok(),
        "整体拒绝：父与子都还在"
    );
}

/// 目的：创建工作的档位来自用户选择；承载不了的虚拟机档如实拒绝（与「开始」的校验同源）。
#[test]
pub(crate) fn create_work_uses_chosen_tier_and_rejects_unavailable_vm() {
    let hist = Arc::new(InMemoryHistory::new());
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
    );
    let host = core
        .create_work(work("h", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    assert_eq!(core.history_open(&host).unwrap().0.exec.tier, Tier::Host);
    let mut vm = work("v", WorkMode::Single, &["a"]);
    vm.tier = Tier::Vm;
    let err = core.create_work(vm).unwrap_err();
    assert!(err.contains("虚拟机档现在不可用"), "{}", err);
    assert!(core.history_open("v").is_err(), "拒绝就该什么都不留下");
}

#[test]
pub(crate) fn create_work_validates_user_choices() {
    let mut core = core_with(
        vec![module_of("a"), module_of("b")],
        gw(BTreeMap::new(), vec!["[]".into()]),
    );
    assert!(
        core.create_work(work("", WorkMode::Single, &["a"]))
            .unwrap_err()
            .contains("工作名不能为空")
    );
    assert!(
        core.create_work(work("x", WorkMode::Single, &["ghost"]))
            .unwrap_err()
            .contains("无此模块")
    );
    // 模块可以为空：零模块 agent 照样建得出来（只用内建文件工具）。
    let opened = core
        .create_work(work("零模块", WorkMode::Single, &[]))
        .expect("零模块 agent 合法");
    assert_eq!(opened.agents.len(), 1);
    assert!(
        core.history_open("零模块").expect("读 meta").0.agents[0]
            .modules
            .is_empty()
    );
    let _ = core.history_delete("零模块");
    // 单模式的**组合语义**：点名多个 = 并成一个临时组合（模块去重、保序；模型取核心默认）。
    let two_agents = WorkSpec {
        name: "x".to_string(),
        mode: WorkMode::Single,
        agents: vec![
            AgentInstance {
                name: "甲".to_string(),
                transient: true,
                modules: vec!["a".to_string()],
                model: None,
                chara: String::new(),
            },
            AgentInstance {
                name: "乙".to_string(),
                transient: true,
                modules: vec!["b".to_string()],
                model: None,
                chara: String::new(),
            },
        ],
        task: None,
        delegate: false,
        permission: None,
        tier: crate::kernel::api::Tier::Host,
        guest: None,
        workspace: None,
    };
    let opened = core
        .create_work(two_agents)
        .expect("单模式点名多个应并成一个，不是报错");
    assert_eq!(opened.agents, vec!["x".to_string()], "并出来的组合用工作名");
    let meta = core.history_open("x").expect("读 meta").0;
    assert_eq!(
        meta.agents[0].modules,
        vec!["a".to_string(), "b".to_string()]
    );
    assert!(meta.agents[0].transient, "并出来的是临时 agent");
    let _ = core.history_delete("x");
    assert!(
        core.create_work(collab_work("x", &["a"], false, "  "))
            .unwrap_err()
            .contains("必须填写本次需求")
    );
    let mut bad_model = work("x", WorkMode::Single, &["a"]);
    bad_model.agents[0].model = Some("ghost".to_string());
    assert!(
        core.create_work(bad_model)
            .unwrap_err()
            .contains("无此模型")
    );
    // 建一次成功后重名被拒
    assert!(
        core.create_work(work("同名", WorkMode::Single, &["a"]))
            .is_ok()
    );
    assert!(
        core.create_work(work("同名", WorkMode::Single, &["a"]))
            .unwrap_err()
            .contains("已存在")
    );
    // 非法字符（将来要当目录名）
    assert!(
        core.create_work(work("a/b", WorkMode::Single, &["a"]))
            .unwrap_err()
            .contains("不能包含")
    );
}
