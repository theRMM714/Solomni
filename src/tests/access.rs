//! 目的：钉住「一席的生效面」三处派生——权限链（会话模型）、共享区归属与沙箱清单（工作台）、工具运行环境（工具单元）。
//! 管：`session_model::api::permissions` 的档位求交、`workspace::api::scope_from` 与 `seat_benches` 的落值、
//!   `tools::api::member_env` 与 `host_read_roots` 的环境派生；纯内存，不碰端口、不落盘。
//! 不管：取事实（`turn` 的 service 层）、机制侧怎么用围栏。
//! 联动：`turn/turn/service/facts.rs` 取好会话链与包事实后转调这三处；改派生规则必须同改断言。

use crate::tests::doubles::module_of;
use crate::units::business::catalog::registry::api::AppSettings;
use crate::units::business::session_model::api::{
    MEETING, RunState, SINGLE, SessionMeta, WorkspaceDecl,
};
use crate::units::business::tools::api::{MemberEnvFacts, host_read_roots, member_env};
use crate::units::business::workbench::api::{Bench, WorkRoots};
use crate::units::business::workspace::api::{
    ModuleDir, SeatFacts, SeatPlan, edit_of, edit_params, scope_from, seat_benches,
};
use crate::units::pure::exec_model::api::Side;
use crate::units::pure::permission::api::{
    PermissionDefaults, PermissionLayer, PermissionOverride, ProfileId,
};
use crate::units::pure::permission::api::{ScopeMode, SharedEdit, SharedScope};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

/// 一条最小会话 meta：权限段与工作台声明由调用方按需覆盖。
fn meta(name: &str, parent: Option<&str>, tier: Side, kind: &str) -> SessionMeta {
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
        sandbox: crate::units::business::sandbox::api::SandboxDecl::of(if tier == Side::Guest {
            crate::units::business::sandbox::api::VM_POLICY
        } else {
            crate::units::business::sandbox::api::HOST_POLICY
        }),
        facts: crate::units::business::sandbox::api::SandboxFacts {
            side: tier,
            ..Default::default()
        },
        permission: PermissionOverride::default(),
        workspace: WorkspaceDecl::layout(),
        parent: parent.map(str::to_string),
        node: None,
        delegation: None,
        thinking: None,
        run: RunState::Active,
    }
}

/// 一席默认权限的沙箱（模块目录为空，共享区走缺省读范围）。
fn sandbox() -> Bench {
    Bench {
        work_name: "w".to_string(),
        session: "s".to_string(),
        agent: "a".to_string(),
        shared: PathBuf::from("shared"),
        private: PathBuf::from("work/a"),
        modules: BTreeMap::new(),
        module_data: Default::default(),
        permissions: PermissionDefaults::default().to_permission_set(),
        scope: SharedScope::default(),
        path_codec: None,
        filter_rules: PathBuf::from(".work").join(".workignore"),
    }
}

/// 把一层共享区覆盖写进一条声明（会话侧本来就只搬声明）。
fn with_shared(mut decl: WorkspaceDecl, edit: SharedEdit) -> WorkspaceDecl {
    decl.params = edit_params(&edit).expect("共享区声明应能落进参数");
    decl
}

/// 目的：网络档位沿链求值——全局默认 → 根覆盖 → 每层子会话收窄；子会话不能选比父更宽的档。
#[test]
fn profile_narrows_with_the_chain() {
    let mut app = AppSettings::default();
    app.sandbox.defaults.profile = ProfileId::Online;
    let mut root = meta("root", None, Side::Host, SINGLE);
    root.permission = PermissionOverride {
        ceiling: Some(PermissionLayer {
            profile: Some(ProfileId::Offline),
        }),
        narrowing: None,
    };
    let mut child = meta("root--a", Some("root"), Side::Host, MEETING);
    child.permission = PermissionOverride {
        ceiling: None,
        narrowing: Some(PermissionLayer {
            profile: Some(ProfileId::Offline),
        }),
    };
    let ok = crate::units::business::session_model::api::permissions(
        &[root.clone(), child.clone()],
        &app.sandbox.defaults,
    )
    .expect("档位链应能求值");
    assert_eq!(
        ok.profile,
        ProfileId::Offline,
        "根 ceiling 把联网档压到离线"
    );
    // 子会话想选联网 = 比父宽：拒绝。
    child.permission.narrowing = Some(PermissionLayer {
        profile: Some(ProfileId::Online),
    });
    let why = crate::units::business::session_model::api::permissions(
        &[root, child],
        &app.sandbox.defaults,
    )
    .expect_err("子会话不能放宽档位");
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
    let mut root = meta("root", None, Side::Host, SINGLE);
    root.workspace = with_shared(
        root.workspace.clone(),
        SharedEdit {
            read: Some(ScopeMode::Paths(vec![
                "docs".to_string(),
                "src".to_string(),
            ])),
            write: Some(ScopeMode::Paths(vec!["docs".to_string()])),
            deny: Vec::new(),
        },
    );
    let mut child = meta("root--a", Some("root"), Side::Host, MEETING);
    child.workspace = with_shared(
        child.workspace.clone(),
        SharedEdit {
            read: Some(ScopeMode::Paths(vec!["docs".to_string()])),
            ..Default::default()
        },
    );
    let out = scope_from(
        &[
            root.workspace.params.clone(),
            child.workspace.params.clone(),
        ],
        &app.sandbox.scope,
        &app.sandbox.policy.deny,
    )
    .expect("共享区归属应能求值");
    assert_eq!(out.read, ScopeMode::Paths(vec!["docs".to_string()]));
    assert!(out.read_ok("docs/a.txt"), "docs/a.txt 应在读范围内");
    assert!(!out.read_ok("src/x.rs"), "子会话把读范围收窄到了 docs");
    assert!(
        out.deny.iter().any(|d| d == "secret/"),
        "全局硬黑名单必须并上"
    );
    assert!(!out.read_ok("secret/key.txt"), "硬黑名单优先于范围");
    // 子会话想把读范围放回整棵 = 放宽：拒绝。
    child.workspace = with_shared(
        child.workspace.clone(),
        SharedEdit {
            read: Some(ScopeMode::All),
            ..Default::default()
        },
    );
    let why = scope_from(
        &[root.workspace.params, child.workspace.params],
        &app.sandbox.scope,
        &app.sandbox.policy.deny,
    )
    .expect_err("子会话不能放宽共享区范围");
    assert!(why.contains("收窄"), "拒绝文案要说清是收窄：{}", why);
}

/// 目的：一层覆盖进声明参数再读出来是同一份（会话不解释参数，只有工作台单元解释）。
#[test]
fn a_layer_round_trips_through_the_declaration_params() {
    let decl = with_shared(
        WorkspaceDecl::layout(),
        SharedEdit {
            read: Some(ScopeMode::Paths(vec!["docs".to_string()])),
            write: None,
            deny: vec!["tmp/".to_string()],
        },
    );
    assert_eq!(
        decl.policy,
        crate::units::business::session_model::api::LAYOUT_POLICY
    );
    let back = edit_of(&decl.params).expect("参数应能读回");
    assert_eq!(back.read, Some(ScopeMode::Paths(vec!["docs".to_string()])));
    assert_eq!(back.write, None, "没写的格不覆盖");
    assert_eq!(back.deny, vec!["tmp/".to_string()]);
}

#[test]
fn member_env_merges_package_roots_once_and_only_host_tier_keeps_base_roots() {
    let sb = sandbox();
    let package_roots = vec![
        PathBuf::from("rt/pkg-b"),
        PathBuf::from("rt/pkg-a"),
        PathBuf::from("rt/pkg-a"),
    ];
    let base = [PathBuf::from("base-b"), PathBuf::from("base-a")];
    let module_vars =
        BTreeMap::from([("m1".to_string(), vec![("K".to_string(), "V".to_string())])]);
    let host = member_env(&MemberEnvFacts {
        bench: &sb,
        host_read_roots: &[],
        net: false,
        side: Side::Host,
        base_roots: &base,
        package_roots: &package_roots,
        package_path: Some(OsString::from("rt/bin")),
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
    let vm = member_env(&MemberEnvFacts {
        bench: &sb,
        host_read_roots: &[],
        net: false,
        side: Side::Guest,
        base_roots: &base,
        package_roots: &package_roots,
        package_path: None,
        module_vars: &module_vars,
    });
    assert_eq!(
        vm.runtime_roots,
        vec![PathBuf::from("rt/pkg-a"), PathBuf::from("rt/pkg-b")],
        "VM 档不带宿主工具链根"
    );
}

/// 目的：主机只读根只从全局策略派生——路径逐条去空白、丢掉空项，顺序不变。
#[test]
fn host_read_roots_drop_blank_entries() {
    let mut app = AppSettings::default();
    assert!(
        host_read_roots(&app.sandbox.policy.host_read_roots).is_empty(),
        "缺省一个只读根都不放行"
    );
    app.sandbox.policy.host_read_roots = vec!["  /tmp  ".to_string(), String::new()];
    assert_eq!(
        host_read_roots(&app.sandbox.policy.host_read_roots),
        vec![PathBuf::from("/tmp")],
        "逐条去空白、丢掉空项，只回用户的非空根"
    );
}

#[test]
fn seat_benches_give_each_agent_only_its_own_modules() {
    let mut m2 = module_of("m2");
    m2.private = Some(PathBuf::from("data/modules/m2/userdata"));
    let modules = vec![
        ModuleDir {
            id: m2.manifest.id.clone(),
            root: m2.root.clone(),
            private: m2.private.clone(),
        },
        ModuleDir {
            id: module_of("m1").manifest.id,
            root: module_of("m1").root,
            private: None,
        },
    ];
    let roots = WorkRoots {
        shared: PathBuf::from("shared"),
        session_root: PathBuf::from("sessions/w"),
        agents: BTreeMap::from([
            ("a".to_string(), PathBuf::from("work/a")),
            ("b".to_string(), PathBuf::from("work/b")),
        ]),
        store: PathBuf::from("store"),
    };
    let decl = WorkspaceDecl::layout();
    let permissions = PermissionDefaults::default().to_permission_set();
    let seats = vec![
        SeatPlan {
            agent: "a".to_string(),
            modules: vec!["m1".to_string()],
            permissions: permissions.clone(),
            params: vec![decl.params.clone()],
            private: None,
        },
        SeatPlan {
            agent: "b".to_string(),
            modules: vec!["m2".to_string()],
            permissions,
            params: vec![decl.params.clone()],
            private: None,
        },
    ];
    let out = seat_benches(&SeatFacts {
        work: "w",
        session: "s",
        roots: &roots,
        codec: None,
        base_scope: &SharedScope::default(),
        hard_deny: &[],
        modules: &modules,
        seats: &seats,
    })
    .expect("沙箱应能组装");
    assert_eq!(out.list.len(), 2);
    let a = &out.list[0];
    assert_eq!(a.agent, "a");
    assert_eq!(a.private, PathBuf::from("work/a"));
    assert_eq!(a.modules.keys().cloned().collect::<Vec<_>>(), vec!["m1"]);
    assert!(a.module_data.is_empty());
    let b = &out.list[1];
    assert_eq!(b.agent, "b");
    assert_eq!(b.private, PathBuf::from("work/b"));
    assert_eq!(b.modules.keys().cloned().collect::<Vec<_>>(), vec!["m2"]);
    assert_eq!(
        b.module_data.get("m2").map(|p| p.as_path()),
        Some(std::path::Path::new("data/modules/m2/userdata"))
    );
    assert_eq!(out.shared, PathBuf::from("shared"));
    assert_eq!(out.session_root, PathBuf::from("sessions/w"));
}

/// 目的：工作区没给出某席的私有根时如实报缺哪个 agent（不猜路径）。
#[test]
fn a_seat_without_a_private_root_is_reported_by_name() {
    let roots = WorkRoots {
        shared: PathBuf::from("shared"),
        session_root: PathBuf::from("sessions/w"),
        agents: BTreeMap::new(),
        store: PathBuf::from("store"),
    };
    let why = seat_benches(&SeatFacts {
        work: "w",
        session: "s",
        roots: &roots,
        codec: None,
        base_scope: &SharedScope::default(),
        hard_deny: &[],
        modules: &[],
        seats: &[SeatPlan {
            agent: "a".to_string(),
            modules: Vec::new(),
            permissions: PermissionDefaults::default().to_permission_set(),
            params: vec![WorkspaceDecl::layout().params],
            private: None,
        }],
    })
    .expect_err("缺私有根应报错");
    assert!(
        why.contains("工作区没有给出 agent a 的工作面路径"),
        "文案要说清缺哪个 agent：{}",
        why
    );
}
