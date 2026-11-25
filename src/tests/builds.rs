//! 目的：构建子业务的行为用例——只经 `units/maintenance/builds/api.rs` 的面。
//! 管：目标名、入口判据（在场才跳过）、没有 build 段时的两种结局（跳过 / 如实拒收）。
//! 不管：真实编译产物与缓存卷（虚拟机档，见 T5 旅程 `vm` 与缺口账）；指纹算法。
//! 联动：src/units/maintenance/builds/api.rs；装配见 src/entry/assembly.rs。

use super::prelude::*;
use crate::units::catalog::modules::api::ToolDecl;
use crate::units::maintenance::builds::api::{Builds, Target};

/// 目的：组合根装出来的真实构建入站面（落点在 `target/test-scratch`，不碰用户目录）。
fn service(tag: &str) -> impl Builds {
    crate::entry::assembly::build_service(&super::scratch(tag)).expect("装构建入站面")
}

/// 目的：声明了一个"模块内产物"入口的模块（入口约定是相对路径）。
fn module_with_entry(id: &str, command: &str) -> Module {
    let mut m = module_of(id);
    m.manifest.tools.insert(
        "indexer".to_string(),
        ToolDecl {
            command: command.to_string(),
            desc: "建索引".to_string(),
            params: None,
            parallel: false,
        },
    );
    m
}

#[test]
pub(crate) fn target_labels_are_what_build_state_records() {
    assert_eq!(Target::Host.label(), "host");
    assert_eq!(Target::Guest("dev".to_string()).label(), "dev");
}

#[test]
pub(crate) fn module_with_missing_entry_and_no_build_section_is_refused() {
    let builds = service("builds-missing");
    let m = module_with_entry("demo", "build/indexer");
    let e = builds
        .ensure_many(&[&m], &Target::Host, false)
        .expect_err("入口不在场且没声明 build 段必须拒收");
    assert!(e.contains("入口不在场"), "{}", e);
    assert!(e.contains("没声明 build 段"), "{}", e);
}

#[test]
pub(crate) fn module_without_tools_needs_no_build_and_is_skipped() {
    let builds = service("builds-skip");
    let m = module_of("plain");
    let out = builds
        .ensure_many(&[&m], &Target::Host, false)
        .expect("跳过不算失败");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].module, "plain");
    assert_eq!(out[0].target, "host");
    assert!(out[0].skipped, "没有工具入口就没有可编的东西");
    assert!(out[0].output.is_empty());
}

#[test]
pub(crate) fn explicit_build_without_declaration_explains_where_to_declare_it() {
    let builds = service("builds-decl");
    let m = module_with_entry("demo", "build/indexer");
    let e = builds
        .build(&m, &Target::Host, false)
        .expect_err("没有 build 段不能静默当成编过");
    assert!(e.contains("没声明 build 段"), "{}", e);
}
