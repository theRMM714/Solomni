//! 目的：钉环境事实的派生——权限链、guest 租约、一席的工具环境与沙箱清单、全局策略（网络开关与宿主只读根）。
//! 管：`env::api` 六个派生函数的行为与错误文案；纯内存，不碰端口、不落盘。
//! 不管：取事实（`runtime` 的 history / workspace / registry）、机制侧怎么用围栏。
//! 联动：`src/units/env/domain/env.rs`；`runtime/service/env.rs` 取事实后转调。

use crate::units::catalog::modules::api::Roster;
use crate::units::catalog::packages::api::PackagePlan;
use crate::units::catalog::registry::api::AgentMeta;
use crate::units::catalog::registry::api::AppSettings;
use crate::units::conversation::session::api::{MEETING, RunState, SINGLE, SessionMeta};
use crate::units::permission::api::{
    PermissionDefaults, PermissionLayer, ProfileId, SessionSandbox,
};
use crate::units::turn::env::api::{
    MemberFacts, SandboxesFacts, SeatAccess, host_read_roots, member, network_allowed, permissions,
    sandboxes, shared_scope,
};
use crate::units::workspace::api::{Sandbox, ScopeMode, SharedEdit, SharedScope, WorkRoots};
use crate::units::exec_model::api::ExecSpec;
use crate::units::exec_model::api::Tier;
use crate::tests::doubles::module_of;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

/// 一条最小会话 meta：权限段与工作区绑定由调用方按需覆盖。
fn meta(name: &str, parent: Option<&str>, tier: Tier, kind: &str) -> SessionMeta {
    SessionMeta {
        usage_totals: Default::default(),
        context_freshness: false,
        freshness_keep_percent: 80,
        name: name.to_string(),
        spec: crate::tests::builders::seeded_spec(kind),
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
        shared: SharedEdit::default(),
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
        scope: SharedScope::default(),
        path_codec: None,
        filter_rules: PathBuf::from(".work").join(".workignore"),
    }
}

/// 目的：网络档位沿链求值——全局默认 → 根覆盖 → 每层子会话收窄；子会话不能选比父更宽的档。
#[test]
fn profile_narrows_with_the_chain() {
    let mut app = AppSettings::default();
    app.sandbox.defaults.profile = ProfileId::Online;
    let mut root = meta("root", None, Tier::Host, SINGLE);
    root.sandbox = SessionSandbox {
        ceiling: Some(PermissionLayer {
            profile: Some(ProfileId::Offline),
        }),
        narrowing: None,
    };
    let mut child = meta("root--a", Some("root"), Tier::Host, MEETING);
    child.sandbox = SessionSandbox {
        ceiling: None,
        narrowing: Some(PermissionLayer {
            profile: Some(ProfileId::Offline),
        }),
    };
    let out = permissions(&[root.clone(), child.clone()], &app).expect("档位链应能求值");
    assert_eq!(out.profile, ProfileId::Offline);
    assert!(
        !out.allows_network(),
        "根会话关掉的网络，子会话不能自己打开"
    );
    // 子会话显式选联网 = 比父更宽：拒绝，而不是悄悄按联网跑。
    child.sandbox.narrowing = Some(PermissionLayer {
        profile: Some(ProfileId::Online),
    });
    let why = permissions(&[root, child], &app).expect_err("子会话不能放宽档位");
    assert!(why.contains("更宽"), "拒绝文案要说清是「更宽」：{}", why);
}

/// 目的：共享区归属沿链求值——全局默认 → 根覆盖 → 子会话收窄；全局硬黑名单最后并上。
#[test]
fn shared_scope_applies_root_then_narrows_and_keeps_hard_deny() {
    let mut app = AppSettings::default();
    app.sandbox.scope = SharedScope {
        read: ScopeMode::All,
        write: ScopeMode::Paths(vec!["docs".to_string()]),
        deny: Vec::new(),
    };
    app.sandbox.policy.deny = vec!["secret/".to_string()];
    let mut root = meta("root", None, Tier::Host, SINGLE);
    root.shared = SharedEdit {
        read: Some(ScopeMode::Paths(vec![
            "docs".to_string(),
            "src".to_string(),
        ])),
        write: Some(ScopeMode::Paths(vec!["docs".to_string()])),
        deny: Vec::new(),
    };
    let mut child = meta("root--a", Some("root"), Tier::Host, MEETING);
    child.shared = SharedEdit {
        read: Some(ScopeMode::Paths(vec!["docs".to_string()])),
        ..Default::default()
    };
    let out = shared_scope(&[root.clone(), child.clone()], &app).expect("共享区归属应能求值");
    assert_eq!(out.read, ScopeMode::Paths(vec!["docs".to_string()]));
    assert_true_ok(&out, "docs/a.txt");
    assert!(!out.read_ok("src/x.rs"), "子会话把读范围收窄到了 docs");
    assert!(
        out.deny.iter().any(|d| d == "secret/"),
        "全局硬黑名单必须并上"
    );
    assert!(!out.read_ok("secret/key.txt"), "硬黑名单优先于范围");
    // 子会话想把读范围放回整棵 = 放宽：拒绝。
    child.shared.read = Some(ScopeMode::All);
    let why = shared_scope(&[root, child], &app).expect_err("子会话不能放宽共享区范围");
    assert!(why.contains("收窄"), "拒绝文案要说清是收窄：{}", why);
}

fn assert_true_ok(scope: &SharedScope, rel: &str) {
    assert!(scope.read_ok(rel), "{} 应在读范围内", rel);
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
            .paths(
                crate::kernel::api::fence::Access::Read,
                crate::kernel::api::fence::Reach::Tree
            )
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

/// 目的：网络开关与主机只读根只从全局策略派生——关掉就断网，路径逐条去空白、丢掉空项，顺序不变。
#[test]
fn network_switch_and_host_read_roots_come_from_the_policy() {
    let mut app = AppSettings::default();
    assert!(!network_allowed(&app), "缺省不放行出站");
    app.sandbox.policy.network = true;
    assert!(
        network_allowed(&app),
        "判据只有 settings 的 sandbox.policy.network，会话无权改"
    );
    assert!(host_read_roots(&app).is_empty(), "缺省一个只读根都不放行");
    app.sandbox.policy.host_read_roots = vec!["  /tmp  ".to_string(), String::new()];
    assert_eq!(
        host_read_roots(&app),
        vec![PathBuf::from("/tmp")],
        "逐条去空白、丢掉空项，只回用户的非空根"
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
    };
    let out = sandboxes(&facts, &|_| {
        Ok(SeatAccess {
            permissions: PermissionDefaults::default().to_permission_set(),
            scope: SharedScope::default(),
        })
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
