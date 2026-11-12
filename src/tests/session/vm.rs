//! 目的：虚拟机档会话：共享 guest 的启动与跟随。
//! 管：启用 VM 历史会话真的拉起共享 guest、子会话跟随主 guest 的档位、代理子树共用 guest。
//! 不管：guest 自身能力见 tests/vm/。
//! 联动：`tests::builders`；虚拟根夹具有 `tests::test_guest`。

use super::super::builders::*;
use super::super::prelude::*;

/// 目的：核心重启后启用一条 VM 历史会话——必须真的准备并拉起共享 guest，且不跑模型。
#[test]
pub(crate) fn activating_a_vm_history_session_boots_the_shared_guest() {
    use crate::capabilities::conductor::service::{PreparedActivation, ReadyActivation};
    let hist = Arc::new(InMemoryHistory::new());
    seed_session(
        &hist,
        "w",
        "single",
        vec![agent_meta("a", &["a"], None)],
        vm_spec(),
    );
    let mut core = core_with_pkgs(
        vec![module_of("a")],
        gw(BTreeMap::new(), Vec::new()),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
        Arc::new(InMemoryPackages::empty()),
    );
    let prepared = core.prepare_activation("w").expect("准备启用");
    let (sid, boot, lease) = match prepared {
        PreparedActivation::Ready(ready) => {
            let ReadyActivation { sid, boot, lease } = *ready;
            (sid, boot, lease)
        }
        PreparedActivation::Running(_) => panic!("历史会话不该被当成正在生成"),
    };
    assert_eq!(lease.as_deref(), Some("w"), "共享 guest 租约 = 顶层工作名");
    let boot = boot.expect("VM 历史会话要准备 guest 启动");
    boot.run().expect("guest 起来");
    let view = core
        .commit_activation(PreparedActivation::Ready(Box::new(ReadyActivation {
            sid,
            boot: None,
            lease,
        })))
        .expect("提交启用");
    assert!(core.session_exists("w"), "启用后会话要进核心表");
    assert_eq!(view.sid, "w");
    assert!(!view.running, "启用本身不生成");
}

/// 目的：协作子会话的工具租约指向主会话树的共享 guest；主会话改档位时子树跟着改。
#[test]
pub(crate) fn child_sessions_share_the_main_guest_and_follow_its_tier() {
    let hist = Arc::new(InMemoryHistory::new());
    seed_session(
        &hist,
        "w",
        "collab",
        vec![agent_meta("a", &["a"], None), agent_meta("b", &["b"], None)],
        vm_spec(),
    );
    let child = SessionMeta {
        context_freshness: false,
        name: "w--a".to_string(),
        mode: "single".to_string(),
        delegate: false,
        modules: vec!["a".to_string()],
        task: None,
        ts: 2,
        agents: vec![agent_meta("a", &["a"], None)],
        exec: vm_spec(),
        sandbox: Default::default(),
        parent: Some("w".to_string()),
        node: None,
        delegation: None,
        thinking: None,
        run: RunState::Active,
        workspace: None,
    };
    hist.create(&child).expect("子会话落盘");
    let mut core = core_with_pkgs(
        vec![module_of("a"), module_of("b")],
        gw(BTreeMap::new(), Vec::new()),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
        Arc::new(InMemoryPackages::empty()),
    );
    let child_meta = hist.meta("w--a").expect("读子会话");
    assert_eq!(
        core.guest_lease_for(&child_meta).expect("取租约"),
        "w",
        "子会话共用主会话树的 guest"
    );
    // 子会话配置把档位标成继承（界面不渲染选择）。
    assert!(
        core.session_config("w--a").expect("配置").tier_locked,
        "子会话档位随主会话"
    );
    // 子会话不能单独改档位。
    let mut bad = edit_of(vec![("a", &["a"], "")]);
    bad.tier = "host".to_string();
    bad.guest = None;
    let err = core
        .prepare_edit("w--a", bad)
        .err()
        .expect("子会话不能单独改档位");
    assert!(err.contains("子会话"), "{}", err);
    // 主会话留在 VM 档改配：子会话的 exec 要同步成主会话的。
    let mut edit = edit_of(vec![("a", &["a"], ""), ("b", &["b"], "")]);
    edit.tier = "vm".to_string();
    edit.guest = Some(crate::tests::test_guest("vm-spec"));
    let (prepared, _job) = core.prepare_edit("w", edit).expect("准备改配");
    let updated = prepared
        .new_children
        .iter()
        .find(|m| m.name == "w--a")
        .expect("子树里有子会话");
    assert_eq!(updated.exec.tier, Tier::Vm, "子会话档位跟主会话");
}

/// 目的：proxy 树里的子工作共用代理根那一台 guest，不再每个子工作各起一台。
#[test]
pub(crate) fn proxy_children_share_the_proxy_tree_guest() {
    let hist = Arc::new(InMemoryHistory::new());
    seed_session(&hist, "p", "proxy", Vec::new(), vm_spec());
    let child = SessionMeta {
        context_freshness: false,
        name: "p--a".to_string(),
        mode: "single".to_string(),
        delegate: false,
        modules: vec!["a".to_string()],
        task: None,
        ts: 2,
        agents: vec![agent_meta("a", &["a"], None)],
        exec: vm_spec(),
        sandbox: Default::default(),
        parent: Some("p".to_string()),
        node: None,
        delegation: None,
        thinking: None,
        run: RunState::Active,
        workspace: None,
    };
    hist.create(&child).expect("子工作落盘");
    let core = core_with_pkgs(
        vec![module_of("a")],
        gw(BTreeMap::new(), Vec::new()),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
        Arc::new(InMemoryPackages::empty()),
    );
    let child_meta = hist.meta("p--a").expect("读子工作");
    assert_eq!(
        core.guest_lease_for(&child_meta).expect("取租约"),
        "p",
        "proxy 子工作要加入代理根那一台 guest"
    );
    assert_eq!(
        core.guest_lease_for(&hist.meta("p").expect("代理根"))
            .expect("代理根租约"),
        "",
        "代理根自己没有 guest；由子工作按同一租约拉起"
    );
}
