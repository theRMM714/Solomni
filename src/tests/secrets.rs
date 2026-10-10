//! 目的：隐秘字段的契约用例——声明视图、置值 / 清值、按模块解析、脱敏。
//! 管：声明与配置状态、值只落存储、解析只给本模块、脱敏替换已知值、未声明字段拒收、已有值读回。
//! 不管：文件落盘的真实实现（在 detail/yaml_secrets.rs，随后续真实消费点补契约）；实际注入子进程。
//! 联动：能力见 src/capabilities/secrets/；替身见 src/tests/doubles.rs。

use crate::capabilities::secrets::api::SecretOps;
use crate::capabilities::secrets::ports::SecretStore;
use crate::capabilities::secrets::service::SecretsService;
use crate::capabilities::workspace::api::{Module, SecretDecl};
use crate::tests::doubles::{
    module_of, test_workspace, InMemoryPackages, InMemorySecretStore, InMemoryWorkspace, VecSource,
};
use std::collections::BTreeMap;
use std::sync::Arc;

/// 一个声明了隐秘字段的模块。
fn module_with_secret(id: &str, name: &str, env: &str) -> Module {
    let mut m = module_of(id);
    m.manifest.secrets.insert(
        name.to_string(),
        SecretDecl {
            env: env.to_string(),
            desc: String::new(),
        },
    );
    m
}

/// 装好内存存储的隐秘字段能力（清单来自真 WorkspaceService + 内存端口）。
fn svc(modules: Vec<Module>) -> (SecretsService, Arc<InMemorySecretStore>) {
    let store = Arc::new(InMemorySecretStore::new());
    let workspace = test_workspace(
        Arc::new(VecSource(modules)),
        Arc::new(InMemoryPackages::empty()),
        Arc::new(InMemoryWorkspace::new()),
    );
    let service = SecretsService::new(
        workspace,
        Arc::clone(&store) as Arc<dyn SecretStore + Send + Sync>,
    )
    .expect("装配");
    (service, store)
}

/// 声明视图 + 置值落盘 + 按模块解析 + 脱敏 + 清值。
#[test]
fn secrets_set_resolve_and_redact() {
    let (service, store) = svc(vec![module_with_secret("m1", "token", "TOKEN")]);
    let list = service.declared().expect("声明视图");
    assert_eq!(list.len(), 1);
    assert!(!list[0].configured, "初始未配置");

    service.set("m1", "token", "s3cr3t").expect("置值");
    assert!(service.declared().expect("视图")[0].configured);
    assert_eq!(store.saves(), 1, "置值要落盘一次");

    let resolved = service.resolve("m1").expect("解析");
    assert_eq!(resolved, vec![("TOKEN".to_string(), "s3cr3t".to_string())]);

    let redacted = service
        .redact("m1", "输出里有 s3cr3t 这个词")
        .expect("脱敏");
    assert!(
        !redacted.contains("s3cr3t") && redacted.contains("***"),
        "{}",
        redacted
    );

    service.clear("m1", "token").expect("清值");
    assert!(service.resolve("m1").expect("解析").is_empty());
}

/// 未声明的字段不许置值（不能凭空冒出一个字段）。
#[test]
fn secrets_reject_undeclared_fields() {
    let (service, _store) = svc(vec![module_with_secret("m1", "token", "TOKEN")]);
    let err = service.set("m1", "nope", "x").unwrap_err();
    assert!(err.contains("没有声明"), "{}", err);
}

/// 已有的值在装配时读回（跨进程重启后仍生效）。
#[test]
fn secrets_load_existing_values() {
    let mut initial = BTreeMap::new();
    initial.insert("m1/token".to_string(), "old".to_string());
    let store = Arc::new(InMemorySecretStore::with(initial));
    let workspace = test_workspace(
        Arc::new(VecSource(vec![module_with_secret("m1", "token", "TOKEN")])),
        Arc::new(InMemoryPackages::empty()),
        Arc::new(InMemoryWorkspace::new()),
    );
    let service = SecretsService::new(
        workspace,
        Arc::clone(&store) as Arc<dyn SecretStore + Send + Sync>,
    )
    .expect("装配");
    assert!(
        service.declared().expect("视图")[0].configured,
        "已有值要读回"
    );
    assert_eq!(service.resolve("m1").expect("解析").len(), 1);
}

/// 一次性模块工具进程也注入该模块的隐秘字段（与常驻服务同一把尺子）。
#[test]
fn module_tool_process_gets_module_secrets() {
    use crate::capabilities::conductor::api::{ActionCall, Caller, ConductorHandle, Ops, Output};
    use crate::kernel::ports::ProcessRunner;
    use crate::tests::builders::EnvRecordingRunner;
    use crate::tests::doubles::{core_with_services, decl, gw};

    let mut m = module_with_secret("m1", "token", "TOKEN");
    m.manifest
        .tools
        .insert("echo".to_string(), decl("python tools/echo.py"));
    let modules = vec![m];
    let runner = Arc::new(EnvRecordingRunner::new());
    let store = Arc::new(InMemorySecretStore::new());
    let workspace = test_workspace(
        Arc::new(VecSource(modules.clone())),
        Arc::new(InMemoryPackages::empty()),
        Arc::new(InMemoryWorkspace::new()),
    );
    let secrets = Arc::new(
        SecretsService::new(
            Arc::clone(&workspace),
            Arc::clone(&store) as Arc<dyn SecretStore + Send + Sync>,
        )
        .expect("secrets"),
    );
    secrets.set("m1", "token", "s3cr3t").expect("置值");
    let handle = ConductorHandle::spawn(core_with_services(
        modules,
        gw(BTreeMap::new(), Vec::new()),
        Arc::clone(&runner) as Arc<dyn ProcessRunner>,
        Arc::new(crate::capabilities::residents::api::NoResidents),
        Arc::clone(&secrets) as Arc<dyn SecretOps + Send + Sync>,
    ))
    .expect("起核心线程");
    let ops: Ops = crate::capabilities::conductor::api::Ops::from_handle(&handle);

    ops.actions
        .act(ActionCall {
            id: "module.m1.echo".to_string(),
            args: serde_json::json!({}),
            caller: Caller::User,
            out: Output::Final,
        })
        .expect("跑模块工具");
    let envs = runner.envs.lock().expect("锁");
    assert!(
        envs.iter()
            .any(|e| e.iter().any(|(k, v)| k == "TOKEN" && v == "s3cr3t")),
        "工具进程要拿到该模块的隐秘字段：{:?}",
        envs
    );
}

/// 设置面动作：`set_secret` / `clear_secret` 走统一动作面（只给 user；值不回显）。
#[test]
fn secret_actions_set_and_clear() {
    use crate::capabilities::conductor::api::{ActionCall, Caller, ConductorHandle, Ops, Output};
    use crate::tests::doubles::{core_with_services, gw};

    let modules = vec![module_with_secret("m1", "token", "TOKEN")];
    let store = Arc::new(InMemorySecretStore::new());
    let workspace = test_workspace(
        Arc::new(VecSource(modules.clone())),
        Arc::new(InMemoryPackages::empty()),
        Arc::new(InMemoryWorkspace::new()),
    );
    let secrets = Arc::new(
        SecretsService::new(
            Arc::clone(&workspace),
            Arc::clone(&store) as Arc<dyn SecretStore + Send + Sync>,
        )
        .expect("secrets"),
    );
    let residents: Arc<dyn crate::capabilities::residents::api::ResidentOps + Send + Sync> =
        Arc::new(crate::capabilities::residents::api::NoResidents);
    let handle = ConductorHandle::spawn(core_with_services(
        modules,
        gw(BTreeMap::new(), Vec::new()),
        Arc::new(crate::tests::builders::SilentRunner),
        residents,
        Arc::clone(&secrets) as Arc<dyn SecretOps + Send + Sync>,
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
            "set_secret",
            serde_json::json!({ "module": "m1", "name": "token", "value": "v1" }),
        ))
        .expect("置值动作");
    assert_eq!(
        secrets.resolve("m1").expect("解析"),
        vec![("TOKEN".to_string(), "v1".to_string())]
    );
    ops.actions
        .act(call(
            "clear_secret",
            serde_json::json!({ "module": "m1", "name": "token" }),
        ))
        .expect("清值动作");
    assert!(secrets.resolve("m1").expect("解析").is_empty());
}
