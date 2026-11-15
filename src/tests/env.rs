//! 目的：钉环境事实的派生——权限链、共享区授权、guest 租约、一席的工具环境与沙箱清单。
//! 管：`env::api` 五个派生函数的行为与错误文案；纯内存，不碰端口、不落盘。
//! 不管：取事实（`conductor` 的 history / workspace / registry）、机制侧怎么用围栏。
//! 联动：`src/capabilities/env/domain/env.rs`；`conductor/service/env.rs` 取事实后转调。

use crate::capabilities::env::api::{
    MemberFacts, SandboxesFacts, member, permissions, sandboxes, workspace_grant,
};
use crate::capabilities::modules::api::Roster;
use crate::capabilities::packages::api::PackagePlan;
use crate::capabilities::permission::api::{
    PermissionDefaults, PermissionLayer, ProfileId, Reach, ReachSet, ScopeMode, SessionSandbox,
};
use crate::capabilities::registry::api::AppSettings;
use crate::capabilities::session::api::{
    AgentMeta, RunState, SessionMeta, WorkspaceBinding, WorkspaceGrant,
};
use crate::capabilities::workspace::api::{Sandbox, WorkRoots};
use crate::kernel::api::{ExecSpec, Tier};
use crate::tests::doubles::module_of;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

/// 一条最小会话 meta：权限段与工作区绑定由调用方按需覆盖。
fn meta(name: &str, parent: Option<&str>, tier: Tier, mode: &str) -> SessionMeta {
    SessionMeta {
        context_freshness: false,
        name: name.to_string(),
        mode: mode.to_string(),
        delegate: false,
        modules: Vec::new(),
        task: None,
        ts: 1,
        agents: Vec::new(),
        exec: ExecSpec {
            tier,
            ..Default::default()
        },
        sandbox: SessionSandbox::default(),
        parent: parent.map(str::to_string),
        node: None,
        delegation: None,
        thinking: None,
        run: RunState::Active,
        workspace: None,
    }
}

/// 一席默认权限的沙箱（模块目录为空，共享区走缺省读范围）。
fn sandbox() -> Sandbox {
    Sandbox {
        work_name: "w".to_string(),
        session: "s".to_string(),
        agent: "a".to_string(),
        shared: PathBuf::from("shared"),
        private: PathBuf::from("work/a"),
        modules: BTreeMap::new(),
        modules_with_userdata: Default::default(),
        permissions: PermissionDefaults::default().to_permission_set(),
        path_codec: None,
        shared_grant: WorkspaceGrant::Session,
    }
}

#[test]
fn permissions_narrow_with_the_chain_and_keep_the_hard_deny() {
    let mut app = AppSettings::default();
    app.sandbox.defaults.profile = ProfileId::External;
    app.sandbox.policy.deny = vec!["secret/".to_string()];
    let mut root = meta("root", None, Tier::Host, "solo");
    root.sandbox = SessionSandbox {
        ceiling: Some(PermissionLayer {
            profile: Some(ProfileId::Workspace),
            ..Default::default()
        }),
        narrowing: None,
    };
    let mut child = meta("root--a", Some("root"), Tier::Host, "collab");
    child.sandbox = SessionSandbox {
        ceiling: None,
        narrowing: Some(PermissionLayer {
            reach: Some(ReachSet::from([Reach::Private, Reach::SharedRead])),
            ..Default::default()
        }),
    };
    let out = permissions(&[root, child], &app).expect("权限链应能求值");
    assert_eq!(out.profile, ProfileId::Workspace);
    assert_eq!(
        out.reach,
        ReachSet::from([Reach::Private, Reach::SharedRead])
    );
    assert!(!out.reach.contains(Reach::HostRead));
    assert!(
        out.deny.iter().any(|d| d == "secret/"),
        "全局硬黑名单必须并上"
    );
}

#[test]
fn a_workspace_grant_of_none_closes_the_shared_root() {
    let mut root = meta("root", None, Tier::Host, "solo");
    root.workspace = Some(WorkspaceBinding {
        root: PathBuf::from("shared"),
        grant: WorkspaceGrant::None,
    });
    let child = meta("root--a", Some("root"), Tier::Host, "collab");
    let chain = vec![root, child];
    assert_eq!(workspace_grant(&chain), WorkspaceGrant::None);
    let out = permissions(&chain, &AppSettings::default()).expect("权限链应能求值");
    assert_eq!(out.shared_read, ScopeMode::None);
    assert_eq!(out.shared_write, ScopeMode::None);
    assert!(!out.reach.contains(Reach::SharedRead));
    assert!(!out.reach.contains(Reach::SharedWrite));
    assert!(!out.read_ok("notes/a.md"));
    assert!(!out.write_ok("notes/a.md"));
}

#[test]
fn member_env_merges_package_roots_once_and_only_host_tier_keeps_base_roots() {
    let sb = sandbox();
    let plan = PackagePlan {
        path: Some(OsString::from("rt/bin")),
        roots: vec![
            PathBuf::from("rt/pkg-b"),
            PathBuf::from("rt/pkg-a"),
            PathBuf::from("rt/pkg-a"),
        ],
        ..Default::default()
    };
    let base = [PathBuf::from("base-b"), PathBuf::from("base-a")];
    let module_vars =
        BTreeMap::from([("m1".to_string(), vec![("K".to_string(), "V".to_string())])]);
    let host = member(&MemberFacts {
        sandbox: &sb,
        host_read_roots: &[],
        net: false,
        tier: Tier::Host,
        base_roots: &base,
        plan: &plan,
        module_vars: &module_vars,
    });
    assert_eq!(host.runtime_path, Some(OsString::from("rt/bin")));
    assert_eq!(
        host.runtime_roots,
        vec![
            PathBuf::from("base-a"),
            PathBuf::from("base-b"),
            PathBuf::from("rt/pkg-a"),
            PathBuf::from("rt/pkg-b"),
        ]
    );
    assert_eq!(host.module_env, module_vars);
    assert_eq!(
        host.fence
            .ro_tree
            .iter()
            .filter(|p| p.ends_with("pkg-a"))
            .count(),
        1,
        "包根并进围栏只留一份"
    );
    let vm = member(&MemberFacts {
        sandbox: &sb,
        host_read_roots: &[],
        net: false,
        tier: Tier::Vm,
        base_roots: &base,
        plan: &plan,
        module_vars: &module_vars,
    });
    assert_eq!(
        vm.runtime_roots,
        vec![PathBuf::from("rt/pkg-a"), PathBuf::from("rt/pkg-b")],
        "VM 档不带宿主工具链根"
    );
}

#[test]
fn sandboxes_give_each_agent_only_its_own_modules() {
    let mut m2 = module_of("m2");
    m2.has_userdata = true;
    let roster = Roster {
        modules: vec![module_of("m1"), m2],
        rejected: Vec::new(),
    };
    let roots = WorkRoots {
        shared: PathBuf::from("shared"),
        session_root: PathBuf::from("sessions/w"),
        agents: BTreeMap::from([
            ("a".to_string(), PathBuf::from("work/a")),
            ("b".to_string(), PathBuf::from("work/b")),
        ]),
        store: PathBuf::from("store"),
    };
    let agents = vec![agent("a", &["m1"]), agent("b", &["m2"])];
    let facts = SandboxesFacts {
        work: "w",
        session: "s",
        agents: &agents,
        roots: &roots,
        roster: &roster,
        codec: None,
        grant: WorkspaceGrant::Session,
    };
    let out = sandboxes(&facts, &|_| {
        Ok(PermissionDefaults::default().to_permission_set())
    })
    .expect("沙箱应能组装");
    assert_eq!(out.list.len(), 2);
    let a = &out.list[0];
    assert_eq!(a.agent, "a");
    assert_eq!(a.private, PathBuf::from("work/a"));
    assert_eq!(a.modules.keys().cloned().collect::<Vec<_>>(), vec!["m1"]);
    assert!(a.modules_with_userdata.is_empty());
    let b = &out.list[1];
    assert_eq!(b.agent, "b");
    assert_eq!(b.private, PathBuf::from("work/b"));
    assert_eq!(b.modules.keys().cloned().collect::<Vec<_>>(), vec!["m2"]);
    assert_eq!(
        b.modules_with_userdata.iter().cloned().collect::<Vec<_>>(),
        vec!["m2"]
    );
    assert_eq!(out.shared, PathBuf::from("shared"));
    assert_eq!(out.session_root, PathBuf::from("sessions/w"));
}

/// 造一条 agent 名单记录（模块清单由参数给）。
fn agent(name: &str, modules: &[&str]) -> AgentMeta {
    AgentMeta {
        name: name.to_string(),
        transient: false,
        modules: modules.iter().map(|s| s.to_string()).collect(),
        model: None,
        chara: String::new(),
    }
}
