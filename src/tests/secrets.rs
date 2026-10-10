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
