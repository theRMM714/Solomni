//! 目的：常驻服务的契约用例——注册表、生命周期、租约与假适配器。
//! 管：发现 / 启动 / 调用 / 停止 / 回收，未知适配器与禁用的拒收，未启动即调用。
//! 不管：真实协议与进程（随 MCP/ACP 适配器接入）；模块声明与启停有效值（modules 的用例）。
//! 联动：能力见 src/capabilities/residents/；替身见 src/tests/doubles.rs。

use crate::capabilities::modules::api::{Module, ServiceDecl};
use crate::capabilities::residents::api::{ResidentOps, ServiceState};
use crate::capabilities::residents::ports::ServiceAdapter;
use crate::capabilities::residents::service::ResidentsService;
use crate::tests::doubles::{module_of, test_modules, FakeServiceAdapter, VecSource};
use std::collections::BTreeMap;
use std::sync::Arc;

/// 一个声明了常驻服务的模块。
fn module_with_service(id: &str, adapter: &str, enabled: bool) -> Module {
    let mut m = module_of(id);
    m.manifest.services.insert(
        "svc".to_string(),
        ServiceDecl {
            adapter: adapter.to_string(),
            command: "run".to_string(),
            desc: String::new(),
            enabled,
            options: BTreeMap::new(),
        },
    );
    m
}

/// 装好假适配器的常驻服务能力（清单来自真 WorkspaceService + 内存端口）。
fn svc(modules: Vec<Module>) -> (ResidentsService, Arc<FakeServiceAdapter>) {
    let fake = Arc::new(FakeServiceAdapter::new());
    let workspace = test_modules(Arc::new(VecSource(modules)));
    let service = ResidentsService::new(
        vec![Arc::clone(&fake) as Arc<dyn ServiceAdapter + Send + Sync>],
        workspace,
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    );
    (service, fake)
}

/// 调一个不可用的服务：回执按**真实原因**说明（未启动 / 已停用 / 模块停用）。
#[test]
fn calling_an_unavailable_service_reports_the_reason() {
    let modules_face = test_modules(Arc::new(VecSource(vec![module_with_service(
        "m1", "fake", true,
    )])));
    let residents: Arc<dyn ResidentOps + Send + Sync> = Arc::new(ResidentsService::new(
        vec![Arc::new(FakeServiceAdapter::new()) as Arc<dyn ServiceAdapter + Send + Sync>],
        Arc::clone(&modules_face),
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    ));
    let args = serde_json::json!({});

    let err = residents.call("m1", "svc", "op", &args).unwrap_err();
    assert!(err.contains("未启动"), "{}", err);

    // 起过再停：不是"从没起过"，但运行态已摘除，如实回未启动。
    residents.start("m1", "svc", "").expect("启动");
    residents.stop("m1", "svc").expect("停掉");
    let err = residents.call("m1", "svc", "op", &args).unwrap_err();
    assert!(err.contains("未启动"), "{}", err);

    modules_face
        .set_service_enabled("m1", "svc", false)
        .expect("停用服务");
    let err = residents.call("m1", "svc", "op", &args).unwrap_err();
    assert!(err.contains("已停用"), "{}", err);

    modules_face
        .set_module_enabled("m1", false)
        .expect("停用模块");
    let err = residents.call("m1", "svc", "op", &args).unwrap_err();
    assert!(err.contains("模块 m1 已停用"), "{}", err);
}

/// 启动拉起：只拉起**有效启用**的服务（作者默认停用的不拉）。
#[test]
fn start_enabled_only_launches_services_that_are_on() {
    let (on, _fake_on) = svc(vec![module_with_service("m1", "fake", true)]);
    let lines = on.start_enabled();
    assert_eq!(lines.len(), 1, "启用的服务要拉起：{:?}", lines);
    assert!(lines[0].contains("已拉起"), "拉起要如实报告：{:?}", lines);

    let (off, _fake_off) = svc(vec![module_with_service("m1", "fake", false)]);
    assert!(off.start_enabled().is_empty(), "作者默认停用的服务不拉起");
}

/// control_resident 的 enable / disable 与 set_module_enabled 都落盘到 modules（启停归它）。
#[test]
fn enable_disable_and_module_toggle_persist_to_modules() {
    use crate::capabilities::conductor::api::{ActionCall, Caller, ConductorHandle, Ops, Output};

    let fake = Arc::new(FakeServiceAdapter::new());
    let modules = vec![module_with_service("m1", "fake", true)];
    let residents: Arc<dyn ResidentOps + Send + Sync> = Arc::new(ResidentsService::new(
        vec![Arc::clone(&fake) as Arc<dyn ServiceAdapter + Send + Sync>],
        test_modules(Arc::new(VecSource(modules.clone()))),
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    ));
    let gateway = crate::tests::doubles::gw(BTreeMap::new(), Vec::new());
    let handle = ConductorHandle::spawn(crate::tests::doubles::core_with_services(
        modules,
        gateway,
        Arc::new(crate::tests::builders::SilentRunner),
        residents,
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    ))
    .expect("起核心线程");
    let ops: Ops = crate::capabilities::conductor::api::Ops::from_handle(&handle);
    let call = |id: &str, args: serde_json::Value| ActionCall {
        id: id.to_string(),
        args,
        caller: Caller::User,
        out: Output::Final,
    };

    ops.actions
        .act(call(
            "control_resident",
            serde_json::json!({ "module": "m1", "name": "svc", "action": "disable" }),
        ))
        .expect("停用服务");
    assert!(
        !ops.modules.service_enabled("m1", "svc").expect("读服务"),
        "disable 要落盘"
    );
    ops.actions
        .act(call(
            "control_resident",
            serde_json::json!({ "module": "m1", "name": "svc", "action": "enable" }),
        ))
        .expect("启用服务");
    assert!(
        ops.modules.service_enabled("m1", "svc").expect("读服务"),
        "enable 要落盘"
    );
    ops.actions
        .act(call(
            "set_module_enabled",
            serde_json::json!({ "module": "m1", "enabled": false }),
        ))
        .expect("停用模块");
    assert!(
        !ops.modules.module_enabled("m1").expect("读模块"),
        "模块停用要落盘"
    );
    assert!(
        !ops.modules.service_enabled("m1", "svc").expect("读服务"),
        "模块停用 ⇒ 服务有效停用"
    );
}

/// 发现 → 启动 → 调用 → 租约回收；回执与状态如实。
#[test]
fn residents_start_call_stop_and_reap() {
    let (service, fake) = svc(vec![module_with_service("m1", "fake", true)]);
    let list = service.services().expect("列服务");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].state, ServiceState::Stopped, "启用但未启动");

    let ops = service.start("m1", "svc", "lease-1").expect("启动");
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].name, "echo");
    assert_eq!(
        service.services().expect("列服务")[0].state,
        ServiceState::Ready
    );

    let r = service
        .call("m1", "svc", "echo", &serde_json::json!({"x":1}))
        .expect("调用");
    assert!(r.ok && r.output.contains("echo"), "回执：{}", r.output);

    service.reap("lease-1").expect("回收");
    assert_eq!(
        service.services().expect("列服务")[0].state,
        ServiceState::Stopped
    );
    assert!(
        fake.calls.lock().expect("锁").iter().any(|c| c == "stop"),
        "租约回收要停实例"
    );
}

/// 禁用（enable=false）的服务不启动，也不在列表里显示成可启动。
#[test]
fn residents_disabled_service_does_not_start() {
    let (service, _fake) = svc(vec![module_with_service("m1", "fake", false)]);
    assert_eq!(
        service.services().expect("列服务")[0].state,
        ServiceState::Disabled
    );
    let err = service.start("m1", "svc", "").unwrap_err();
    assert!(err.contains("未启用"), "{}", err);
}

/// 未知适配器：列表如实标注并列出可用项；启动被拒。
#[test]
fn residents_unknown_adapter_is_reported_with_available_names() {
    let (service, _fake) = svc(vec![module_with_service("m1", "nope", true)]);
    let view = &service.services().expect("列服务")[0];
    assert!(
        view.reason.contains("未知适配器") && view.reason.contains("fake"),
        "{}",
        view.reason
    );
    let err = service.start("m1", "svc", "").unwrap_err();
    assert!(err.contains("未知适配器"), "{}", err);
}

/// 起服务时注入该模块的隐秘字段（env），回执按已知值脱敏。
#[test]
fn residents_inject_module_secrets_and_redact_receipts() {
    use crate::capabilities::modules::api::SecretDecl;
    use crate::capabilities::secrets::api::SecretOps;
    use crate::capabilities::secrets::ports::SecretStore;
    use crate::capabilities::secrets::service::SecretsService;
    use crate::tests::doubles::InMemorySecretStore;

    let mut modules = vec![module_with_service("m1", "fake", true)];
    modules[0].manifest.secrets.insert(
        "token".to_string(),
        SecretDecl {
            env: "TOKEN".to_string(),
            desc: String::new(),
        },
    );
    let fake = Arc::new(FakeServiceAdapter::new());
    let workspace = test_modules(Arc::new(VecSource(modules)));
    let secrets = Arc::new(
        SecretsService::new(
            Arc::clone(&workspace),
            Arc::new(InMemorySecretStore::new()) as Arc<dyn SecretStore + Send + Sync>,
        )
        .expect("secrets"),
    );
    secrets.set("m1", "token", "s3cr3t").expect("置值");
    let service = ResidentsService::new(
        vec![Arc::clone(&fake) as Arc<dyn ServiceAdapter + Send + Sync>],
        workspace,
        Arc::clone(&secrets) as Arc<dyn SecretOps + Send + Sync>,
    );

    service.start("m1", "svc", "").expect("启动");
    assert!(
        fake.envs()
            .iter()
            .any(|(k, v)| k == "TOKEN" && v == "s3cr3t"),
        "起服务要注入该模块的隐秘字段：{:?}",
        fake.envs()
    );
    let r = service
        .call("m1", "svc", "echo", &serde_json::json!({}))
        .expect("调用");
    assert!(
        !r.output.contains("s3cr3t") && r.output.contains("***"),
        "回执要脱敏：{}",
        r.output
    );
}

/// 崩溃检测：服务进程结束时如实标失败、从运行态摘除，后续调用如实报"没在跑"，租约回收清掉失败态。
#[test]
fn residents_mark_failed_when_service_process_dies() {
    use crate::tests::doubles::DyingServiceAdapter;

    let workspace = test_modules(Arc::new(VecSource(vec![module_with_service(
        "m1", "dying", true,
    )])));
    let service = ResidentsService::new(
        vec![Arc::new(DyingServiceAdapter) as Arc<dyn ServiceAdapter + Send + Sync>],
        workspace,
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    );
    service.start("m1", "svc", "lease-d").expect("启动");
    assert_eq!(
        service.services().expect("列服务")[0].state,
        ServiceState::Ready
    );

    let receipt = service
        .call("m1", "svc", "boom", &serde_json::json!({}))
        .expect("调用给出回执");
    assert!(
        !receipt.ok && receipt.output.contains("已结束"),
        "{}",
        receipt.output
    );
    let view = &service.services().expect("列服务")[0];
    assert!(
        matches!(view.state, ServiceState::Failed(_)),
        "进程死了要如实标失败：{:?}",
        view.state
    );

    let err = service
        .call("m1", "svc", "boom", &serde_json::json!({}))
        .unwrap_err();
    assert!(err.contains("进程已结束"), "{}", err);

    // 租约回收把失败态也清掉：会话没了，失败记录不该留。
    service.reap("lease-d").expect("回收");
    assert_eq!(
        service.services().expect("列服务")[0].state,
        ServiceState::Stopped
    );
}

/// `control_resident` 动作走统一动作面（参数校验 + 授权 + 分发）：start 返回发现的操作，stop 停实例。
#[test]
fn control_resident_action_starts_and_stops() {
    use crate::capabilities::conductor::api::{ActionCall, Caller, ConductorHandle, Ops, Output};

    let fake = Arc::new(FakeServiceAdapter::new());
    let modules = vec![module_with_service("m1", "fake", true)];
    let workspace = test_modules(Arc::new(VecSource(modules.clone())));
    let residents: Arc<dyn ResidentOps + Send + Sync> = Arc::new(ResidentsService::new(
        vec![Arc::clone(&fake) as Arc<dyn ServiceAdapter + Send + Sync>],
        workspace,
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    ));
    let gateway = crate::tests::doubles::gw(BTreeMap::new(), Vec::new());
    let handle = ConductorHandle::spawn(crate::tests::doubles::core_with_services(
        modules,
        gateway,
        Arc::new(crate::tests::builders::SilentRunner),
        residents,
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    ))
    .expect("起核心线程");
    let ops: Ops = crate::capabilities::conductor::api::Ops::from_handle(&handle);
    let call = |action: &str| ActionCall {
        id: "control_resident".to_string(),
        args: serde_json::json!({ "module": "m1", "name": "svc", "action": action }),
        caller: Caller::User,
        out: Output::Final,
    };

    ops.actions.act(call("start")).expect("start 动作");
    ops.actions.act(call("stop")).expect("stop 动作");
    assert!(
        fake.calls.lock().expect("锁").iter().any(|c| c == "stop"),
        "动作面 stop 要停实例"
    );
}

/// 已启动服务的操作走 `module.<id>.<service>.<op>` 动作（静态模块工具优先，其次常驻服务操作）。
#[test]
fn service_operations_are_module_actions() {
    use crate::capabilities::conductor::api::{
        Acted, ActionCall, Caller, ConductorHandle, Ops, Output,
    };

    let fake = Arc::new(FakeServiceAdapter::new());
    let modules = vec![module_with_service("m1", "fake", true)];
    let workspace = test_modules(Arc::new(VecSource(modules.clone())));
    let residents: Arc<dyn ResidentOps + Send + Sync> = Arc::new(ResidentsService::new(
        vec![Arc::clone(&fake) as Arc<dyn ServiceAdapter + Send + Sync>],
        workspace,
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    ));
    let gateway = crate::tests::doubles::gw(BTreeMap::new(), Vec::new());
    let handle = ConductorHandle::spawn(crate::tests::doubles::core_with_services(
        modules,
        gateway,
        Arc::new(crate::tests::builders::SilentRunner),
        residents,
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    ))
    .expect("起核心线程");
    let ops: Ops = crate::capabilities::conductor::api::Ops::from_handle(&handle);

    ops.actions
        .act(ActionCall {
            id: "control_resident".to_string(),
            args: serde_json::json!({ "module": "m1", "name": "svc", "action": "start" }),
            caller: Caller::User,
            out: Output::Final,
        })
        .expect("启动服务");
    let acted = ops
        .actions
        .act(ActionCall {
            id: "module.m1.svc.echo".to_string(),
            args: serde_json::json!({ "x": 1 }),
            caller: Caller::User,
            out: Output::Final,
        })
        .expect("调用服务操作");
    match acted {
        Acted::Done(v) => assert!(
            v["output"].as_str().unwrap_or("").contains("echo"),
            "回执应含操作名：{}",
            v
        ),
        other => panic!("应为 Done，实际：{:?}", other),
    }
    let err = ops
        .actions
        .act(ActionCall {
            id: "module.m1.svc.nope".to_string(),
            args: serde_json::json!({}),
            caller: Caller::User,
            out: Output::Final,
        })
        .unwrap_err();
    assert!(err.contains("没有操作"), "未知操作要如实拒绝：{}", err);
}

/// 动作目录：就绪服务的操作进目录（Web/CLI 的动作面板共用）；未启动不出现。
#[test]
fn action_catalog_lists_ready_service_operations() {
    use crate::capabilities::conductor::api::{ActionCall, Caller, ConductorHandle, Ops, Output};

    let fake = Arc::new(FakeServiceAdapter::new());
    let modules = vec![module_with_service("m1", "fake", true)];
    let workspace = test_modules(Arc::new(VecSource(modules.clone())));
    let residents: Arc<dyn ResidentOps + Send + Sync> = Arc::new(ResidentsService::new(
        vec![Arc::clone(&fake) as Arc<dyn ServiceAdapter + Send + Sync>],
        workspace,
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    ));
    let gateway = crate::tests::doubles::gw(BTreeMap::new(), Vec::new());
    let handle = ConductorHandle::spawn(crate::tests::doubles::core_with_services(
        modules,
        gateway,
        Arc::new(crate::tests::builders::SilentRunner),
        residents,
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    ))
    .expect("起核心线程");
    let ops: Ops = crate::capabilities::conductor::api::Ops::from_handle(&handle);

    let before = ops.actions.catalog(&Caller::User, None).expect("目录");
    assert!(
        !before.iter().any(|a| a.id == "module.m1.svc.echo"),
        "未启动的服务操作不该出现在目录里"
    );
    assert!(
        before.iter().any(|a| a.id == "control_resident"),
        "生命周期动作应当在目录里"
    );

    ops.actions
        .act(ActionCall {
            id: "control_resident".to_string(),
            args: serde_json::json!({ "module": "m1", "name": "svc", "action": "start" }),
            caller: Caller::User,
            out: Output::Final,
        })
        .expect("启动服务");
    let after = ops.actions.catalog(&Caller::User, None).expect("目录");
    assert!(
        after.iter().any(|a| a.id == "module.m1.svc.echo"),
        "就绪服务的操作要进目录"
    );
}

/// 没启动就调用：如实拒绝（不静默跑一次）。
#[test]
fn residents_call_requires_a_running_service() {
    let (service, _fake) = svc(vec![module_with_service("m1", "fake", true)]);
    let err = service
        .call("m1", "svc", "echo", &serde_json::json!({}))
        .unwrap_err();
    assert!(err.contains("未启动"), "{}", err);
}

/// 目的：常驻服务也按宿主目标取运行包：PATH 与围栏只读根都来自 packages 计划。
#[test]
fn resident_service_gets_host_package_path_and_roots() {
    use crate::tests::doubles::{test_packages, FixedProbe, InMemoryPackages};
    let dir = crate::tests::scratch("resident-host-pkg");
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).expect("建 bin");
    let mut m = module_with_service("m1", "fake", true);
    m.manifest.runtimes = vec!["pkgtool".to_string()];
    let mut p = crate::tests::doubles::pkg("pkgtool", "1");
    p.source_dir = dir.clone();
    let packages = test_packages(Arc::new(InMemoryPackages::from(vec![p])));
    let probe = Arc::new(FixedProbe {
        dirs: vec![bin.clone()],
        ..Default::default()
    });
    let fake = Arc::new(FakeServiceAdapter::new());
    let service = ResidentsService::new(
        vec![Arc::clone(&fake) as Arc<dyn ServiceAdapter + Send + Sync>],
        test_modules(Arc::new(VecSource(vec![m]))),
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    )
    .with_host_runtime(packages, probe);
    service.start("m1", "svc", "").expect("服务要起得来");
    let paths = fake.paths();
    let path = paths[0].as_ref().expect("常驻服务要拿到包 PATH");
    let first = std::env::split_paths(path).next().expect("PATH 第一段");
    assert_eq!(first, bin, "包 bin 前置到服务 PATH");
    assert!(fake.roots()[0].contains(&dir), "包根要进围栏只读根");
    let _ = std::fs::remove_dir_all(&dir);
}
