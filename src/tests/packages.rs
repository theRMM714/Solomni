//! 目的：packages 能力的用例测试——包契约、包库、目标筛选、选包计划与诊断。
//! 管：清单校验、版本/去重、冲突预检、平台分包、计划顺序与诊断说法。
//! 不管：guest 挂载与叠写（`src/tests/vm/`）、构建编排（`src/tests/diagnostics.rs`）。
//! 联动：契约见 RUNTIME_SPEC.md；被测实现见 `src/units/packages/`。

use super::builders::*;
use super::prelude::*;
use crate::units::catalog::packages::api as pkgs;
use crate::units::catalog::packages::api::{Facts, Library, PinPolicy, Problem};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// 目的：测试用的宿主事实：两个闭包就够（选型只问可执行名与 bin 目录）。
struct Probe2<'a> {
    has: &'a dyn Fn(&str) -> bool,
    bin: &'a dyn Fn(&Path) -> bool,
}

impl pkgs::ExecProbe for Probe2<'_> {
    fn has_exe(&self, name: &str) -> bool {
        (self.has)(name)
    }
    fn bin_ok(&self, dir: &Path) -> bool {
        (self.bin)(dir)
    }
}

#[test]
pub(crate) fn base_capabilities_prefer_project_then_usable_host_fallback() {
    let modules = vec![module_with_runtimes("a", &["python", "gcc"])];
    let has = |_: &str| false;
    let bin_ok = |_: &Path| true;
    let current = OsStr::new("");
    let base = vec![("python".to_string(), "3.14.9".to_string())];
    let mut fallback = BTreeMap::new();
    fallback.insert(
        "cc".to_string(),
        pkgs::HostFallback {
            path: PathBuf::from("C:/host/gcc.exe"),
            fenced: true,
            why: String::new(),
        },
    );
    let facts = Facts {
        probe: &Probe2 {
            has: &has,
            bin: &bin_ok,
        },
        current_path: current,
        base: &base,
        host_fallback: &fallback,
    };
    let plan = pkgs::plan_host(
        &ExecSpec::default(),
        &modules,
        &Library::default(),
        PinPolicy::Require,
        &facts,
    );
    assert!(plan.problems.is_empty(), "{:?}", plan.problems);
    assert_eq!(
        plan.providers.get("python"),
        Some(&pkgs::Provider::Base {
            id: "python".to_string(),
            version: "3.14.9".to_string(),
        })
    );
    assert_eq!(plan.providers.get("gcc"), Some(&pkgs::Provider::Host));
}

#[test]
pub(crate) fn base_capability_uses_host_fallback_even_when_fence_cannot_grant() {
    let modules = vec![module_with_runtimes("a", &["python"])];
    let has = |_: &str| false;
    let bin_ok = |_: &Path| true;
    let current = OsStr::new("");
    let base: Vec<(String, String)> = Vec::new();
    let mut fallback = BTreeMap::new();
    fallback.insert(
        "python".to_string(),
        pkgs::HostFallback {
            path: PathBuf::from("C:/host/python.exe"),
            fenced: false,
            why: "目录属主不是当前用户".to_string(),
        },
    );
    let facts = Facts {
        probe: &Probe2 {
            has: &has,
            bin: &bin_ok,
        },
        current_path: current,
        base: &base,
        host_fallback: &fallback,
    };
    let plan = pkgs::plan_host(
        &ExecSpec::default(),
        &modules,
        &Library::default(),
        PinPolicy::Require,
        &facts,
    );
    assert!(plan.problems.is_empty(), "{:?}", plan.problems);
    assert_eq!(plan.providers.get("python"), Some(&pkgs::Provider::Host));
    assert!(
        plan.unusable.is_empty(),
        "回退存在但围栏授不了权时仍可执行（工具层会问是否本次无围栏跑）：{:?}",
        plan.unusable
    );

    let empty = BTreeMap::new();
    let facts = Facts {
        probe: &Probe2 {
            has: &has,
            bin: &bin_ok,
        },
        current_path: current,
        base: &base,
        host_fallback: &empty,
    };
    let plan = pkgs::plan_host(
        &ExecSpec::default(),
        &modules,
        &Library::default(),
        PinPolicy::Require,
        &facts,
    );
    assert!(
        matches!(
            plan.problems.as_slice(),
            [Problem::MissingBase { capability, .. }] if capability == "python"
        ),
        "{:?}",
        plan.problems
    );
    let text = pkgs::diagnose_text(&plan.problems);
    assert!(
        text.contains("node start.js --provision --ensure=toolchain --yes"),
        "{}",
        text
    );
}

#[test]
pub(crate) fn exec_vm_tier_reports_missing_ambiguous_and_unavailable() {
    let modules = vec![module_with_runtimes("a", &["python"])];
    let empty = Library::default();
    let spec = vm_spec();
    assert_eq!(
        pkgs::diagnoses(&modules, &empty, &spec, &[]),
        vec![Problem::Missing {
            module: "a".to_string(),
            capability: "python".to_string()
        }]
    );
    let un = pkgs::plan_guest(&spec, &modules, &empty, &[]).unusable;
    assert_eq!(
        un.get("a"),
        Some(&vec!["python".to_string()]),
        "虚拟机档缺包 = 该模块工具不可用"
    );
    // 缺包 = **不创建**（不再降级）：没有工具用户完不成工作，与其拉一台跑不了工具的 guest，不如选型这步如实拒。
    let plan = pkgs::plan_guest(&spec, &modules, &empty, &[]);
    assert!(
        matches!(plan.problems.as_slice(), [Problem::Missing { .. }]),
        "{:?}",
        plan.problems
    );
    let said = pkgs::diagnose_text(&pkgs::diagnoses(&modules, &empty, &spec, &[]));
    assert!(
        said.contains("runtimes/"),
        "缺包的说法要告诉用户把包放哪：{}",
        said
    );
    // guest 镜像自带的能力不算缺（镜像里已经有解释器）：诊断空、工具可用、计划成立。
    let guest = vec!["python".to_string()];
    assert!(pkgs::diagnoses(&modules, &empty, &spec, &guest).is_empty());
    assert!(
        pkgs::plan_guest(&spec, &modules, &empty, &guest)
            .unusable
            .is_empty()
    );
    assert!(
        pkgs::plan_guest(&spec, &modules, &empty, &guest)
            .packages
            .is_empty()
    );
    // 多版本且未定版 = 不替用户选
    let two = Library::build(
        vec![pkg("python", "3.12.4"), pkg("python", "3.11.9")],
        Vec::new(),
    );
    assert!(
        pkgs::diagnoses(&modules, &two, &spec, &[])
            .iter()
            .any(|d| matches!(d, Problem::Ambiguous { .. }))
    );
    // 定版指定的版本不在库里 = 如实报
    let bad = ExecSpec {
        pins: BTreeMap::from([("python".to_string(), "9.9".to_string())]),
        ..vm_spec()
    };
    assert!(
        pkgs::diagnoses(&modules, &two, &bad, &[])
            .iter()
            .any(|d| matches!(d, Problem::UnknownPin { .. }))
    );
    // 多版本未定版 = 选型不成立：如实拒绝（这是用户要解决的选型问题，不是"缺包"）
    let refused = pkgs::plan_guest(&spec, &modules, &two, &[]);
    assert!(
        pkgs::diagnose_text(&refused.problems).contains("多个版本"),
        "{}",
        pkgs::diagnose_text(&refused.problems)
    );
    // 定版之后可以成立
    let ok = ExecSpec {
        pins: BTreeMap::from([("python".to_string(), "3.12.4".to_string())]),
        ..vm_spec()
    };
    assert!(pkgs::diagnoses(&modules, &two, &ok, &[]).is_empty());
    assert_eq!(pkgs::plan_guest(&ok, &modules, &two, &[]).packages.len(), 1);
}

#[test]
pub(crate) fn exec_vm_plan_pins_versions_and_orders_prefix_before_system() {
    let lib = Library::build(
        vec![
            pkg_yaml(
                "id: cc\nversion: 13.2.0\nkind: system\nprovides_paths: [usr/bin, usr/include]\nrequires: [binutils]",
            ),
            pkg_yaml("id: binutils\nversion: 2.42\nprefix: opt/rt/binutils2.42"),
            pkg("python", "3.12.4"),
        ],
        Vec::new(),
    );
    let modules = vec![module_with_runtimes("a", &["python", "cc"])];
    let plan = pkgs::plan_guest(&vm_spec(), &modules, &lib, &[]);
    let ids: Vec<&str> = plan.packages.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["binutils", "python", "cc"],
        "先独立前缀，后写进系统路径的包；包的 requires 走闭包"
    );
    assert_eq!(plan.packages[0].version, "2.42", "计划里是定版后的精确版本");
    assert_eq!(
        plan.packages[0].prefix, "opt/rt/binutils2.42",
        "独立前缀随计划走（装配按它挂载）"
    );
    assert_eq!(plan.packages[2].kind, "system", "写进系统路径的包排在最后");
}

#[test]
pub(crate) fn diagnose_text_spells_out_every_package_reason() {
    let missing = pkgs::diagnose_text(&[Problem::Missing {
        module: "a".to_string(),
        capability: "python".to_string(),
    }]);
    assert!(
        missing.contains("模块 a") && missing.contains("python") && missing.contains("runtimes/"),
        "{}",
        missing
    );
    let ambiguous = pkgs::diagnose_text(&[Problem::Ambiguous {
        capability: "python".to_string(),
        versions: vec!["3.11.9".to_string(), "3.12.4".to_string()],
    }]);
    assert!(
        ambiguous.contains("多个版本") && ambiguous.contains("3.12.4"),
        "{}",
        ambiguous
    );
    let bad = pkgs::diagnose_text(&[Problem::UnknownPin {
        capability: "python".to_string(),
        version: "9.9".to_string(),
    }]);
    assert!(bad.contains("定版 9.9"), "{}", bad);
    let clash = pkgs::diagnose_text(&[Problem::Conflict {
        path: "usr/lib".to_string(),
        a: "a@1".to_string(),
        b: "b@1".to_string(),
    }]);
    assert!(
        clash.contains("usr/lib") && clash.contains("a@1") && clash.contains("b@1"),
        "{}",
        clash
    );
}

#[test]
pub(crate) fn build_packages_take_guest_or_library() {
    let lib = Library::build(
        vec![
            pkg_yaml(
                "id: cc\nversion: 13.2.0\nkind: system\nprovides_paths: [usr/bin, usr/lib]\nrequires: [binutils]",
            ),
            pkg("binutils", "2.42"),
            pkg("python", "3.12.4"),
        ],
        Vec::new(),
    );
    // guest 自带的跳过；库里有的一并带上传递 requires；都没有的如实报缺。
    let (pkgs, missing) = pkgs::build_packages(
        &["python".to_string(), "cc".to_string(), "rustc".to_string()],
        &lib,
        &["python".to_string()],
    );
    let ids: Vec<&str> = pkgs.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids.len(), 2, "cc 与它的传递依赖 binutils：{:?}", ids);
    assert!(
        ids.contains(&"cc") && ids.contains(&"binutils"),
        "{:?}",
        ids
    );
    assert!(
        !ids.contains(&"python"),
        "guest 自带的运行能力不装载包：{:?}",
        ids
    );
    assert_eq!(missing, vec!["rustc".to_string()], "缺的能力要如实报");
    let cc = pkgs.iter().find(|p| p.id == "cc").expect("有 cc");
    assert_eq!(cc.kind, "system");
    assert_eq!(
        cc.provides_paths,
        vec!["usr/bin".to_string(), "usr/lib".to_string()]
    );
}

#[test]
pub(crate) fn library_rejects_invalid_manifests_and_duplicate_versions() {
    let lib = Library::build(
        vec![
            pkg_yaml("id: Bad/UPPER\nversion: 1\nprefix: opt/x"),
            pkg("a", "1"),
            pkg("a", "1"),
        ],
        vec!["目录里有一份读不了的文件".to_string()],
    );
    assert_eq!(lib.packages.len(), 1, "只留下先出现的 a@1");
    assert_eq!(lib.packages[0].id, "a");
    assert_eq!(
        lib.rejected.len(),
        3,
        "三类拒收都如实留下：{:?}",
        lib.rejected
    );
    assert!(
        lib.rejected.iter().any(|r| r.contains("id 不合法")),
        "{:?}",
        lib.rejected
    );
    assert!(
        lib.rejected
            .iter()
            .any(|r| r.contains("同一 id 与版本出现两次")),
        "{:?}",
        lib.rejected
    );
    assert!(
        lib.rejected.iter().any(|r| r.contains("读不了的文件")),
        "适配层带入的拒收不能丢：{:?}",
        lib.rejected
    );
}

#[test]
pub(crate) fn overlapping_system_paths_are_diagnosed_before_assembly() {
    let modules = vec![module_with_runtimes("m", &["a", "b"])];
    let lib = Library::build(
        vec![
            pkg("a", "1"),
            pkg_yaml("id: b\nversion: 1\nkind: system\nprovides_paths: [opt/rt/a-1/lib]"),
        ],
        Vec::new(),
    );
    let diags = pkgs::diagnoses(&modules, &lib, &vm_spec(), &[]);
    let clash = diags
        .iter()
        .find(|d| matches!(d, Problem::Conflict { .. }))
        .unwrap_or_else(|| panic!("前后缀重叠必须复现冲突：{:?}", diags));
    let text = pkgs::diagnose_text(std::slice::from_ref(clash));
    assert!(text.contains("都要写进 opt/rt/a-1"), "{}", text);
    assert!(text.contains("a@1") && text.contains("b@1"), "{}", text);
}
