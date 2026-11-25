//! 目的：常驻服务能力的行为用例——只经 `units/residents/api.rs` 的面。
//! 管：清单与状态、起停、调用回执（含隐秘字段注入与脱敏）、崩溃标记、按租约回收。
//! 不管：真实协议进程（在 `src/units/residents/detail/`）；MCP 协议细节（由 T5 旅程 `mcp` 覆盖）。
//! 联动：src/units/residents/api.rs；装配见 src/entry/assembly.rs。

use super::prelude::*;
use crate::units::catalog::modules::api::{SecretDecl, ServiceDecl};
use crate::units::residents::api::{ResidentOps, ServiceState, ServiceView};

/// 目的：一个带两个常驻服务（回显 / 一调就死）与一个未知适配器服务、一个停用服务的模块。
fn demo_modules() -> Vec<Module> {
    let mut m = module_of("demo");
    let svc = |adapter: &str, command: &str, enabled: bool| ServiceDecl {
        adapter: adapter.to_string(),
        command: command.to_string(),
        desc: String::new(),
        enabled,
        options: Default::default(),
    };
    m.manifest
        .services
        .insert("echo".to_string(), svc("fake", "tools/echo", true));
    m.manifest
        .services
        .insert("boom".to_string(), svc("dying", "tools/boom", true));
    m.manifest
        .services
        .insert("nope".to_string(), svc("unknown", "tools/nope", true));
    m.manifest
        .services
        .insert("off".to_string(), svc("fake", "tools/off", false));
    m.manifest.secrets.insert(
        "token".to_string(),
        SecretDecl {
            env: "DEMO_TOKEN".to_string(),
            desc: "演示令牌".to_string(),
        },
    );
    vec![m]
}

/// 目的：装好的一组能力面：常驻服务 + 值面 + 可观察的假适配器。
struct Harness {
    residents: Arc<dyn ResidentOps + Send + Sync>,
    fake: Arc<FakeServiceAdapter>,
    secrets: Arc<dyn crate::units::catalog::secrets::api::SecretOps + Send + Sync>,
}

fn harness() -> Harness {
    let fake = Arc::new(FakeServiceAdapter::new());
    let modules = test_modules(Arc::new(VecSource(demo_modules())));
    let secrets = crate::entry::assembly::secrets_with(
        Arc::clone(&modules),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("装隐秘字段能力面");
    let dying: Arc<dyn crate::units::residents::ports::ServiceAdapter + Send + Sync> =
        Arc::new(DyingServiceAdapter);
    let residents = crate::entry::assembly::residents(
        vec![
            Arc::clone(&fake)
                as Arc<dyn crate::units::residents::ports::ServiceAdapter + Send + Sync>,
            dying,
        ],
        modules,
        Arc::clone(&secrets),
    );
    Harness {
        residents: Arc::new(residents),
        fake,
        secrets,
    }
}

fn view(h: &Harness, name: &str) -> ServiceView {
    h.residents
        .services()
        .expect("服务清单")
        .into_iter()
        .find(|v| v.name == name)
        .unwrap_or_else(|| panic!("清单里没有服务 {}", name))
}

#[test]
pub(crate) fn services_report_state_and_name_unknown_adapters() {
    let h = harness();
    assert_eq!(
        view(&h, "echo").state,
        ServiceState::Stopped,
        "启用但还没起"
    );
    assert!(view(&h, "echo").reason.is_empty());
    assert_eq!(
        view(&h, "off").state,
        ServiceState::Disabled,
        "作者没开默认启用"
    );
    let nope = view(&h, "nope");
    assert_eq!(nope.state, ServiceState::Stopped);
    assert!(
        nope.reason.contains("未知适配器：unknown"),
        "{}",
        nope.reason
    );
    assert!(
        nope.reason.contains("dying"),
        "可用适配器要如实列出：{}",
        nope.reason
    );
}

#[test]
pub(crate) fn start_exposes_operations_and_marks_ready() {
    let h = harness();
    let ops = h.residents.start("demo", "echo", "L1").expect("启动");
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].name, "echo");
    assert!(ops[0].parameters.is_some(), "操作带的参数契约要原样带出");
    let v = view(&h, "echo");
    assert_eq!(v.state, ServiceState::Ready);
    assert_eq!(v.operations.len(), 1);
    assert_eq!(h.fake.calls.lock().expect("锁")[0], "start:echo");
}

#[test]
pub(crate) fn call_injects_declared_secrets_and_redacts_the_receipt() {
    let h = harness();
    h.secrets.set("demo", "token", "s3cret").expect("置值");
    h.residents.start("demo", "echo", "L1").expect("启动");
    let receipt = h
        .residents
        .call("demo", "echo", "echo", &serde_json::json!({"x": 1}))
        .expect("调用");
    assert!(receipt.ok);
    assert!(
        h.fake
            .envs()
            .contains(&("DEMO_TOKEN".to_string(), "s3cret".to_string())),
        "值要进服务进程的环境变量：{:?}",
        h.fake.envs()
    );
    assert!(
        receipt.output.contains("DEMO_TOKEN=***"),
        "回执出口要脱敏：{}",
        receipt.output
    );
    assert!(
        !receipt.output.contains("s3cret"),
        "回执不能带值：{}",
        receipt.output
    );
}

#[test]
pub(crate) fn stop_returns_to_stopped_and_later_calls_are_refused() {
    let h = harness();
    h.residents.start("demo", "echo", "L1").expect("启动");
    h.residents.stop("demo", "echo").expect("停止");
    assert_eq!(view(&h, "echo").state, ServiceState::Stopped);
    let e = h
        .residents
        .call("demo", "echo", "echo", &serde_json::json!({}))
        .expect_err("停掉后调用必须被拒");
    assert!(e.contains("未启动"), "{}", e);
}

#[test]
pub(crate) fn dead_service_is_marked_failed_and_reports_the_reason() {
    let h = harness();
    h.residents.start("demo", "boom", "L2").expect("启动");
    let receipt = h
        .residents
        .call("demo", "boom", "boom", &serde_json::json!({}))
        .expect("调用本身有回执");
    assert!(!receipt.ok, "适配器失败要如实进回执");
    assert_eq!(receipt.output, "服务进程已结束（EOF）");
    assert_eq!(
        view(&h, "boom").state,
        ServiceState::Failed("服务进程已结束".to_string())
    );
    let e = h
        .residents
        .call("demo", "boom", "boom", &serde_json::json!({}))
        .expect_err("已崩溃的服务再调必须被拒");
    assert!(e.contains("进程已结束（上次：服务进程已结束）"), "{}", e);
}

#[test]
pub(crate) fn reap_stops_only_the_matching_lease_and_clears_its_failures() {
    let h = harness();
    h.residents.start("demo", "echo", "L1").expect("启动");
    h.residents.start("demo", "boom", "L2").expect("启动");
    h.residents
        .call("demo", "boom", "boom", &serde_json::json!({}))
        .expect("调用");
    h.residents.reap("L2").expect("回收 L2");
    assert_eq!(
        view(&h, "boom").state,
        ServiceState::Stopped,
        "失败记录随该租约一起清掉"
    );
    assert_eq!(
        view(&h, "echo").state,
        ServiceState::Ready,
        "别的租约不受影响"
    );
    h.residents.reap("L1").expect("回收 L1");
    assert_eq!(view(&h, "echo").state, ServiceState::Stopped);
}

#[test]
pub(crate) fn start_enabled_starts_only_author_enabled_services() {
    let h = harness();
    let lines = h.residents.start_enabled();
    assert!(
        lines.iter().any(|l| l.contains("已拉起 demo.echo")),
        "{:?}",
        lines
    );
    assert!(
        lines.iter().any(|l| l.contains("已拉起 demo.boom")),
        "{:?}",
        lines
    );
    assert!(
        !lines.iter().any(|l| l.contains("demo.off")),
        "停用的不起：{:?}",
        lines
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("demo.nope") && l.contains("拉起失败")),
        "未知适配器的拉起要如实报失败：{:?}",
        lines
    );
}
