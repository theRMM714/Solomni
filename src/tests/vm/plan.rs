//! 目的：运行计划派生规则的用例测试——宿主包选型、缺能力收口与"同一把尺子"。
//! 管：`host_plan_for` 选包与 `unavailable_modules` 的判定一致性。
//! 不管：包库本身的校验与目标筛选（`src/tests/packages.rs`）、guest 生命周期（`src/tests/vm/`）。
//! 联动：被测实现见 `src/units/business/vm/service/plan.rs`，经 `vm::api` 导出。

use super::super::prelude::*;

use crate::tests::doubles::FixedProbe;
use crate::units::business::catalog::packages::api::{BaseFacts, Library, PinPolicy, Problem};
use crate::units::business::vm::api::{
    MissingEntry, host_plan_for, plan_for_spec, unavailable_modules,
};
use std::collections::BTreeMap;

/// 目的：本机档的包计划把缺的能力按模块收口；入口不在场时 `plan_for_spec` 如实给出诊断（规则在 vm，不在会话层）。
#[test]
pub(crate) fn host_plan_selects_host_packages_and_reports_missing_capabilities() {
    // 空库 + 空基础 profile / 空 PATH：模块声明的能力没有提供者。
    let lib = Library::default();
    let modules = vec![module_with_runtimes("a", &["solidity"])];
    let probe = FixedProbe::default();
    let base = BaseFacts::default();
    let spec = ExecSpec::default();
    let plan = host_plan_for(&lib, &base, &probe, &spec, &modules, PinPolicy::Require);
    assert_eq!(
        plan.unusable,
        BTreeMap::from([("a".to_string(), vec!["solidity".to_string()])]),
        "缺的能力按模块 a 收口，只有这一条"
    );
    assert!(
        plan.problems.iter().any(|p| matches!(
            p,
            Problem::Missing { module, capability }
                if module == "a" && capability == "solidity"
        )),
        "缺包要说清是哪个模块缺哪个能力：{:?}",
        plan.problems
    );
    let missing = vec![MissingEntry {
        module: "a".to_string(),
        tool: "indexer".to_string(),
        command: "build/indexer".to_string(),
        buildable: false,
    }];
    let err = plan_for_spec(
        &crate::tests::root_dir(),
        &probe,
        &lib,
        &spec,
        &modules,
        &missing,
        None,
        false,
    )
    .expect_err("入口不在场必须拒绝");
    assert!(
        err.contains("build/indexer"),
        "错误串要指出缺的是哪个入口：{err}"
    );
    assert!(!err.is_empty(), "诊断串不能空");
}

/// 目的：`unavailable_modules` 与宿主包计划用同一把尺子——同一份事实下两边的 unusable 必须逐条相等，不许各判一次。
#[test]
pub(crate) fn unavailable_modules_uses_the_same_ruler_as_the_host_plan() {
    let lib = Library::build(
        vec![pkg("node", "20.11.1"), pkg("cc", "13.2.0")],
        Vec::new(),
    );
    let modules = vec![
        module_with_runtimes("a", &["node", "rust"]),
        module_with_runtimes("b", &["cc"]),
    ];
    // 两个包都装在缺省的 source_dir 下，bin/ 一次声明即可（本机档要求目录在场）。
    let probe = FixedProbe {
        dirs: vec![std::path::PathBuf::from("bin")],
        ..FixedProbe::default()
    };
    let base = BaseFacts::default();
    let spec = ExecSpec::default();
    let plan = host_plan_for(&lib, &base, &probe, &spec, &modules, PinPolicy::Require);
    let got = unavailable_modules(
        &crate::tests::root_dir(),
        &probe,
        &lib,
        &base,
        &spec,
        &modules,
    );
    assert_eq!(
        got, plan.unusable,
        "同一个问题只能有一个判法：unavailable_modules 必须等于宿主计划的 unusable"
    );
    assert_eq!(
        got,
        BTreeMap::from([("a".to_string(), vec!["rust".to_string()])]),
        "只有缺 rust 的模块 a 不可用，模块 b 的 cc 在库里"
    );
}
