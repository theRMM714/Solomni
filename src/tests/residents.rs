//! 目的：常驻服务的契约用例——注册表、生命周期、租约与假适配器。
//! 管：发现 / 启动 / 调用 / 停止 / 回收，未知适配器与禁用的拒收，未启动即调用。
//! 不管：真实协议与进程（随 MCP/ACP 适配器接入）；模块声明解析（workspace 的用例）。
//! 联动：能力见 src/capabilities/residents/；替身见 src/tests/doubles.rs。

use crate::capabilities::residents::api::{ResidentOps, ServiceState};
use crate::capabilities::residents::ports::ServiceAdapter;
use crate::capabilities::residents::service::ResidentsService;
use crate::capabilities::workspace::api::{Module, ServiceDecl};
use crate::tests::doubles::{
    module_of, test_workspace, FakeServiceAdapter, InMemoryPackages, InMemoryWorkspace, VecSource,
};
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
    let workspace = test_workspace(
        Arc::new(VecSource(modules)),
        Arc::new(InMemoryPackages::empty()),
        Arc::new(InMemoryWorkspace::new()),
    );
    let service = ResidentsService::new(
        vec![Arc::clone(&fake) as Arc<dyn ServiceAdapter + Send + Sync>],
        workspace,
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    );
    (service, fake)
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
    assert!(err.contains("禁用"), "{}", err);
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
    use crate::capabilities::secrets::api::SecretOps;
    use crate::capabilities::secrets::ports::SecretStore;
    use crate::capabilities::secrets::service::SecretsService;
    use crate::capabilities::workspace::api::SecretDecl;
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
    let workspace = test_workspace(
        Arc::new(VecSource(modules)),
        Arc::new(InMemoryPackages::empty()),
        Arc::new(InMemoryWorkspace::new()),
    );
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

/// `control_resident` 动作走统一动作面（参数校验 + 授权 + 分发）：start 返回发现的操作，stop 停实例。
#[test]
fn control_resident_action_starts_and_stops() {
    use crate::capabilities::conductor::api::{ActionCall, Caller, ConductorHandle, Ops, Output};

    let fake = Arc::new(FakeServiceAdapter::new());
    let modules = vec![module_with_service("m1", "fake", true)];
    let workspace = test_workspace(
        Arc::new(VecSource(modules.clone())),
        Arc::new(InMemoryPackages::empty()),
        Arc::new(InMemoryWorkspace::new()),
    );
    let residents: Arc<dyn ResidentOps + Send + Sync> = Arc::new(ResidentsService::new(
        vec![Arc::clone(&fake) as Arc<dyn ServiceAdapter + Send + Sync>],
        workspace,
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    ));
    let gateway = crate::tests::doubles::gw(BTreeMap::new(), Vec::new());
    let handle = ConductorHandle::spawn(crate::tests::doubles::core_with_services(
        modules,
        gateway,
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
    let workspace = test_workspace(
        Arc::new(VecSource(modules.clone())),
        Arc::new(InMemoryPackages::empty()),
        Arc::new(InMemoryWorkspace::new()),
    );
    let residents: Arc<dyn ResidentOps + Send + Sync> = Arc::new(ResidentsService::new(
        vec![Arc::clone(&fake) as Arc<dyn ServiceAdapter + Send + Sync>],
        workspace,
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    ));
    let gateway = crate::tests::doubles::gw(BTreeMap::new(), Vec::new());
    let handle = ConductorHandle::spawn(crate::tests::doubles::core_with_services(
        modules,
        gateway,
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

/// 没启动就调用：如实拒绝（不静默跑一次）。
#[test]
fn residents_call_requires_a_running_service() {
    let (service, _fake) = svc(vec![module_with_service("m1", "fake", true)]);
    let err = service
        .call("m1", "svc", "echo", &serde_json::json!({}))
        .unwrap_err();
    assert!(err.contains("没在跑"), "{}", err);
}
