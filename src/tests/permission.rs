//! 目的：钉收敛后的权限模型——两档网络开关、共享区路径归属、固定授予的围栏，以及工作区提交工具的消费。
//! 管：档位与父子收窄、共享区三态与覆盖/收窄/黑名单、沙箱落点、围栏规格、提交与读作用域过滤。
//! 不管：平台真机验收（tests/<平台>/）；确认队列本身（见 src/tests/session/）。
//! 联动：src/units/pure/permission/、src/units/business/workspace/domain/scope.rs、docs/permission/README.md。

use crate::tests::doubles::{
    InMemoryWorkStore, InMemoryWorkspace, abs, test_sandbox, test_workspace, test_workspace_store,
};
use crate::units::business::workbench::api::Area;
use crate::units::pure::permission::api::{
    PermissionDefaults, PermissionLayer, PermissionOverride, PermissionSet, ProfileId,
};
use crate::units::pure::permission::api::{ScopeMode, SharedEdit, SharedScope, validate_entry};

fn paths(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn pset(profile: ProfileId) -> PermissionSet {
    PermissionDefaults { profile }.to_permission_set()
}

/// 目的：档位只有两档，且**只有网络开关不同**——其余一切固定授予，不由档位表达。
#[test]
fn profiles_are_two_tiers_that_differ_only_in_network() {
    assert_eq!(ProfileId::all(), [ProfileId::Offline, ProfileId::Online]);
    assert_eq!(ProfileId::default(), ProfileId::Offline);
    assert!(!pset(ProfileId::Offline).allows_network());
    assert!(pset(ProfileId::Online).allows_network());
    for p in ProfileId::all() {
        assert_eq!(ProfileId::parse(p.as_str()), Ok(p), "写法要能往返");
        assert!(!p.label().is_empty());
        assert!(!p.summary().is_empty());
        assert_eq!(p.spec().id, p);
    }
    let why = ProfileId::parse("workspace").expect_err("旧档位已不存在");
    assert!(why.contains("offline"), "{}", why);
}

/// 目的：父子合并只动档位——根可换档，子只能更严。
#[test]
fn permission_set_root_and_narrowing_only_touch_the_tier() {
    let base = pset(ProfileId::Offline);
    let root = base
        .apply_root(&PermissionLayer {
            profile: Some(ProfileId::Online),
        })
        .expect("根可以换档");
    assert!(root.allows_network());
    let child = pset(ProfileId::Online)
        .apply_narrowing(&PermissionLayer {
            profile: Some(ProfileId::Offline),
        })
        .expect("子会话可以更严");
    assert!(!child.allows_network());
    let why = pset(ProfileId::Offline)
        .apply_narrowing(&PermissionLayer {
            profile: Some(ProfileId::Online),
        })
        .expect_err("子会话不能放宽");
    assert!(why.contains("更宽"), "{}", why);
    // 空层什么都不改：不声明就不动。
    assert_eq!(
        pset(ProfileId::Online),
        pset(ProfileId::Online)
            .apply_root(&PermissionLayer::default())
            .unwrap()
    );
    let _ = PermissionOverride::default();
}

/// 目的：共享区范围是三态，空集合不再兼作整棵。
#[test]
fn shared_scope_is_explicitly_tristate() {
    let all = ScopeMode::All;
    let none = ScopeMode::None;
    let docs = ScopeMode::Paths(paths(&["docs"]));
    assert!(all.allows("anything/at/all"));
    assert!(!none.allows("docs/x"));
    assert!(docs.allows("docs/x"));
    assert!(docs.allows("docs"));
    assert!(!docs.allows("docsx/y"), "按路径组件匹配，不做字符串前缀");
    assert_eq!(all.intersect(&docs), docs);
    assert_eq!(none.intersect(&all), none);
    assert_eq!(
        docs.intersect(&ScopeMode::Paths(paths(&["docs/a"]))),
        ScopeMode::Paths(paths(&["docs/a"]))
    );
    assert_eq!(docs.intersect(&ScopeMode::Paths(paths(&["src"]))), none);
    assert!(none.is_subset_of(&docs));
    assert!(docs.is_subset_of(&all));
    assert!(!all.is_subset_of(&docs));
    assert!(!docs.is_subset_of(&none));

    // serde：三个写法都能读，别的不认。
    let read = |s: &str| serde_json::from_str::<ScopeMode>(s).expect(s);
    assert_eq!(read("\"all\""), ScopeMode::All);
    assert_eq!(read("\"none\""), ScopeMode::None);
    assert_eq!(
        read(r#"["docs","src"]"#),
        ScopeMode::Paths(paths(&["docs", "src"]))
    );
    assert!(serde_json::from_str::<ScopeMode>("\"every\"").is_err());
}

/// 目的：沙箱落点——私有区恒可写、模块目录恒只读、共享主副本只读，读共享区由归属判。
#[test]
fn sandbox_places_are_fixed_except_the_shared_scope() {
    let mut sb = test_sandbox("a1", &["data"]);
    let shared = sb.shared.clone();
    let module = sb.modules.get("data").expect("模块目录").clone();
    assert!(sb.can_read(&Area::Shared, &shared.join("docs").join("x")));
    assert!(
        !sb.can_write(&Area::Shared),
        "共享区主副本永不被直接写：改动经 work_commit"
    );
    assert!(sb.can_write(&Area::Private), "沙箱恒可写");
    assert!(sb.can_read(&Area::Module("data".to_string()), &module.join("script.py")));
    assert!(
        !sb.can_write(&Area::Module("data".to_string())),
        "模块目录对 agent 整块只读（userdata 只给模块进程自己）"
    );

    // 共享区读范围：三态直接生效，黑名单优先。
    sb.scope = SharedScope {
        read: ScopeMode::Paths(paths(&["docs"])),
        write: ScopeMode::All,
        deny: paths(&["docs/secret.txt"]),
    };
    assert!(sb.can_read(&Area::Shared, &shared.join("docs").join("x")));
    assert!(!sb.can_read(&Area::Shared, &shared.join("src").join("x")));
    assert!(!sb.can_read(&Area::Shared, &shared.join("docs").join("secret.txt")));
    sb.scope.read = ScopeMode::None;
    assert!(!sb.can_read(&Area::Shared, &shared.join("docs").join("x")));
}

/// 目的：围栏按固定规则推——私有区 + 模块私有区可写，模块源码目录只读，共享区永不进围栏。
#[test]
fn fence_spec_grants_fixed_surfaces() {
    use crate::kernel::api::fence::{Access, FencePlan, Reach};
    let mut sb = test_sandbox("a1", &[]);
    let module = abs(&["mods", "data"]);
    sb.modules.insert("data".to_string(), module.clone());
    sb.module_data
        .insert("data".to_string(), module.join("userdata"));

    let session = FencePlan::for_session(&sb, &[], false);
    assert!(
        session.paths(Access::Read, Reach::Tree).contains(&module),
        "模块目录对 agent 只读"
    );
    assert!(
        session.write_roots().contains(&module.join("userdata")),
        "模块进程可写自己的 userdata"
    );
    assert!(
        !session.write_roots().contains(&module),
        "agent 不能整块写模块目录"
    );
    assert!(
        !session.write_roots().contains(&sb.shared),
        "共享区永不被直接写"
    );
    assert!(session.write_roots().contains(&sb.private), "私有沙箱可写");
    assert_eq!(session.home, sb.private, "HOME/TEMP 落在私有沙箱");
    assert!(!session.net.on(), "离线档不给网络");

    // 一次调用：模块工具带上该模块的 userdata；不带模块就不给。
    let call = FencePlan::for_call(&sb, Some("data"), &session);
    assert!(call.write_roots().contains(&module.join("userdata")));
    assert!(call.write_roots().contains(&sb.private));
    assert!(!call.write_roots().contains(&module));
    assert!(!call.paths(Access::Read, Reach::Tree).contains(&sb.shared));
    let call = FencePlan::for_call(&sb, None, &session);
    assert!(!call.write_roots().contains(&module.join("userdata")));
    assert!(call.paths(Access::Read, Reach::Tree).contains(&module));

    // 联网档 + 全局开网：围栏才带 net；关全局则仍是 off。
    sb.permissions = pset(ProfileId::Online);
    assert!(FencePlan::for_session(&sb, &[], true).net.on());
    assert!(!FencePlan::for_session(&sb, &[], false).net.on());
    sb.permissions = pset(ProfileId::Offline);
    assert!(!FencePlan::for_session(&sb, &[], true).net.on());
}

/// 目的：外部工作区的共享根不是可写入面——shell 的围栏不含它，内置工具也不能写它。
/// 约束：外部磁盘的改动由 work_sync 采纳；work_adopt_sandbox 对它返回空是预期，不是门禁例外。
#[test]
fn external_shared_root_is_not_a_shell_write_surface() {
    use crate::kernel::api::fence::{Access, FencePlan, Reach};
    let external = abs(&["external", "proj"]);
    let mut sb = test_sandbox("a1", &[]);
    sb.shared = external.clone();
    let session = FencePlan::for_session(&sb, &[], false);
    assert!(
        !session.write_roots().contains(&external),
        "共享根永不被直接写"
    );
    let call = FencePlan::for_call(&sb, None, &session);
    assert!(
        !call.write_roots().contains(&external)
            && !call.paths(Access::Read, Reach::Tree).contains(&external),
        "按次围栏不含共享根：{:?}",
        call.write_roots()
    );
    assert!(!sb.can_write(&Area::Shared));
}

/// 目的：work_status 的路径清单按读归属过滤，显式越界路径直接拒绝。
#[test]
fn work_status_filters_paths_outside_read_scope() {
    use crate::kernel::ports::{ToolCtx, ToolHandler};
    use crate::units::business::versions::api::{CommitRequest, PushRequest, WorkHandler};

    let dirs = std::sync::Arc::new(InMemoryWorkspace::new());
    let store = std::sync::Arc::new(InMemoryWorkStore::new());
    let ws = test_workspace_store(dirs, store.clone());
    ws.ensure_layout("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    let sb_b = ws
        .roots("w", &["b".to_string()])
        .unwrap()
        .agents
        .get("b")
        .cloned()
        .unwrap();
    store.write_under(&sb_b, "public.txt", b"p").unwrap();
    store.write_under(&sb_b, "secret.txt", b"s").unwrap();
    let req = CommitRequest {
        paths: vec!["public.txt".to_string(), "secret.txt".to_string()],
        deletes: Vec::new(),
        message: "提交".to_string(),
        time: 1,
        line: 1,
    };
    ws.work_commit("w", "b", "w--b", &req).unwrap();
    ws.work_push(
        "w",
        "b",
        "w--b",
        &PushRequest {
            message: "发布".to_string(),
            time: 1,
            line: 1,
        },
    )
    .unwrap();
    ws.work_pull(
        "w",
        "a",
        &["public.txt".to_string(), "secret.txt".to_string()],
    )
    .unwrap();
    let sb_a = ws
        .roots("w", &["a".to_string()])
        .unwrap()
        .agents
        .get("a")
        .cloned()
        .unwrap();
    store.write_under(&sb_a, "extra-secret.txt", b"x").unwrap();

    let scope = SharedScope {
        read: ScopeMode::Paths(paths(&["public.txt"])),
        ..Default::default()
    };
    let h = WorkHandler::new(ws.versions.clone(), "w", "a", scope);
    let ctx = ToolCtx {
        session: "s",
        agent: "a",
        line: 1,
    };
    let out = h.run(&ctx, "work_status", "{}");
    assert!(out.ok, "{}", out.output);
    assert!(out.output.contains("public.txt"), "{}", out.output);
    assert!(!out.output.contains("secret.txt"), "{}", out.output);
    assert!(!out.output.contains("extra-secret.txt"), "{}", out.output);

    let denied = h.run(&ctx, "work_status", r#"{"paths":["secret.txt"]}"#);
    assert!(
        !denied.ok && denied.output.contains("读取被拒"),
        "{}",
        denied.output
    );
}

/// 目的：work_lost 的清单与恢复也按读归属收口，不能拿备份绕过范围。
#[test]
fn work_lost_is_scoped_by_read_permission() {
    use crate::kernel::ports::{ToolCtx, ToolHandler};
    use crate::units::business::versions::api::WorkHandler;

    let dirs = std::sync::Arc::new(InMemoryWorkspace::new());
    let store = std::sync::Arc::new(InMemoryWorkStore::new());
    let ws = test_workspace_store(dirs, store.clone());
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    let root = ws.roots("w", &[]).unwrap().store.join("lost").join("a");
    store
        .write_under(&root, "public/lost.txt", b"public-lost")
        .unwrap();
    store
        .write_under(&root, "secret/lost.txt", b"secret-lost")
        .unwrap();

    let scope = SharedScope {
        read: ScopeMode::Paths(paths(&["public"])),
        ..Default::default()
    };
    let h = WorkHandler::new(ws.versions.clone(), "w", "a", scope);
    let ctx = ToolCtx {
        session: "s",
        agent: "a",
        line: 1,
    };
    let out = h.run(&ctx, "work_lost", "{}");
    assert!(out.ok, "{}", out.output);
    assert!(out.output.contains("public/lost.txt"), "{}", out.output);
    assert!(!out.output.contains("secret/lost.txt"), "{}", out.output);

    let denied = h.run(&ctx, "work_lost", r#"{"path":"secret/lost.txt"}"#);
    assert!(
        !denied.ok && denied.output.contains("读取被拒"),
        "{}",
        denied.output
    );
    let restore = h.run(
        &ctx,
        "work_lost",
        r#"{"path":"secret/lost.txt","restore":true,"expect_current":""}"#,
    );
    assert!(
        !restore.ok && restore.output.contains("读取被拒"),
        "{}",
        restore.output
    );
}

/// 目的：work_versions / work_resolve 也按读归属收口，不能拿它们绕过范围。
#[test]
fn work_versions_and_resolve_are_scoped_by_read_permission() {
    use crate::kernel::ports::{ToolCtx, ToolHandler};
    use crate::units::business::versions::api::WorkHandler;
    let ws = test_workspace(std::sync::Arc::new(InMemoryWorkspace::new()));
    let scope = SharedScope {
        read: ScopeMode::Paths(paths(&["docs"])),
        ..Default::default()
    };
    let h = WorkHandler::new(ws.versions.clone(), "w", "a", scope);
    let ctx = ToolCtx {
        session: "s",
        agent: "a",
        line: 1,
    };
    let v = h.run(&ctx, "work_versions", r#"{"path":"secret/x"}"#);
    assert!(!v.ok && v.output.contains("读取被拒"), "{}", v.output);
    let r = h.run(
        &ctx,
        "work_resolve",
        r#"{"paths":["secret/x"],"take":"local"}"#,
    );
    assert!(!r.ok && r.output.contains("读取被拒"), "{}", r.output);

    let bad_take = h.run(
        &ctx,
        "work_resolve",
        r#"{"paths":["docs/a"],"take":"nope"}"#,
    );
    assert!(!bad_take.ok && bad_take.output.contains("upstream 或 local"));
    let no_path = h.run(&ctx, "work_versions", "{}");
    assert!(!no_path.ok && no_path.output.contains("path"));
}

/// 目的：没有提交范围时，work_commit / work_push 直接拒绝——范围是准入门槛，不是提示。
#[test]
fn no_write_scope_refuses_commit_and_push() {
    use crate::kernel::ports::{ToolCtx, ToolHandler};
    use crate::units::business::versions::api::WorkHandler;
    let ws = test_workspace(std::sync::Arc::new(InMemoryWorkspace::new()));
    let scope = SharedScope {
        write: ScopeMode::None,
        ..Default::default()
    };
    let h = WorkHandler::new(ws.versions.clone(), "w", "a", scope);
    let ctx = ToolCtx {
        session: "s",
        agent: "a",
        line: 1,
    };
    let c = h.run(&ctx, "work_commit", "{}");
    assert!(!c.ok && c.output.contains("提交被拒"), "{}", c.output);
    let p = h.run(&ctx, "work_push", "{}");
    assert!(!p.ok && p.output.contains("推送被拒"), "{}", p.output);
}

/// 目的：条目写法是硬校验——相对、无空段、无上级段；点代表整棵。
#[test]
fn entries_must_be_clean_relative_paths() {
    for bad in ["", "  ", "/abs", "C:/x", "a//b", "a/../b", "\\srv"] {
        assert!(validate_entry(bad).is_err(), "{:?} 应被拒", bad);
    }
    for ok in ["docs", "docs/x", "a.b/c", "."] {
        assert!(
            validate_entry(ok).is_ok(),
            "{:?} 应通过：{:?}",
            ok,
            validate_entry(ok)
        );
    }
    let scope = SharedScope {
        read: ScopeMode::Paths(paths(&["/abs"])),
        ..Default::default()
    };
    let why = scope.validate().expect_err("绝对路径条目要报错");
    assert!(why.contains("shared.read"), "{}", why);

    // 黑名单命中：空表 = 不拦；"." = 整棵拦截；按组件匹配。
    assert!(!crate::units::pure::permission::api::hit(&[], "docs/x"));
    assert!(crate::units::pure::permission::api::hit(
        &paths(&["docs"]),
        "docs/x"
    ));
    assert!(crate::units::pure::permission::api::hit(
        &paths(&["docs"]),
        "docs"
    ));
    assert!(!crate::units::pure::permission::api::hit(
        &paths(&["docs"]),
        "docsx/y"
    ));
    assert!(crate::units::pure::permission::api::hit(
        &paths(&["."]),
        "anything"
    ));
    // 两侧空白与尾部分隔符削掉；组件内部的空格是合法目录名，原样保留。
    assert_eq!(
        crate::units::pure::permission::api::normalize("  docs\\sub\\ "),
        "docs/sub"
    );
    assert_eq!(
        crate::units::pure::permission::api::normalize("docs/ 子 目录"),
        "docs/ 子 目录"
    );
}

/// 目的：覆盖 / 收窄只会更严——子会话交上来的任何一条范围都必须落在父的范围内。
#[test]
fn shared_scope_narrowing_never_widens() {
    let modes = [
        ScopeMode::All,
        ScopeMode::None,
        ScopeMode::Paths(paths(&["docs"])),
        ScopeMode::Paths(paths(&["docs/a"])),
        ScopeMode::Paths(paths(&["src"])),
    ];
    for parent in &modes {
        let base = SharedScope {
            read: parent.clone(),
            write: parent.clone(),
            deny: Vec::new(),
        };
        for child in &modes {
            let edit = SharedEdit {
                read: Some(child.clone()),
                write: Some(child.clone()),
                deny: Vec::new(),
            };
            match base.apply_narrowing(&edit) {
                Ok(out) => {
                    assert!(
                        out.read.is_subset_of(parent),
                        "收窄结果必须落在父范围内：{:?} vs {:?}",
                        out.read,
                        parent
                    );
                    assert_eq!(out.read, parent.intersect(child));
                }
                Err(why) => {
                    assert!(
                        !child.is_subset_of(parent),
                        "只有放宽才该被拒：{:?} vs {:?}（{}）",
                        child,
                        parent,
                        why
                    );
                }
            }
        }
        // 根覆盖是整体替换：任意三态都合法。
        for child in &modes {
            let edit = SharedEdit {
                read: Some(child.clone()),
                ..Default::default()
            };
            let out = base.apply_root(&edit).expect("根覆盖不设上限");
            assert_eq!(&out.read, child);
        }
    }
}

/// 目的：共享区配置的读写形状固定——未知字段拒绝、缺省整棵、黑名单追加去重。
#[test]
fn shared_scope_serde_and_merge_edges() {
    let full: SharedScope =
        serde_json::from_str(r#"{"read":"none","write":["docs"],"deny":["secret"]}"#).unwrap();
    assert_eq!(full.read, ScopeMode::None);
    assert_eq!(full.write, ScopeMode::Paths(paths(&["docs"])));
    assert_eq!(full.deny, paths(&["secret"]));
    let empty: SharedScope = serde_json::from_str("{}").unwrap();
    assert_eq!(empty, SharedScope::default());
    assert!(
        serde_json::from_str::<SharedScope>(r#"{"nope":1}"#).is_err(),
        "未知字段要拒"
    );

    let base = SharedScope::default();
    let out = base
        .apply_root(&SharedEdit {
            read: Some(ScopeMode::None),
            write: None,
            deny: paths(&["secret", "secret", " keys "]),
        })
        .unwrap();
    assert_eq!(out.read, ScopeMode::None);
    assert_eq!(out.write, ScopeMode::All, "没声明的面不改");
    assert_eq!(out.deny, paths(&["secret", " keys "]), "黑名单去重");
    assert!(!out.read_ok("docs/x"), "读范围关了就是关了");
    assert!(out.write_ok("docs/x"), "写范围没动");
    assert_eq!(out.read_roots_listing(), "（无）");
    assert_eq!(out.with_hard_deny(&paths(&["secret"])).deny.len(), 2);
}
