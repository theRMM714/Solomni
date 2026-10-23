//! 目的：packages 能力的用例测试——包契约、包库、目标筛选、选包计划与诊断。
//! 管：清单校验、版本/去重、冲突预检、平台分包、计划顺序与诊断说法。
//! 不管：guest 挂载与叠写（`src/tests/vm.rs`）、构建编排（`src/tests/diagnostics.rs`）。
//! 联动：契约见 RUNTIME_SPEC.md；被测实现见 `src/capabilities/packages/`。

use super::builders::*;
use super::prelude::*;
use crate::capabilities::packages::api as pkgs;
use crate::capabilities::packages::api::{Facts, Library, PinPolicy, Problem, Target};
use crate::capabilities::packages::detail::FsPackages;
use crate::capabilities::packages::domain::manifest::{
    check_manifest, conflicts, host_platform, is_platform_dir_name, PackageManifest,
};
use crate::capabilities::packages::ports::PackageSource;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

#[test]
pub(crate) fn package_manifest_check_rejects_illegal_forms() {
    assert!(
        check_manifest(&pkg("python", "3.12.4")).is_ok(),
        "prefix 类默认 kind"
    );
    assert!(
        check_manifest(&pkg_yaml("id: Python\nversion: 1\nprefix: opt/p")).is_err(),
        "id 只允许小写"
    );
    assert!(
        check_manifest(&pkg_yaml("id: py\nversion: 1\nkind: magic\nprefix: opt/p")).is_err(),
        "kind 只认 prefix / system"
    );
    assert!(
        check_manifest(&pkg_yaml("id: py\nversion: 1")).is_err(),
        "prefix 类必须给 prefix"
    );
    assert!(
        check_manifest(&pkg_yaml("id: py\nversion: 1\nprefix: opt/../etc")).is_err(),
        "前缀不能含 .."
    );
    assert!(
        check_manifest(&pkg_yaml("id: py\nversion: 1\nprefix: opt\\rt")).is_err(),
        "前缀用 / 书写形式"
    );
    assert!(
        check_manifest(&pkg_yaml("id: cc\nversion: 1\nkind: system")).is_err(),
        "system 类必须给 provides_paths"
    );
    assert!(check_manifest(&pkg_yaml(
        "id: cc\nversion: 1\nkind: system\nprovides_paths: [usr/include]"
    ))
    .is_ok());
    assert!(
        check_manifest(&pkg_yaml("id: a\nversion: 1\nprefix: opt/a\nrequires: [a]")).is_err(),
        "requires 不能依赖自己"
    );
}

#[test]
pub(crate) fn library_keeps_versions_and_rejects_duplicates() {
    let lib = Library::build(
        vec![
            pkg("python", "3.12.4"),
            pkg("python", "3.11.9"),
            pkg("python", "3.12.4"),
            pkg_yaml("id: bad\nversion: 1"),
        ],
        vec!["x：package.yaml 非法".to_string()],
    );
    let versions: Vec<&str> = lib
        .versions_of("python")
        .iter()
        .map(|p| p.version.as_str())
        .collect();
    assert_eq!(
        versions,
        vec!["3.11.9", "3.12.4"],
        "同 (id, version) 只收一份，版本升序"
    );
    assert!(
        lib.rejected.iter().any(|r| r.contains("只收先出现的那份")),
        "{:?}",
        lib.rejected
    );
    assert!(
        lib.rejected.iter().any(|r| r.contains("prefix")),
        "非法清单要说明原因：{:?}",
        lib.rejected
    );
    assert!(
        lib.rejected.iter().any(|r| r.contains("package.yaml 非法")),
        "适配层拒收原因也要留：{:?}",
        lib.rejected
    );
    let caps = lib.capability_versions();
    assert_eq!(caps.get("python").map(|v| v.len()), Some(2));
}

#[test]
pub(crate) fn package_conflicts_flag_overlapping_paths() {
    let lib = Library::build(
        vec![
            pkg_yaml("id: a\nversion: 1\nkind: system\nprovides_paths: [usr/lib]"),
            pkg_yaml("id: b\nversion: 1\nkind: system\nprovides_paths: [usr/lib/x86_64]"),
            pkg("node", "20.11.1"),
        ],
        Vec::new(),
    );
    let refs: Vec<&PackageManifest> = lib.packages.iter().collect();
    let got = conflicts(&refs);
    assert_eq!(got.len(), 1, "只有那对写进同一处的包冲突：{:?}", got);
    assert_eq!(got[0].0, "usr/lib");
    assert!(
        got[0].1.contains("a@1") && got[0].2.contains("b@1"),
        "{:?}",
        got
    );
}

#[test]
pub(crate) fn fs_packages_records_source_dir() {
    let dir = crate::tests::scratch("packages-source");
    let sub = dir.join("py");
    std::fs::create_dir_all(&sub).expect("建包目录");
    std::fs::write(
        sub.join("package.yaml"),
        "id: python\nversion: 3.12.4\nprefix: opt/rt/py\n",
    )
    .expect("写清单");
    let lib = FsPackages::new(dir.clone()).scan();
    assert_eq!(lib.packages.len(), 1, "一个包");
    assert_eq!(lib.packages[0].source_dir, sub, "扫描要记下来源目录");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 目的：运行包按 guest 平台分包——平台子目录名的认法、按 guest 筛库、平台名拼法。
#[test]
pub(crate) fn packages_split_by_guest_platform() {
    assert!(is_platform_dir_name("linux-x86_64"));
    assert!(is_platform_dir_name("windows-aarch64"));
    assert!(!is_platform_dir_name("linux"), "要 os-arch 两段");
    assert!(
        !is_platform_dir_name("linux-riscv64"),
        "不认的架构不算平台目录"
    );
    assert!(!is_platform_dir_name("bin"), "普通内容目录不算平台目录");

    let mk = |id: &str, src: &str, dirs: Vec<(&str, &str)>| PackageManifest {
        id: id.to_string(),
        version: "1".to_string(),
        kind: "prefix".to_string(),
        prefix: format!("opt/rt/{}", id),
        provides_paths: Vec::new(),
        requires: Vec::new(),
        license: Vec::new(),
        note: String::new(),
        files: Vec::new(),
        source_dir: std::path::PathBuf::from(src),
        platform_dirs: dirs
            .into_iter()
            .map(|(k, v)| (k.to_string(), std::path::PathBuf::from(v)))
            .collect(),
    };
    let lib = Library {
        packages: vec![
            mk(
                "python",
                "/rt/python",
                vec![
                    ("linux-x86_64", "/rt/python/linux-x86_64"),
                    ("windows-x86_64", "/rt/python/windows-x86_64"),
                ],
            ),
            mk("plain", "/rt/plain", Vec::new()),
        ],
        rejected: Vec::new(),
    };
    let g = pkgs::library_for_guest(&lib, "linux-x86_64");
    assert_eq!(
        g.packages.len(),
        2,
        "匹配的与不分平台的都留：{:?}",
        g.packages
    );
    assert_eq!(
        g.packages
            .iter()
            .find(|p| p.id == "python")
            .expect("python 在")
            .source_dir,
        std::path::PathBuf::from("/rt/python/linux-x86_64"),
        "source_dir 指到匹配的平台子目录"
    );
    let w = pkgs::library_for_guest(&lib, "windows-aarch64");
    assert!(
        w.packages.iter().all(|p| p.id != "python"),
        "没有匹配分包 → 这个包在 guest 上不可用（诊断会如实报缺）"
    );
    assert!(
        w.packages.iter().any(|p| p.id == "plain"),
        "不分平台的包照旧"
    );

    assert_eq!(
        crate::capabilities::vm::api::guest_platform(
            crate::capabilities::vm::domain::guest::GuestOs::Linux,
            crate::capabilities::vm::domain::guest::GuestArch::Arm64
        ),
        "linux-aarch64",
        "平台名是 os-arch，架构用 x86_64 / aarch64"
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
    assert!(pkgs::plan_guest(&spec, &modules, &empty, &guest)
        .unusable
        .is_empty());
    assert!(pkgs::plan_guest(&spec, &modules, &empty, &guest)
        .packages
        .is_empty());
    // 多版本且未定版 = 不替用户选
    let two = Library::build(
        vec![pkg("python", "3.12.4"), pkg("python", "3.11.9")],
        Vec::new(),
    );
    assert!(pkgs::diagnoses(&modules, &two, &spec, &[])
        .iter()
        .any(|d| matches!(d, Problem::Ambiguous { .. })));
    // 定版指定的版本不在库里 = 如实报
    let bad = ExecSpec {
        pins: BTreeMap::from([("python".to_string(), "9.9".to_string())]),
        ..vm_spec()
    };
    assert!(pkgs::diagnoses(&modules, &two, &bad, &[])
        .iter()
        .any(|d| matches!(d, Problem::UnknownPin { .. })));
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
            pkg_yaml("id: cc\nversion: 13.2.0\nkind: system\nprovides_paths: [usr/bin, usr/include]\nrequires: [binutils]"),
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

/// 目的：按当前宿主目标造一个包清单（平台子目录可指定；source_dir 由调用方给）。
fn manifest(
    id: &str,
    version: &str,
    kind: &str,
    prefix: &str,
    source_dir: &Path,
    dirs: &[(&str, &Path)],
) -> PackageManifest {
    PackageManifest {
        id: id.to_string(),
        version: version.to_string(),
        kind: kind.to_string(),
        prefix: prefix.to_string(),
        provides_paths: if kind == "system" {
            vec!["usr/lib".to_string()]
        } else {
            Vec::new()
        },
        requires: Vec::new(),
        license: Vec::new(),
        note: String::new(),
        files: Vec::new(),
        source_dir: source_dir.to_path_buf(),
        platform_dirs: dirs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_path_buf()))
            .collect(),
    }
}

/// 目的：目标平台载荷：host-* 优先、guest-* 优先、无前缀通用回退；不留错的那份。
#[test]
pub(crate) fn target_selects_the_matching_payload() {
    let Some(platform) = host_platform() else {
        return;
    };
    let root = PathBuf::from("/packages/py");
    let host_dir = root.join(format!("host-{}", platform));
    let guest_dir = root.join(format!("guest-{}", platform));
    let common = root.join(platform);
    let p = manifest(
        "py",
        "1",
        "prefix",
        "opt/rt/py",
        &root,
        &[
            (&format!("host-{}", platform), &host_dir),
            (&format!("guest-{}", platform), &guest_dir),
            (platform, &common),
        ],
    );
    assert_eq!(
        p.payload_for(&Target::host()).as_deref(),
        Some(host_dir.as_path()),
        "本机档优先 host-*"
    );
    assert_eq!(
        p.payload_for(&Target::guest(platform)).as_deref(),
        Some(guest_dir.as_path()),
        "虚拟机档优先 guest-*"
    );
    let only_guest = manifest(
        "py",
        "1",
        "prefix",
        "opt/rt/py",
        &root,
        &[(&format!("guest-{}", platform), &guest_dir)],
    );
    assert!(only_guest.payload_for(&Target::host()).is_none());
    let common_only = manifest("py", "1", "prefix", "opt/rt/py", &root, &[]);
    assert_eq!(
        common_only.payload_for(&Target::host()).as_deref(),
        Some(root.as_path())
    );
}

/// 目的：本机计划优先用运行包，包 bin/ 前置到 PATH、包根进只读根；host 缺少的可执行名回退宿主。
#[test]
pub(crate) fn host_plan_prefers_packages_and_falls_back_to_host_path() {
    let dir = crate::tests::scratch("host-plan");
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).expect("建 bin");
    let p = manifest("tool", "1", "prefix", "opt/rt/tool", &dir, &[]);
    let lib = Library::build(vec![p], Vec::new());
    let modules = vec![module_with_runtimes("a", &["tool"])];
    let spec = ExecSpec::default();
    let has_any = |_: &str| true;
    let bin_ok = |p: &Path| p == bin;
    let current = OsStr::new("/host/bin");
    let facts = Facts {
        has_exe: &has_any,
        bin_ok: &bin_ok,
        current_path: current,
    };
    let plan = pkgs::plan_host(&spec, &modules, &lib, PinPolicy::Require, &facts);
    assert!(plan.problems.is_empty(), "{:?}", plan.problems);
    assert_eq!(
        plan.providers.get("tool"),
        Some(&pkgs::Provider::Package {
            id: "tool".to_string(),
            version: "1".to_string()
        }),
        "有包就用包（即使宿主 PATH 上也有）"
    );
    let path = plan.path.expect("有包就有 PATH");
    let first = std::env::split_paths(&path).next().expect("第一段");
    assert_eq!(first, bin, "包 bin/ 前置");
    assert_eq!(plan.roots, vec![dir.clone()], "包根进只读根");

    let empty = Library::default();
    let plan = pkgs::plan_host(&spec, &modules, &empty, PinPolicy::Require, &facts);
    assert!(plan.problems.is_empty(), "{:?}", plan.problems);
    assert_eq!(plan.providers.get("tool"), Some(&pkgs::Provider::Host));
    assert!(plan.path.is_none(), "宿主 PATH 已够用，不需要覆盖");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 目的：多版本在会话里必须定版；无会话面按稳定序取第一个。
#[test]
pub(crate) fn host_plan_requires_a_pin_for_multiple_versions() {
    let p1 = manifest("tool", "1", "prefix", "opt/rt/tool1", Path::new("/p1"), &[]);
    let p2 = manifest("tool", "2", "prefix", "opt/rt/tool2", Path::new("/p2"), &[]);
    let lib = Library::build(vec![p1, p2], Vec::new());
    let modules = vec![module_with_runtimes("a", &["tool"])];
    let spec = ExecSpec::default();
    let has = |_: &str| false;
    let bin_ok = |_: &Path| true;
    let facts = Facts {
        has_exe: &has,
        bin_ok: &bin_ok,
        current_path: OsStr::new(""),
    };
    let plan = pkgs::plan_host(&spec, &modules, &lib, PinPolicy::Require, &facts);
    assert!(
        plan.problems
            .iter()
            .any(|p| matches!(p, Problem::Ambiguous { .. })),
        "{:?}",
        plan.problems
    );
    assert_eq!(plan.unusable.get("a"), Some(&vec!["tool".to_string()]));
    let plan = pkgs::plan_host(&spec, &modules, &lib, PinPolicy::First, &facts);
    assert!(plan.problems.is_empty(), "{:?}", plan.problems);
    assert_eq!(plan.packages[0].version, "1", "无会话面取稳定序第一个");
}

/// 目的：system 类包不服务本机档；包没有可执行 bin 时不假装可用。
#[test]
pub(crate) fn host_plan_refuses_system_and_binless_packages() {
    let system = manifest("cc", "13", "system", "", Path::new("/p/cc"), &[]);
    let lib = Library::build(vec![system], Vec::new());
    let modules = vec![module_with_runtimes("a", &["cc"])];
    let spec = ExecSpec::default();
    let has = |_: &str| false;
    let bin_ok = |_: &Path| true;
    let facts = Facts {
        has_exe: &has,
        bin_ok: &bin_ok,
        current_path: OsStr::new(""),
    };
    let plan = pkgs::plan_host(&spec, &modules, &lib, PinPolicy::Require, &facts);
    assert!(
        plan.problems
            .iter()
            .any(|p| matches!(p, Problem::NotForTarget { .. })),
        "{:?}",
        plan.problems
    );
    assert_eq!(plan.unusable.get("a"), Some(&vec!["cc".to_string()]));

    let no_bin = manifest(
        "tool",
        "1",
        "prefix",
        "opt/rt/tool",
        Path::new("/p/tool"),
        &[],
    );
    let lib = Library::build(vec![no_bin], Vec::new());
    let tool_modules = vec![module_with_runtimes("a", &["tool"])];
    let bin_ok = |_: &Path| false;
    let facts = Facts {
        has_exe: &has,
        bin_ok: &bin_ok,
        current_path: OsStr::new(""),
    };
    let plan = pkgs::plan_host(&spec, &tool_modules, &lib, PinPolicy::Require, &facts);
    assert!(
        plan.problems
            .iter()
            .any(|p| matches!(p, Problem::NoBin { .. })),
        "{:?}",
        plan.problems
    );
}
