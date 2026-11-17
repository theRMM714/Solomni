//! 目的：隐秘字段能力的行为用例——只经 `capabilities/catalog/secrets/api.rs` 的面。
//! 管：声明视图、置值 / 清值 / 注入项、回执脱敏，以及未知模块与未声明字段的拒收。
//! 不管：`.home/secrets.yaml` 的落盘形态（由组合根的 `secrets` 装配路径承载）；提示词侧的"值不进转录"。
//! 联动：src/capabilities/catalog/secrets/api.rs；装配见 src/entry/assembly.rs。

use super::prelude::*;
use crate::capabilities::catalog::modules::api::SecretDecl;

/// 目的：一个声明了 `token` 字段的模块（只声明标识与 env 名，值永不进清单）。
fn demo_modules() -> Vec<Module> {
    let mut m = module_of("demo");
    m.manifest.secrets.insert(
        "token".to_string(),
        SecretDecl {
            env: "DEMO_TOKEN".to_string(),
            desc: "演示令牌".to_string(),
        },
    );
    vec![m]
}

/// 目的：装好的能力面 + 可观察的存储替身。
fn demo() -> (
    Arc<dyn crate::capabilities::catalog::secrets::api::SecretOps + Send + Sync>,
    Arc<InMemorySecretStore>,
) {
    let store = Arc::new(InMemorySecretStore::new());
    let modules = test_modules(Arc::new(VecSource(demo_modules())));
    let port: Arc<dyn crate::capabilities::catalog::secrets::ports::SecretStore + Send + Sync> =
        store.clone();
    let ops = crate::entry::assembly::secrets_with(modules, port).expect("装隐秘字段能力面");
    (ops, store)
}

#[test]
pub(crate) fn declared_lists_fields_with_configured_flag_but_never_values() {
    let (ops, _store) = demo();
    let list = ops.declared().expect("声明清单");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].module, "demo");
    assert_eq!(list[0].name, "token");
    assert_eq!(list[0].env, "DEMO_TOKEN");
    assert!(!list[0].configured, "没置值就是未配置");
    ops.set("demo", "token", "s3cret").expect("置值");
    assert!(
        ops.declared().expect("声明清单")[0].configured,
        "置值后可见已配置"
    );
}

#[test]
pub(crate) fn set_persists_and_resolve_gives_the_injection_pair() {
    let (ops, store) = demo();
    ops.set("demo", "token", "s3cret").expect("置值");
    assert_eq!(store.saves(), 1, "置值立即落盘一次");
    assert_eq!(
        ops.resolve("demo").expect("注入项"),
        vec![("DEMO_TOKEN".to_string(), "s3cret".to_string())]
    );
}

#[test]
pub(crate) fn redact_masks_known_values_in_receipts() {
    let (ops, _store) = demo();
    ops.set("demo", "token", "s3cret").expect("置值");
    let out = ops
        .redact("demo", "回执里出现了 s3cret 这个值")
        .expect("脱敏");
    assert_eq!(out, "回执里出现了 *** 这个值");
}

#[test]
pub(crate) fn clear_drops_the_value_but_keeps_the_declaration() {
    let (ops, store) = demo();
    ops.set("demo", "token", "s3cret").expect("置值");
    ops.clear("demo", "token").expect("清值");
    assert!(
        ops.resolve("demo").expect("注入项").is_empty(),
        "清值后不再注入"
    );
    let list = ops.declared().expect("声明清单");
    assert_eq!(list.len(), 1, "声明还在");
    assert!(!list[0].configured);
    assert_eq!(store.saves(), 2, "置值与清值各落盘一次");
}

#[test]
pub(crate) fn unknown_module_and_undeclared_field_are_refused() {
    let (ops, _store) = demo();
    let e = ops.set("nope", "token", "v").expect_err("未知模块必须被拒");
    assert!(e.contains("清单里没有模块 nope"), "{}", e);
    let e = ops
        .set("demo", "nope", "v")
        .expect_err("未声明的字段必须被拒");
    assert!(e.contains("没有声明隐秘字段 nope"), "{}", e);
    let e = ops.resolve("nope").expect_err("未知模块解析必须被拒");
    assert!(e.contains("清单里没有模块 nope"), "{}", e);
}
