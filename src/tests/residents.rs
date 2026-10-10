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
    ));
    let gateway = crate::tests::doubles::gw(BTreeMap::new(), Vec::new());
    let handle = ConductorHandle::spawn(crate::tests::doubles::core_with_residents(
        modules, gateway, residents,
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

/// 没启动就调用：如实拒绝（不静默跑一次）。
#[test]
fn residents_call_requires_a_running_service() {
    let (service, _fake) = svc(vec![module_with_service("m1", "fake", true)]);
    let err = service
        .call("m1", "svc", "echo", &serde_json::json!({}))
        .unwrap_err();
    assert!(err.contains("没在跑"), "{}", err);
}
