//! 目的：会话建立、校验、历史与删除。
//! 管：创建/校验、树形历史、删除级联与运行态拒绝、工作根回溯、身份取机制与角色。
//! 不管：回合生成与回档。
//! 联动：`tests::builders`；落盘见 docs/session/session-model.md。

use super::super::builders::*;
use super::super::prelude::*;
use crate::capabilities::conversation::session::api::SINGLE;

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
        .create_work(work("工作一", SINGLE, &["a"]))
        .unwrap()
        .sid;
    assert_eq!(sid, "工作一");
    with_turn_io(|io, cancel, emit| {
        core.turn.single_say(
            &sid,
            "你好",
            io,
            cancel,
            emit,
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
    })
    .unwrap();
    let list = core.lifecycle.history_list();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "工作一");
    assert_eq!(list[0].kind, SINGLE);
    let (meta, events) = core.lifecycle.history_open("工作一").unwrap();
    assert_eq!(meta.spec.kind, SINGLE);
    assert!(
        events
            .iter()
            .any(|e| e.get("type").and_then(|t| t.as_str()) == Some("transcript")),
        "历史里应有转录事件"
    );
    // 重名（磁盘已存在）被拒
    assert!(
        core.create_work(work("工作一", SINGLE, &["a"]))
            .unwrap_err()
            .contains("已存在")
    );
    // 删除会话：内存与磁盘一起删，之后名字可复用
    assert!(
        core.lifecycle
            .history_delete(&mut core.turn, "工作一")
            .unwrap()
    );
    assert!(core.lifecycle.history_list().is_empty());
    assert!(core.create_work(work("工作一", SINGLE, &["a"])).is_ok());
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
    // 不存在的会话：只清内存并如实回 false（非落盘会话没有围栏收尾段）。
    assert!(
        !core
            .lifecycle
            .history_delete(&mut core.turn, "does-not-exist")
            .unwrap(),
        "不存在的会话要回 false"
    );
    let parent = core
        .create_work(execute_work("p", &["a"], "需求"))
        .unwrap()
        .sid;
    let child = core
        .turn
        .spawn_agent_session(
            &parent,
            "a",
            "executor",
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
        .unwrap();
    // 子会话不允许单独删，理由要明确，且拒绝后它仍在。
    let err = core
        .lifecycle
        .history_delete(&mut core.turn, &child)
        .unwrap_err();
    assert!(err.contains("子会话"), "{}", err);
    assert!(hist.load(&child).is_ok(), "拒绝删除后子会话仍在");
    // 删父会话：子会话随之消失。
    assert!(
        core.lifecycle
            .history_delete(&mut core.turn, &parent)
            .unwrap()
    );
    assert!(hist.load(&parent).is_err());
    assert!(hist.load(&child).is_err(), "子会话随父一起消失");
}

/// 目的：删除收尾里的围栏撤权**不占调用方等待路径**：目录先删完，再在后台线程按租约撤权。
///   回调被闸门挡住时 `run()` 仍要立刻返回；回调里能看到会话已经删掉（先删目录的快分支前提）。
#[test]
pub(crate) fn session_delete_releases_fences_in_background_after_directories() {
    use crate::capabilities::lifecycle::api::SessionDelete;
    use crate::capabilities::vm::api::GuestStop;
    use crate::capabilities::vm::api::Vm;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    let store = Arc::new(InMemoryHistory::new());
    seed_session(
        &store,
        "w",
        SINGLE,
        vec![agent_meta("a", &[], None)],
        ExecSpec::default(),
    );
    let hist = test_history_of(Arc::clone(&store));
    let vm: Arc<dyn Vm + Send + Sync> = Arc::new(crate::tests::doubles::RecordingVm::default());
    let gone = Arc::new(AtomicBool::new(false));
    let started = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let (gate_tx, gate_rx) = mpsc::channel::<()>();
    let gate_rx = Arc::new(Mutex::new(gate_rx));
    let release: crate::capabilities::lifecycle::api::ReleaseByLease = {
        let gone = Arc::clone(&gone);
        let started = Arc::clone(&started);
        let done = Arc::clone(&done);
        let hist = Arc::clone(&hist);
        Arc::new(move |_lease: &str| {
            gone.store(hist.load("w").is_err(), Ordering::SeqCst);
            started.store(true, Ordering::SeqCst);
            let _ = gate_rx.lock().expect("锁").recv();
            done.store(true, Ordering::SeqCst);
            Ok(())
        })
    };
    let delete = SessionDelete::for_test_with_fences(
        GuestStop::new(vm, Vec::new()),
        hist,
        vec!["w".to_string()],
        "w".to_string(),
        vec!["lease-w".to_string()],
        release,
        Arc::new(crate::kernel::ports::NoopLog),
    );
    let before = Instant::now();
    assert!(delete.run().expect("删除收尾"), "会话目录该删掉");
    assert!(
        before.elapsed() < Duration::from_secs(2),
        "撤权在后台：调用方不该等它"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while !started.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(started.load(Ordering::SeqCst), "后台撤权要真的开始");
    assert!(
        gone.load(Ordering::SeqCst),
        "撤权必须发生在目录删除之后（路径不在 → 只销账的快分支）"
    );
    gate_tx.send(()).expect("放行后台撤权");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(done.load(Ordering::SeqCst), "后台撤权要跑完");
}

/// 目的：没有租约时不起后台撤权线程（空清单直接返回）。
#[test]
pub(crate) fn session_delete_with_no_fence_leases_skips_release() {
    use crate::capabilities::lifecycle::api::SessionDelete;
    use crate::capabilities::vm::api::GuestStop;
    use crate::capabilities::vm::api::Vm;
    use std::sync::atomic::{AtomicBool, Ordering};

    let store = Arc::new(InMemoryHistory::new());
    seed_session(
        &store,
        "w",
        SINGLE,
        vec![agent_meta("a", &[], None)],
        ExecSpec::default(),
    );
    let hist = test_history_of(Arc::clone(&store));
    let vm: Arc<dyn Vm + Send + Sync> = Arc::new(crate::tests::doubles::RecordingVm::default());
    let called = Arc::new(AtomicBool::new(false));
    let release: crate::capabilities::lifecycle::api::ReleaseByLease = {
        let called = Arc::clone(&called);
        Arc::new(move |_lease: &str| {
            called.store(true, Ordering::SeqCst);
            Ok(())
        })
    };
    let delete = SessionDelete::for_test_with_fences(
        GuestStop::new(vm, Vec::new()),
        hist,
        vec!["w".to_string()],
        "w".to_string(),
        Vec::new(),
        release,
        Arc::new(crate::kernel::ports::NoopLog),
    );
    assert!(delete.run().expect("删除收尾"), "会话目录该删掉");
    assert!(!called.load(Ordering::SeqCst), "没有租约就不该调用撤权");
}

/// 目的：后台撤权失败要如实记日志（失败条目留在台账，下次重试）。
#[test]
pub(crate) fn session_delete_logs_background_release_failures() {
    use crate::capabilities::lifecycle::api::SessionDelete;
    use crate::capabilities::vm::api::GuestStop;
    use crate::capabilities::vm::api::Vm;
    use std::time::{Duration, Instant};

    let store = Arc::new(InMemoryHistory::new());
    seed_session(
        &store,
        "w",
        SINGLE,
        vec![agent_meta("a", &[], None)],
        ExecSpec::default(),
    );
    let hist = test_history_of(Arc::clone(&store));
    let vm: Arc<dyn Vm + Send + Sync> = Arc::new(crate::tests::doubles::RecordingVm::default());
    let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let release: crate::capabilities::lifecycle::api::ReleaseByLease = {
        let called = Arc::clone(&called);
        Arc::new(move |_lease: &str| {
            called.store(true, std::sync::atomic::Ordering::SeqCst);
            Err("模拟撤权失败".to_string())
        })
    };
    let log = Arc::new(crate::tests::doubles::RecordingLog::new());
    let log_dyn: Arc<dyn crate::kernel::ports::Log + Send + Sync> = log.clone();
    let delete = SessionDelete::for_test_with_fences(
        GuestStop::new(vm, Vec::new()),
        hist,
        vec!["w".to_string()],
        "w".to_string(),
        vec!["lease-w".to_string()],
        release,
        log_dyn,
    );
    assert!(delete.run().expect("删除收尾"), "会话目录该删掉");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !log.has("模拟撤权失败") && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        called.load(std::sync::atomic::Ordering::SeqCst),
        "后台撤权要真的执行"
    );
    assert!(
        log.has("模拟撤权失败"),
        "撤权失败要进日志（位置 lifecycle::release_fences）"
    );
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
        .create_work(execute_work("p", &["a"], "需求"))
        .unwrap()
        .sid;
    let mid = core
        .turn
        .spawn_agent_session(
            &top,
            "a",
            "executor",
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
        .unwrap();
    let low = core
        .turn
        .spawn_agent_session(
            &mid,
            "a",
            "executor",
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
        .unwrap();
    let history = core.history_handle();
    let root_of = |sid: &str| {
        crate::capabilities::conversation::session::api::work_root(history.as_ref(), sid)
    };
    assert_eq!(root_of(&top).unwrap(), top);
    assert_eq!(root_of(&mid).unwrap(), top, "第二层锚在顶");
    assert_eq!(root_of(&low).unwrap(), top, "第三层也锚在顶");
}

/// 目的：身份块里的机制说明按**席位**取：有这份说明的拿得到，没有的不拿。
#[test]
pub(crate) fn identity_takes_the_mechanism_of_its_seat() {
    let prompt = test_prompts();
    let mode = crate::capabilities::llm::api::ToolMode::Envelope;
    let identity = |role: &str| {
        let mut p = test_params("a");
        p.seat = role.to_string();
        p.identity(&prompt, mode)
    };
    let solo = identity("solo");
    let discussant = identity("discussant");
    let executor = identity("executor");
    let proxy = identity("core_proxy");
    let planner_role = identity("planner");
    assert!(solo.contains("单 agent 工作"), "{}", solo);
    assert!(discussant.contains("动词"), "{}", discussant);
    assert!(executor.contains("submit_report"), "{}", executor);
    assert!(proxy.contains("派完活就让出回合"), "{}", proxy);
    assert!(!solo.contains("派完活就让出回合"), "各拿自己那一份");
    assert!(!discussant.contains("单 agent 工作"), "各拿自己那一份");
    assert!(
        !planner_role.contains("== 这个系统怎么运转 =="),
        "没配说明的角色不拿机制说明：{}",
        planner_role
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
        .create_work(execute_work("p", &["a"], "需求"))
        .unwrap()
        .sid;
    let child = core
        .turn
        .spawn_agent_session(
            &parent,
            "a",
            "executor",
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
        .unwrap();
    core.turn
        .take_single(&child)
        .expect("把子会话交给工作线程（标记生成中）");
    let err = core
        .lifecycle
        .history_delete(&mut core.turn, &parent)
        .unwrap_err();
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
    let host = core.create_work(work("h", SINGLE, &["a"])).unwrap().sid;
    assert_eq!(
        core.lifecycle.history_open(&host).unwrap().0.exec.tier,
        Tier::Host
    );
    let mut vm = work("v", SINGLE, &["a"]);
    vm.tier = Tier::Vm;
    let err = core.create_work(vm).unwrap_err();
    assert!(err.contains("虚拟机档现在不可用"), "{}", err);
    assert!(
        core.lifecycle.history_open("v").is_err(),
        "拒绝就该什么都不留下"
    );
}

#[test]
pub(crate) fn create_work_validates_user_choices() {
    let mut core = core_with(
        vec![module_of("a"), module_of("b")],
        gw(BTreeMap::new(), vec!["[]".into()]),
    );
    assert!(
        core.create_work(work("", SINGLE, &["a"]))
            .unwrap_err()
            .contains("工作名不能为空")
    );
    assert!(
        core.create_work(work("x", SINGLE, &["ghost"]))
            .unwrap_err()
            .contains("无此模块")
    );
    // 模块可以为空：零模块 agent 照样建得出来（只用内建文件工具）。
    let opened = core
        .create_work(work("零模块", SINGLE, &[]))
        .expect("零模块 agent 合法");
    assert_eq!(opened.agents.len(), 1);
    assert!(
        core.lifecycle
            .history_open("零模块")
            .expect("读 meta")
            .0
            .agents[0]
            .modules
            .is_empty()
    );
    let _ = core.lifecycle.history_delete(&mut core.turn, "零模块");
    // 单模式的**组合语义**：点名多个 = 并成一个临时组合（模块去重、保序；模型取核心默认）。
    let two_agents = WorkSpec {
        flows: None,
        freshness: None,
        name: "x".to_string(),
        kind: SINGLE.to_string(),
        agents: vec![
            AgentMeta {
                name: "甲".to_string(),
                transient: true,
                modules: vec!["a".to_string()],
                model: None,
                chara: String::new(),
            },
            AgentMeta {
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
        tier: crate::kernel::api::types::Tier::Host,
        guest: None,
        workspace: None,
    };
    let opened = core
        .create_work(two_agents)
        .expect("单模式点名多个应并成一个，不是报错");
    assert_eq!(opened.agents, vec!["x".to_string()], "并出来的组合用工作名");
    let meta = core.lifecycle.history_open("x").expect("读 meta").0;
    assert_eq!(
        meta.agents[0].modules,
        vec!["a".to_string(), "b".to_string()]
    );
    assert!(meta.agents[0].transient, "并出来的是临时 agent");
    let _ = core.lifecycle.history_delete(&mut core.turn, "x");
    assert!(
        core.create_work(execute_work("x", &["a"], "  "))
            .unwrap_err()
            .contains("必须填写本次需求")
    );
    let mut bad_model = work("x", SINGLE, &["a"]);
    bad_model.agents[0].model = Some("ghost".to_string());
    assert!(
        core.create_work(bad_model)
            .unwrap_err()
            .contains("无此模型")
    );
    // 建一次成功后重名被拒
    assert!(core.create_work(work("同名", SINGLE, &["a"])).is_ok());
    assert!(
        core.create_work(work("同名", SINGLE, &["a"]))
            .unwrap_err()
            .contains("已存在")
    );
    // 非法字符（将来要当目录名）
    assert!(
        core.create_work(work("a/b", SINGLE, &["a"]))
            .unwrap_err()
            .contains("不能包含")
    );
}
