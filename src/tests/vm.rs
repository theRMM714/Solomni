//! 目的：虚拟机档能力的用例测试——运行包库、执行档位与计划、就绪检测与诊断。
//! 管：本能力的不变式（清单校验、计划顺序、承载判定、诊断说法）。
//! 不管：工作区与沙箱（`src/tests/workspace.rs`）、conductor 的编排。
//! 联动：契约见 docs/vm/README.md 与 RUNTIME_SPEC.md。
use super::builders::*;
use super::prelude::*;
use crate::capabilities::vm::api::Vm;
use crate::capabilities::vm::domain::guest::{
    classify_base, GuestImage, GuestMount, GuestRoot, GuestRun, GuestSpec,
};
use crate::capabilities::vm::ports::{PackageSource, ShareBackend, VmEngine};
use crate::kernel::ports::HostProbe;

/// 目的：虚拟根名 → 那一格目录的解析——名字是**一段**，路径与越界写法如实拒绝（不猜、不静默当路径）。
#[test]
pub(crate) fn guest_names_resolve_to_one_slot_and_paths_are_refused() {
    use crate::capabilities::vm::api::resolve_guest;
    let root = crate::tests::root_dir();
    let got = resolve_guest(&root, "debian").expect("清单里的名字要认得");
    assert_eq!(got, root.join(".vm").join("guest").join("debian"));
    assert!(resolve_guest(&root, "  ").is_err(), "空名要拒");
    assert!(resolve_guest(&root, "a/b").is_err(), "路径分隔符要拒");
    assert!(resolve_guest(&root, "a\\b").is_err(), "反斜杠也要拒");
    assert!(resolve_guest(&root, "..").is_err(), "上级目录要拒");
    assert!(resolve_guest(&root, ".").is_err(), "当前目录要拒");
    assert!(
        resolve_guest(&root, "/abs/path").is_err(),
        "绝对路径不是虚拟根名"
    );
}

#[test]
pub(crate) fn package_manifest_check_rejects_illegal_forms() {
    let check = crate::capabilities::vm::domain::packages::check_manifest;
    assert!(
        check(&pkg("python", "3.12.4")).is_ok(),
        "prefix 类默认 kind"
    );
    assert!(
        check(&pkg_yaml("id: Python\nversion: 1\nprefix: opt/p")).is_err(),
        "id 只允许小写"
    );
    assert!(
        check(&pkg_yaml("id: py\nversion: 1\nkind: magic\nprefix: opt/p")).is_err(),
        "kind 只认 prefix / system"
    );
    assert!(
        check(&pkg_yaml("id: py\nversion: 1")).is_err(),
        "prefix 类必须给 prefix"
    );
    assert!(
        check(&pkg_yaml("id: py\nversion: 1\nprefix: opt/../etc")).is_err(),
        "前缀不能含 .."
    );
    assert!(
        check(&pkg_yaml("id: py\nversion: 1\nprefix: opt\\\\rt")).is_err(),
        "前缀用 / 书写形式"
    );
    assert!(
        check(&pkg_yaml("id: cc\nversion: 1\nkind: system")).is_err(),
        "system 类必须给 provides_paths"
    );
    assert!(check(&pkg_yaml(
        "id: cc\nversion: 1\nkind: system\nprovides_paths: [usr/include]"
    ))
    .is_ok());
    assert!(
        check(&pkg_yaml("id: a\nversion: 1\nprefix: opt/a\nrequires: [a]")).is_err(),
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
    let got = crate::capabilities::vm::domain::packages::conflicts(&refs);
    assert_eq!(got.len(), 1, "只有那对写进同一处的包冲突：{:?}", got);
    assert_eq!(got[0].0, "usr/lib");
    assert!(
        got[0].1.contains("a@1") && got[0].2.contains("b@1"),
        "{:?}",
        got
    );
}

/// 目的：钉住承载判定这一条事实——界面的"能不能选"与创建/编辑的拒绝走同一把尺子。
/// 约束：现在的判据是**逐项清单**：缺哪几项、每项怎么补，都要能读出来。
#[test]
pub(crate) fn vm_tier_readiness_gates_creation_and_editing() {
    // 本机档：任何机器上都能承载（不装载运行包、不要 guest）。
    let host = exec::tier_readiness(
        &crate::tests::root_dir(),
        &ExecSpec::default(),
        None,
        None,
        &crate::kernel::detail::HostProbeAdapter,
    );
    assert!(host.ready(), "本机档没有前置条件");
    assert!(host.requirements.is_empty(), "本机档不该有虚拟机前置清单");
    assert!(exec::tier_refusal(
        &crate::tests::root_dir(),
        &ExecSpec::default(),
        None,
        None,
        &crate::kernel::detail::HostProbeAdapter
    )
    .is_none());

    // 虚拟机档：guest 本体按**已验证平台**放开（Linux 已验，Windows / macOS 待验），其余项按事实；这里基础根是假的。
    let ghost = ExecSpec {
        tier: Tier::Vm,
        guest: Some("definitely-not-a-real-base-root".to_string()),
        ..Default::default()
    };
    let r = exec::tier_readiness(
        &crate::tests::root_dir(),
        &ghost,
        None,
        None,
        &crate::kernel::detail::HostProbeAdapter,
    );
    assert!(!r.ready(), "前置不齐就不成立：{:?}", r);
    let guest_item = r
        .requirements
        .iter()
        .find(|x| x.id == "guest")
        .expect("清单里要有 guest 这一项");
    assert_eq!(
        guest_item.met,
        crate::capabilities::vm::domain::exec::host_verified(std::env::consts::OS),
        "guest 项按已验平台数据表放开：{:?}",
        guest_item
    );
    let combo = {
        let c = crate::capabilities::vm::domain::exec::default_combos(std::env::consts::OS)
            .iter()
            .find(|c| {
                c.engine == "qemu"
                    && crate::capabilities::vm::domain::exec::share_available(
                        c.share,
                        None,
                        &crate::kernel::detail::HostProbeAdapter,
                    )
            })
            .expect("本平台要有可用的组合");
        format!("{}/{}", c.engine, c.share)
    };
    assert!(
        guest_item.detail.contains(&combo),
        "就绪说明要带上本平台默认组合 {}：{:?}",
        combo,
        guest_item
    );
    let unmet: Vec<&str> = r.unmet().iter().map(|x| x.id).collect();
    assert!(
        unmet.contains(&"base"),
        "填错的基础根要如实列出：{:?}",
        unmet
    );
    for item in r.unmet() {
        assert!(
            !item.how.is_empty(),
            "每一项没满足都要给出怎么补：{:?}",
            item
        );
        assert!(!item.detail.is_empty(), "每一项都要有现状描述：{:?}", item);
    }
    let why = exec::tier_refusal(
        &crate::tests::root_dir(),
        &ghost,
        None,
        None,
        &crate::kernel::detail::HostProbeAdapter,
    )
    .expect("不成立就要给可读理由");
    assert!(why.contains("虚拟机档现在不可用"), "{}", why);

    // 虚拟根在场：base 这一项要认出来；再故意登记一个不存在的 QEMU 路径，验证仍不成立（不随机器变）。
    let name = crate::tests::test_guest("tier-readiness");
    let real = ExecSpec {
        tier: Tier::Vm,
        guest: Some(name),
        ..Default::default()
    };
    let r2 = exec::tier_readiness(
        &crate::tests::root_dir(),
        &real,
        Some("definitely-not-qemu.exe"),
        None,
        &crate::kernel::detail::HostProbeAdapter,
    );
    let base_item = r2
        .requirements
        .iter()
        .find(|x| x.id == "base")
        .expect("清单里要有基础根这一项");
    assert!(base_item.met, "在场的虚拟根要认出来：{:?}", base_item);
    assert!(!r2.ready(), "缺 QEMU 一项就不成立");
}

/// 目的：QEMU 检测——登记了就用登记的路径，没登记就看 PATH；依赖不预置、不分发。
#[test]
pub(crate) fn vm_requirements_report_qemu_registration() {
    let base = crate::tests::scratch("vm-req-qemu");
    let base_str = base.to_string_lossy().into_owned();
    let spec = ExecSpec {
        tier: Tier::Vm,
        guest: Some(base_str.clone()),
        ..Default::default()
    };

    // 登记了一个不存在的路径：必须报"找不到"，并给出怎么补。
    let r = exec::tier_readiness(
        &crate::tests::root_dir(),
        &spec,
        Some("definitely-not-qemu.exe"),
        None,
        &crate::kernel::detail::HostProbeAdapter,
    );
    let qemu = r
        .requirements
        .iter()
        .find(|x| x.id == "qemu")
        .expect("清单里要有 QEMU 这一项");
    assert!(!qemu.met, "不存在的路径不算找到：{:?}", qemu);
    assert!(!qemu.how.is_empty(), "没找到就要给怎么补：{:?}", qemu);

    // 登记一个真实存在的文件：要认出来（QEMU 是不是真的不重要——这里只钉"登记生效"）。
    let fake = base.join("qemu-system-x86_64");
    std::fs::write(&fake, b"stub").unwrap();
    let r2 = exec::tier_readiness(
        &crate::tests::root_dir(),
        &spec,
        Some(fake.to_string_lossy().as_ref()),
        None,
        &crate::kernel::detail::HostProbeAdapter,
    );
    let qemu2 = r2
        .requirements
        .iter()
        .find(|x| x.id == "qemu")
        .expect("清单里要有 QEMU 这一项");
    assert!(qemu2.met, "登记的路径在场就要认出来：{:?}", qemu2);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
pub(crate) fn missing_entries_flags_only_relative_command_entries() {
    let with_tool = |module: &str, tool: &str, command: &str| {
        let mut m = module_of(module);
        m.manifest.tools.insert(
            tool.to_string(),
            crate::capabilities::modules::api::ToolDecl {
                command: command.to_string(),
                desc: String::new(),
                params: None,
                parallel: false,
            },
        );
        m
    };
    let modules = vec![
        with_tool("a", "script", "python tools/scan.py"),
        with_tool("a", "bare", "indexer build"),
        with_tool("a", "artifact", "build/indexer build"),
        with_tool("a", "absolute", "/usr/bin/x build"),
    ];
    // 宿主事实：只有 a 的 build/indexer 不在场（其余入口判据都不看）。
    let gone = abs(&["a", "build", "indexer"]);
    let missing = exec::missing_entries(&modules, &|p| p != gone.as_path());
    let got: Vec<(&str, &str, &str)> = missing
        .iter()
        .map(|m| (m.module.as_str(), m.tool.as_str(), m.command.as_str()))
        .collect();
    assert_eq!(got, vec![("a", "artifact", "build/indexer")]);
}

#[test]
pub(crate) fn exec_entry_missing_refuses_before_any_tier() {
    let modules = vec![module_with_runtimes("a", &["python"])];
    let missing = vec![crate::capabilities::vm::api::MissingEntry {
        module: "a".to_string(),
        tool: "build".to_string(),
        command: "build/indexer".to_string(),
        buildable: true,
    }];
    // 本机档：入口判据是唯一要说的。
    let host = exec::plan(
        &ExecSpec::default(),
        &modules,
        &Library::default(),
        &[],
        &missing,
    )
    .unwrap_err();
    assert!(
        matches!(host.as_slice(), [Diagnosis::MissingEntry { .. }]),
        "{:?}",
        host
    );
    // 虚拟机档：入口判据排最前，缺包一并如实报（一次把话说全）。
    let vm = exec::plan(&vm_spec(), &modules, &Library::default(), &[], &missing).unwrap_err();
    assert!(
        matches!(vm.first(), Some(Diagnosis::MissingEntry { .. })),
        "入口判据要排在最前：{:?}",
        vm
    );
    let said = exec::diagnose_text(&vm);
    assert!(
        said.contains("build/indexer") && said.contains("--build=a"),
        "模块声明了构建方法时，拒绝理由要指到 --build：{}",
        said
    );
    // 没有这份事实时照旧成立：入口判据不牵连别的东西。
    exec::plan(
        &ExecSpec::default(),
        &modules,
        &Library::default(),
        &[],
        &[],
    )
    .expect("本机档照旧可成立");
}
#[test]
pub(crate) fn package_setup_scripts_apply_prefix_bins_and_system_payloads() {
    use crate::capabilities::vm::domain::exec::{ExecPlan, PlannedPackage};
    let pkg = |id: &str, kind: &str, prefix: &str, rels: &[&str]| PlannedPackage {
        id: id.to_string(),
        version: "1".to_string(),
        kind: kind.to_string(),
        prefix: prefix.to_string(),
        source_dir: std::path::PathBuf::from(format!("/pkgs/{}", id)),
        provides_paths: rels.iter().map(|s| s.to_string()).collect(),
    };
    let plan = ExecPlan {
        tier: Tier::Vm,
        net: false,
        guest: Some("debian".to_string()),
        packages: vec![
            pkg("cc", "prefix", "opt/cc", &[]),
            pkg("tools", "system", "", &["usr/bin", "usr/share/tools"]),
            pkg("empty", "system", "", &[]),
        ],
    };
    let scripts = crate::capabilities::vm::api::package_setup_scripts(&plan);
    assert_eq!(
        scripts.len(),
        2,
        "prefix 一条 + system 一条；没有 provides_paths 的不生成：{:?}",
        scripts
    );
    // prefix 包：把 <prefix>/bin 软链进 PATH。
    assert!(
        scripts[0].contains("/opt/cc/bin") && scripts[0].contains("/usr/local/bin"),
        "{}",
        scripts[0]
    );
    // system 包：从只读载荷根按 provides_paths 叠写，并落"已叠"标记。
    let sys = &scripts[1];
    assert!(sys.contains("solomni-packages/tools"), "{}", sys);
    assert!(sys.contains("/usr/bin"), "{}", sys);
    assert!(sys.contains("usr/share/tools"), "{}", sys);
    assert!(
        sys.contains("/var/lib/solomni/packages/tools@1"),
        "标记：{}",
        sys
    );
    // 生成的脚本必须是合法 shell（引号与拼接错了在这里就露）。
    #[cfg(unix)]
    {
        use std::io::Write;
        let path = crate::tests::scratch("pkg-setup-script").join("apply.sh");
        let mut f = std::fs::File::create(&path).expect("建脚本");
        f.write_all(scripts.join("\n").as_bytes()).expect("写脚本");
        let st = std::process::Command::new("sh")
            .arg("-n")
            .arg(&path)
            .status()
            .expect("跑 sh -n");
        assert!(st.success(), "生成的装配脚本要能过 sh -n：{:?}", scripts);
    }
}
#[test]
pub(crate) fn exec_host_tier_ignores_packages() {
    let modules = vec![module_with_runtimes("a", &["python"])];
    let plan = exec::plan(
        &ExecSpec::default(),
        &modules,
        &Library::default(),
        &[],
        &[],
    )
    .expect("本机档不装载运行包，不会因缺包失败");
    assert_eq!(plan.tier, Tier::Host);
    assert!(plan.packages.is_empty());
    assert!(plan.guest.is_none());
    assert!(!plan.net, "默认不放行出站网络");
    let summary = exec::plan_summary(&plan);
    assert!(
        summary.contains("本机") && summary.contains("不放行"),
        "{}",
        summary
    );
}

#[test]
pub(crate) fn exec_vm_tier_reports_missing_ambiguous_and_unavailable() {
    let modules = vec![module_with_runtimes("a", &["python"])];
    let empty = Library::default();
    let spec = vm_spec();
    assert_eq!(
        exec::vm_diagnoses(&modules, &empty, &spec, &[]),
        vec![Diagnosis::Missing {
            module: "a".to_string(),
            capability: "python".to_string()
        }]
    );
    let un = exec::unavailable(&spec, &modules, &empty, &[]);
    assert_eq!(
        un.get("a"),
        Some(&vec!["python".to_string()]),
        "虚拟机档缺包 = 该模块工具不可用"
    );
    assert!(
        exec::unavailable(&ExecSpec::default(), &modules, &empty, &[]).is_empty(),
        "本机档一律可用"
    );
    // 缺包 = **不创建**（不再降级）：没有工具用户完不成工作，与其拉一台跑不了工具的 guest，不如选型这步如实拒。
    let refused = exec::plan(&spec, &modules, &empty, &[], &[]).unwrap_err();
    assert!(
        matches!(refused.as_slice(), [Diagnosis::Missing { .. }]),
        "{:?}",
        refused
    );
    let said = exec::diagnose_text(&exec::vm_diagnoses(&modules, &empty, &spec, &[]));
    assert!(
        said.contains("runtimes/"),
        "缺包的说法要告诉用户把包放哪：{}",
        said
    );
    // guest 镜像自带的能力不算缺（镜像里已经有解释器）：诊断空、工具可用、计划成立。
    let guest = vec!["python".to_string()];
    assert!(exec::vm_diagnoses(&modules, &empty, &spec, &guest).is_empty());
    assert!(exec::unavailable(&spec, &modules, &empty, &guest).is_empty());
    assert!(exec::plan(&spec, &modules, &empty, &guest, &[])
        .expect("guest 自带就不算缺")
        .packages
        .is_empty());
    // 多版本且未定版 = 不替用户选
    let two = Library::build(
        vec![pkg("python", "3.12.4"), pkg("python", "3.11.9")],
        Vec::new(),
    );
    assert!(exec::vm_diagnoses(&modules, &two, &spec, &[])
        .iter()
        .any(|d| matches!(d, Diagnosis::Ambiguous { .. })));
    // 定版指定的版本不在库里 = 如实报
    let bad = ExecSpec {
        pins: BTreeMap::from([("python".to_string(), "9.9".to_string())]),
        ..vm_spec()
    };
    assert!(exec::vm_diagnoses(&modules, &two, &bad, &[])
        .iter()
        .any(|d| matches!(d, Diagnosis::UnknownPin { .. })));
    // 多版本未定版 = 选型不成立：派生计划如实拒绝（这是用户要解决的选型问题，不是"缺包"）
    let refused = exec::plan(&spec, &modules, &two, &[], &[]).unwrap_err();
    assert!(
        exec::diagnose_text(&refused).contains("多个版本"),
        "{}",
        exec::diagnose_text(&refused)
    );
    // 定版之后可以成立
    let ok = ExecSpec {
        pins: BTreeMap::from([("python".to_string(), "3.12.4".to_string())]),
        ..vm_spec()
    };
    assert!(exec::vm_diagnoses(&modules, &two, &ok, &[]).is_empty());
    assert_eq!(
        exec::plan(&ok, &modules, &two, &[], &[])
            .unwrap()
            .packages
            .len(),
        1
    );
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
    let plan = exec::plan(&vm_spec(), &modules, &lib, &[], &[]).expect("虚拟机档可成立");
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
    assert_eq!(plan.guest.as_deref(), Some("vm-spec"));
    assert!(!plan.net, "默认不放行出站网络");
}

#[test]
pub(crate) fn diagnose_text_spells_out_every_reason() {
    let missing = exec::diagnose_text(&[Diagnosis::Missing {
        module: "a".to_string(),
        capability: "python".to_string(),
    }]);
    assert!(
        missing.contains("模块 a") && missing.contains("python") && missing.contains("runtimes/"),
        "{}",
        missing
    );
    let ambiguous = exec::diagnose_text(&[Diagnosis::Ambiguous {
        capability: "python".to_string(),
        versions: vec!["3.11.9".to_string(), "3.12.4".to_string()],
    }]);
    assert!(
        ambiguous.contains("多个版本") && ambiguous.contains("3.12.4"),
        "{}",
        ambiguous
    );
    let bad = exec::diagnose_text(&[Diagnosis::UnknownPin {
        capability: "python".to_string(),
        version: "9.9".to_string(),
    }]);
    assert!(bad.contains("定版 9.9"), "{}", bad);
    let clash = exec::diagnose_text(&[Diagnosis::Conflict {
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
pub(crate) fn fs_packages_records_source_dir() {
    let dir = crate::tests::scratch("vm-pkg-source");
    let sub = dir.join("py");
    std::fs::create_dir_all(&sub).expect("建包目录");
    std::fs::write(
        sub.join("package.yaml"),
        "id: python\nversion: 3.12.4\nprefix: opt/rt/py\n",
    )
    .expect("写清单");
    let lib = crate::capabilities::vm::detail::FsPackages::new(dir.clone()).scan();
    assert_eq!(lib.packages.len(), 1, "一个包");
    assert_eq!(lib.packages[0].source_dir, sub, "扫描要记下来源目录");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
pub(crate) fn guest_base_requires_rootfs_and_kernel() {
    let probe = crate::kernel::detail::HostProbeAdapter;
    let dir = crate::tests::scratch("vm-guest-base");
    // 空目录：缺内核。
    assert!(classify_base(&dir, &probe).is_err(), "没有内核不算基础根");
    // 放内核：可用，无 initrd。
    std::fs::write(dir.join("vmlinuz"), b"x").expect("写内核");
    let base = classify_base(&dir, &probe).expect("有内核就该可用");
    assert_eq!(base.kernel, Some(dir.join("vmlinuz")));
    assert!(base.initrd.is_none(), "没放 initrd 就是没有");
    // 放 initrd：一并认出来。
    std::fs::write(dir.join("initrd.img"), b"x").expect("写 initrd");
    let base = classify_base(&dir, &probe).expect("有 initrd 仍可用");
    assert_eq!(base.initrd, Some(dir.join("initrd.img")));
    // 不在场：不是一个目录。
    assert!(
        classify_base(&dir.join("nope"), &probe).is_err(),
        "不在场要报出来"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 目的：运行包按 guest 平台分包——平台子目录名的认法、按 guest 筛库、平台名拼法。
#[test]
pub(crate) fn packages_split_by_guest_platform() {
    use crate::capabilities::vm::domain::packages::{
        is_platform_dir_name, library_for_guest, Library, PackageManifest,
    };
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
    let g = library_for_guest(&lib, "linux-x86_64");
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
    let w = library_for_guest(&lib, "windows-aarch64");
    assert!(
        w.packages.iter().all(|p| p.id != "python"),
        "没有匹配分包 → 这个包在 guest 上不可用（诊断会如实报缺）"
    );
    assert!(
        w.packages.iter().any(|p| p.id == "plain"),
        "不分平台的包照旧"
    );

    assert_eq!(
        crate::capabilities::vm::domain::exec::guest_platform(
            crate::capabilities::vm::domain::guest::GuestOs::Linux,
            crate::capabilities::vm::domain::guest::GuestArch::Arm64
        ),
        "linux-aarch64",
        "平台名是 os-arch，架构用 x86_64 / aarch64"
    );
}

/// 目的：命令装配按 guest 方言译路径——工作目录与本机路径型环境变量改成 guest 拼法，命令与非路径变量不动。
#[test]
pub(crate) fn translate_run_maps_paths_to_the_guest_dialect() {
    use crate::capabilities::vm::domain::guest::{translate_run, GuestMount, GuestOs, GuestRun};
    // 挂载点用**宿主自己的写法**（宿主路径只能由宿主解析），guest 路径由我们给——断言因此三平台一致。
    let (rt, work) = if cfg!(windows) {
        ("D:\\rt\\py", "D:\\work")
    } else {
        ("/rt/py", "/work")
    };
    let mounts = vec![
        GuestMount {
            host: std::path::PathBuf::from(rt),
            guest: "/opt/rt/py".to_string(),
            read_only: true,
        },
        GuestMount {
            host: std::path::PathBuf::from(work),
            guest: "/guest/work".to_string(),
            read_only: false,
        },
    ];
    let run = GuestRun {
        command: "read_txt".to_string(),
        args_json: "{}".to_string(),
        env: vec![
            ("PATH".to_string(), format!("{}/bin", rt)),
            ("LANG".to_string(), "C.UTF-8".to_string()),
        ],
        cwd: format!("{}/mod", work),
    };
    let out = translate_run(&run, GuestOs::Linux, &mounts);
    assert_eq!(
        out.cwd, "/guest/work/mod",
        "工作目录按挂载表译成 guest 拼法"
    );
    assert_eq!(
        out.env[0].1, "/opt/rt/py/bin",
        "包目录型环境变量按**挂载点**译（不是镜像根）"
    );
    assert_eq!(out.env[1].1, "C.UTF-8", "非路径变量不动");
    assert_eq!(out.command, run.command, "命令不动");
    assert_eq!(
        out.args_json, run.args_json,
        "参数不动（模型给的就是 guest 拼法）"
    );

    // 挂载表优先于方言：guest 换成 Windows，命中的还是同一个挂载点。
    let same = translate_run(&run, GuestOs::Windows, &mounts);
    assert_eq!(same.cwd, "/guest/work/mod", "挂载表优先");

    // 不在任何挂载里的宿主路径：同方言原样，跨方言才走镜像根。
    let loose = translate_run(
        &GuestRun {
            command: "x".to_string(),
            args_json: "{}".to_string(),
            env: Vec::new(),
            cwd: format!("{}-outside", work),
        },
        GuestOs::Linux,
        &mounts,
    );
    if cfg!(windows) {
        assert_eq!(
            loose.cwd, "/solomni-host/d/work-outside",
            "不在挂载里就走镜像根"
        );
    } else {
        assert_eq!(loose.cwd, format!("{}-outside", work), "同方言：原样");
    }
}

/// 目的：路径桥同方言恒等；跨方言把宿主路径编进 guest 镜像根（非法字符转义）。
#[test]
pub(crate) fn path_bridge_maps_host_paths_into_the_guest() {
    use crate::capabilities::vm::domain::paths::{PathBridge, PathDialect};
    let id_posix = PathBridge::new("linux", PathDialect::Posix);
    assert_eq!(
        id_posix.to_guest(std::path::Path::new("/work/x")),
        "/work/x",
        "Linux 宿主 + POSIX guest 是同方言：原样"
    );
    let id_win = PathBridge::new("windows", PathDialect::Windows);
    assert_eq!(
        id_win.to_guest(std::path::Path::new("D:\\work")),
        "D:\\work",
        "Windows 宿主 + Windows guest 是同方言：原样"
    );

    // POSIX 宿主 → Windows guest：镜像根 + 非法字符转义（与宿主是哪个平台无关）。
    let w = PathBridge::new("linux", PathDialect::Windows);
    assert_eq!(
        w.to_guest(std::path::Path::new("/home/u/a:b")),
        "C:\\solomni-host\\home\\u\\a%3Ab",
        "Windows guest 走 Windows 镜像根并把 : 转义"
    );

    // Windows 宿主 → POSIX guest：宿主路径由**宿主自己**解析，所以这条只在 Windows 上判
    //（POSIX 上 "D:\\x" 是一个普通组件，拿它当 Windows 路径断言是错的）。
    #[cfg(windows)]
    {
        let b = PathBridge::new("windows", PathDialect::Posix);
        assert_eq!(
            b.to_guest(std::path::Path::new("D:\\AIproject\\Solomni\\work")),
            "/solomni-host/d/AIproject/Solomni/work",
            "Windows 宿主编进 POSIX 镜像根"
        );
    }
}

/// 目的：围栏根经桥落到 guest——跨方言时是镜像根，不再是宿主原路径。
#[test]
pub(crate) fn fence_roots_follow_the_bridge() {
    use crate::capabilities::vm::domain::exec::ExecPlan;
    use crate::capabilities::vm::domain::guest::mounts_from;
    use crate::capabilities::vm::domain::paths::{PathBridge, PathDialect};
    let plan = ExecPlan {
        tier: Tier::Vm,
        net: false,
        guest: None,
        packages: Vec::new(),
    };
    // 跨方言那一侧按宿主取（宿主路径只能由宿主自己解析）：断言的都是"围栏根按桥落到 guest 镜像根"。
    let (mods, work, bridge, want_mods, want_work) = if cfg!(windows) {
        (
            "D:\\mods\\a",
            "D:\\work\\x",
            PathBridge::new("windows", PathDialect::Posix),
            "/solomni-host/d/mods/a",
            "/solomni-host/d/work/x",
        )
    } else {
        (
            "/mods/a",
            "/work/x",
            PathBridge::new("linux", PathDialect::Windows),
            "C:\\solomni-host\\mods\\a",
            "C:\\solomni-host\\work\\x",
        )
    };
    let fence = crate::kernel::api::FenceSpec::standalone(
        std::path::Path::new(mods),
        Some(std::path::Path::new(work)),
        false,
    );
    let mounts = mounts_from(&plan, &fence, &bridge);
    assert!(
        mounts.iter().any(|m| m.guest == want_mods && m.read_only),
        "只读模块目录按镜像根挂：{:?}",
        mounts
    );
    assert!(
        mounts.iter().any(|m| m.guest == want_work && !m.read_only),
        "可写根按镜像根挂：{:?}",
        mounts
    );
}

#[test]
pub(crate) fn guest_mounts_prefix_packages_and_fence_roots() {
    use crate::capabilities::vm::domain::exec::{ExecPlan, PlannedPackage};
    use crate::capabilities::vm::domain::guest::mounts_from;
    let plan = ExecPlan {
        tier: Tier::Vm,
        net: false,
        guest: None,
        packages: vec![PlannedPackage {
            id: "python".to_string(),
            version: "3.12.4".to_string(),
            kind: "prefix".to_string(),
            prefix: "opt/rt/py".to_string(),
            source_dir: std::path::PathBuf::from("/rt/py"),
            provides_paths: Vec::new(),
        }],
    };
    let fence = crate::kernel::api::FenceSpec::standalone(
        std::path::Path::new("/mods/a"),
        Some(std::path::Path::new("/work/x")),
        false,
    );
    // 恒等桥（宿主与 guest 同方言）：这条用例判的是"prefix 与围栏根各自挂到哪"，不是跨方言换算。
    let bridge = crate::capabilities::vm::domain::paths::PathBridge::new(
        "linux",
        crate::capabilities::vm::domain::paths::PathDialect::Posix,
    );
    let mounts = mounts_from(&plan, &fence, &bridge);
    assert!(
        mounts
            .iter()
            .any(|m| m.guest == "/opt/rt/py" && m.read_only),
        "prefix 包只读挂到 /<prefix>：{:?}",
        mounts
    );
    assert!(
        mounts.iter().any(|m| m.guest == "/work/x" && !m.read_only),
        "可写根按宿主原路径挂入：{:?}",
        mounts
    );
    assert!(
        mounts.iter().any(|m| m.guest == "/mods/a" && m.read_only),
        "模块目录只读挂入：{:?}",
        mounts
    );
    let share = crate::capabilities::vm::detail::NinePShare::new()
        .prepare("qemu", &mounts, None)
        .expect("9p 计划要能派生")
        .0;
    let spec = GuestSpec {
        lease: "w".to_string(),
        image: GuestImage::linux(
            "test",
            GuestRoot::Dir(std::path::PathBuf::from("/base")),
            Some(std::path::PathBuf::from("/base/vmlinuz")),
            Some(std::path::PathBuf::from("/base/initrd.img")),
        ),
        mounts,
        net: false,
        engine_path: None,
        share_path: None,
        seed: None,
        isolated: false,
    };
    let args = crate::capabilities::vm::detail::qemu::engine_args(
        &spec,
        "host=127.0.0.1,port=1,server=on,wait=off",
        false,
        false,
        None,
        &[],
    );
    assert!(args.contains(&"none".to_string()), "不给网卡");
    assert!(
        !args.iter().any(|a| a.contains("netdev")),
        "没有网卡就没有 netdev"
    );
    assert!(
        args.iter().any(|a| a.contains("root=base")),
        "rootfs 走 9p 的 base 标签"
    );
    assert!(
        args.iter()
            .any(|a| a.starts_with("local,id=fsbase,") && !a.contains("readonly=on")),
        "基础根共享要可写（内核 root=... rw，额外挂载点也要建得出来）"
    );
    let accel = args
        .iter()
        .position(|a| a.as_str() == "-machine")
        .map(|i| args[i + 1].as_str());
    assert_eq!(
        accel,
        Some(crate::capabilities::vm::detail::qemu::accelerators()),
        "加速器按平台给（本平台硬件加速优先，TCG 兜底）"
    );
    let serial = args
        .iter()
        .position(|a| a.as_str() == "-serial")
        .map(|i| args[i + 1].as_str());
    assert_eq!(
        serial,
        Some("stdio"),
        "console=ttyS0 要有对应串口设备（-nodefaults 会去掉默认串口）"
    );
    assert!(
        args.iter().any(|a| a.contains("initrd.img")),
        "initrd 要带上"
    );

    // 共享后端：9p 把额外挂载翻成 -fsdev/-device 与 guest 端 mount 脚本。
    assert_eq!(share.kind, "9p");
    assert!(
        share
            .engine_args
            .iter()
            .any(|a| a.contains("virtio-9p-pci,fsdev=fs0,mount_tag=m0")),
        "额外挂载要接成 virtio-9p 设备：{:?}",
        share.engine_args
    );
    assert!(
        share
            .guest_scripts
            .iter()
            .any(|s| s.contains("mount -t 9p") && s.contains("m0")),
        "guest 端要给出 9p 挂载命令：{:?}",
        share.guest_scripts
    );
    assert!(
        share.guest_scripts.iter().any(|s| s.contains(",ro")),
        "只读挂载脚本要带 ro：{:?}",
        share.guest_scripts
    );
}

/// 目的：加速器表按平台给；网卡随共享后端需要而变——放行出站给普通网卡，共享隧道给受限网卡，都不要就不给。
#[test]
pub(crate) fn accelerators_and_nic_follow_the_platform_and_share_need() {
    use crate::capabilities::vm::detail::qemu::{accelerators, engine_args};
    let spec = |net: bool| GuestSpec {
        lease: "w".to_string(),
        image: GuestImage::linux(
            "test",
            GuestRoot::Dir(std::path::PathBuf::from("/base")),
            Some(std::path::PathBuf::from("/base/vmlinuz")),
            None,
        ),
        mounts: Vec::new(),
        net,
        engine_path: None,
        share_path: None,
        seed: None,
        isolated: false,
    };
    let expected = if cfg!(windows) {
        "accel=whpx:tcg"
    } else if cfg!(target_os = "macos") {
        "accel=hvf:tcg"
    } else if cfg!(target_os = "linux") {
        "accel=kvm:tcg"
    } else {
        "accel=tcg"
    };
    assert_eq!(accelerators(), expected, "加速器表按平台给");

    let plain = engine_args(&spec(false), "opts", false, false, None, &[]);
    assert!(
        plain.contains(&"none".to_string()) && !plain.iter().any(|a| a.contains("netdev")),
        "没有隧道需求就不给网卡：{:?}",
        plain
    );
    let tunnel = engine_args(&spec(false), "opts", true, false, None, &[]);
    assert!(
        tunnel.iter().any(|a| a == "user,id=n0,restrict=on"),
        "共享隧道要受限网卡（出站全断）：{:?}",
        tunnel
    );
    let online = engine_args(&spec(true), "opts", false, false, None, &[]);
    assert!(
        online.iter().any(|a| a == "user,id=n0"),
        "放行出站时给普通 user 网卡：{:?}",
        online
    );
    assert!(
        !online.iter().any(|a| a.contains("restrict=on")),
        "放行出站不设受限：{:?}",
        online
    );

    // vhost-user 设备（virtiofs）要 guest RAM 可共享：同尺寸 memfd 后端 + 唯一 numa 节点。
    let shared = engine_args(&spec(false), "opts", false, true, None, &[]);
    let want_mem = format!(
        "memory-backend-memfd,id=solomni-mem,size={}M,share=on",
        crate::capabilities::vm::detail::qemu::GUEST_MEM_MB
    );
    assert!(
        shared.iter().any(|a| a == &want_mem),
        "共享内存后端要与 -m 同尺寸：{:?}",
        shared
    );
    assert!(
        shared.iter().any(|a| a == "node,memdev=solomni-mem"),
        "内存后端要挂到 numa 节点：{:?}",
        shared
    );
    assert!(
        !engine_args(&spec(false), "opts", false, false, None, &[])
            .iter()
            .any(|a| a.contains("memory-backend-memfd")),
        "不需要共享内存时不加 memfd 后端"
    );
}

/// 目的：磁盘镜像根可以没有内核（靠镜像自带引导）；目录根没有内核则如实拒收。
#[test]
pub(crate) fn disk_base_may_boot_without_a_kernel() {
    let probe = crate::kernel::detail::HostProbeAdapter;
    let dir = crate::tests::scratch("vm-guest-nokernel");
    std::fs::write(dir.join("rootfs.ext4"), b"img").expect("写磁盘镜像");
    let image = classify_base(&dir, &probe).expect("磁盘根可以没有内核");
    assert!(image.kernel.is_none(), "没有 vmlinuz 就是 None");
    assert!(
        matches!(image.root, GuestRoot::Disk(_)),
        "同目录里的磁盘镜像仍判成磁盘根"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 目的：没有内核的磁盘根不传 -kernel / -append，交给镜像自带引导（固件启动）。
#[test]
pub(crate) fn disk_root_without_kernel_boots_via_firmware() {
    use crate::capabilities::vm::detail::qemu::engine_args;
    let spec = GuestSpec {
        lease: "w".to_string(),
        image: GuestImage::linux(
            "test",
            GuestRoot::Disk(std::path::PathBuf::from("/img/root.qcow2")),
            None,
            None,
        ),
        mounts: Vec::new(),
        net: false,
        engine_path: None,
        share_path: None,
        seed: None,
        isolated: false,
    };
    let args = engine_args(&spec, "opts", false, false, None, &[]);
    assert!(
        !args.iter().any(|a| a == "-kernel"),
        "没有内核就不传 -kernel：{:?}",
        args
    );
    assert!(
        !args.iter().any(|a| a == "-append"),
        "没有内核就不传 -append：{:?}",
        args
    );
    assert!(
        args.iter().any(|a| a == "file=/img/root.qcow2,if=virtio"),
        "磁盘根照给：{:?}",
        args
    );
}

/// 目的：磁盘镜像根走 virtio 磁盘（三平台通用），不再依赖 9p 的 base 标签。
#[test]
pub(crate) fn disk_root_uses_a_virtio_disk_not_9p() {
    use crate::capabilities::vm::detail::qemu::engine_args;
    let spec = GuestSpec {
        lease: "w".to_string(),
        image: GuestImage::linux(
            "test",
            GuestRoot::Disk(std::path::PathBuf::from("/img/root.qcow2")),
            Some(std::path::PathBuf::from("/img/vmlinuz")),
            None,
        ),
        mounts: Vec::new(),
        net: false,
        engine_path: None,
        share_path: None,
        seed: None,
        isolated: false,
    };
    let args = engine_args(&spec, "opts", false, false, None, &[]);
    assert!(
        args.iter().any(|a| a == "file=/img/root.qcow2,if=virtio"),
        "磁盘根要接成 virtio 磁盘：{:?}",
        args
    );
    assert!(
        args.iter().any(|a| a.contains("root=/dev/vda")),
        "内核命令行指向磁盘根：{:?}",
        args
    );
    assert!(
        !args.iter().any(|a| a.contains("fsbase")),
        "磁盘根不该再有 9p 基础根共享：{:?}",
        args
    );
}

/// 目的：同一租约里 guest 与共享凭据同生命周期；每次执行前后走共享的同步钩子。
#[test]
pub(crate) fn share_syncs_around_each_run_and_releases_on_stop() {
    let engine = Arc::new(crate::tests::doubles::FakeEngine::new());
    let share = Arc::new(crate::tests::doubles::FakeShare::new());
    let vm = crate::capabilities::vm::service::VmService::new(
        Arc::new(InMemoryPackages::empty()),
        engine.clone(),
        share.clone(),
    );
    let spec = GuestSpec {
        lease: "w".to_string(),
        image: GuestImage::linux(
            "test",
            GuestRoot::Dir(std::path::PathBuf::from("/base")),
            Some(std::path::PathBuf::from("/base/vmlinuz")),
            None,
        ),
        mounts: Vec::new(),
        net: false,
        engine_path: None,
        share_path: None,
        seed: None,
        isolated: false,
    };
    vm.guest_start(&spec).expect("起 guest");
    let run = GuestRun {
        command: "echo hi".to_string(),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: String::new(),
    };
    assert!(vm.guest_run("w", &run).ok, "执行要成功");
    vm.guest_stop("w");
    let calls = share.calls.lock().expect("锁").clone();
    assert_eq!(
        calls,
        vec!["prepare", "sync_in", "sync_out", "release"],
        "共享钩子顺序：装配 → 进去 → 回来 → 释放"
    );
    let ecalls = engine.calls.lock().expect("锁").clone();
    assert!(ecalls.iter().any(|c| c == "run:w:echo hi"), "{:?}", ecalls);
    assert!(ecalls.iter().any(|c| c == "stop:w"), "{:?}", ecalls);
}

/// 目的：复制后端不产生引擎参数，只在 guest 里建目录；宿主侧 tar 打包/解包真跑通。
#[test]
pub(crate) fn copy_share_plans_mkdirs_and_tar_round_trips_on_the_host() {
    use crate::capabilities::vm::detail::copy::{host_tar, host_untar, CopyShare};
    use crate::capabilities::vm::ports::ShareBackend;
    let share = CopyShare::new();
    let mounts = vec![GuestMount {
        host: std::path::PathBuf::from("/host/work"),
        guest: "/work".to_string(),
        read_only: false,
    }];
    let (plan, _guard) = share.prepare("qemu", &mounts, None).expect("复制计划");
    assert_eq!(plan.kind, "copy");
    assert!(
        plan.engine_args.is_empty(),
        "复制后端没有引擎参数：{:?}",
        plan.engine_args
    );
    assert_eq!(plan.guest_scripts, vec!["mkdir -p '/work'".to_string()]);
    assert!(!plan.needs_net, "复制后端不需要网卡");

    let src = crate::tests::scratch("copy-tar-src");
    let dst = crate::tests::scratch("copy-tar-dst");
    std::fs::write(src.join("a.txt"), b"hello-copy").expect("写文件");
    let tar = host_tar(&src).expect("打包");
    assert!(!tar.is_empty(), "tar 不该是空的");
    host_untar(&dst, &tar).expect("解包");
    assert_eq!(
        std::fs::read(dst.join("a.txt")).expect("读回来"),
        b"hello-copy".to_vec()
    );
    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(&dst);
}

/// 目的：目录里放着 rootfs 磁盘镜像时判成磁盘根（三平台通用），就绪项如实说明，引擎参数走 virtio 磁盘。
#[test]
pub(crate) fn disk_image_base_is_classified_as_a_disk_root() {
    let probe = crate::kernel::detail::HostProbeAdapter;
    // 虚拟根就装在项目内的那一格（就绪检测走真实探针，所以这里要造真目录）。
    let name = crate::tests::test_guest("disk-root");
    let dir = crate::tests::root_dir()
        .join(".vm")
        .join("guest")
        .join(&name);
    std::fs::write(dir.join("vmlinuz"), b"x").expect("写内核");
    std::fs::write(dir.join("rootfs.ext4"), b"img").expect("写磁盘镜像");
    let image = classify_base(&dir, &probe).expect("磁盘布局可用");
    assert!(
        matches!(image.root, GuestRoot::Disk(_)),
        "有磁盘镜像就判成磁盘根：{:?}",
        image.root
    );
    let spec = ExecSpec {
        tier: Tier::Vm,
        guest: Some(name.clone()),
        ..Default::default()
    };
    let req = exec::tier_readiness(&crate::tests::root_dir(), &spec, None, None, &probe);
    let base_item = req
        .requirements
        .iter()
        .find(|r| r.id == "base")
        .expect("清单里要有基础根这一项");
    assert!(
        base_item.met && base_item.detail.contains("磁盘镜像"),
        "就绪项要说明是磁盘根：{:?}",
        base_item
    );
    let args = crate::capabilities::vm::detail::qemu::engine_args(
        &GuestSpec {
            lease: "w".to_string(),
            image,
            mounts: Vec::new(),
            net: false,
            engine_path: None,
            share_path: None,
            seed: None,
            isolated: false,
        },
        "opts",
        false,
        false,
        None,
        &[],
    );
    assert!(
        args.iter()
            .any(|a| a.starts_with("file=") && a.ends_with(",if=virtio")),
        "磁盘根要接成 virtio 磁盘：{:?}",
        args
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 目的：装配前的两道如实拒绝——引擎承载不了这份 guest、共享后端落不到这台引擎/guest 上。
#[test]
pub(crate) fn vm_service_refuses_unfit_engine_and_share_with_a_reason() {
    use crate::capabilities::vm::detail::{NinePShare, QemuEngine};
    use crate::capabilities::vm::domain::guest::GuestArch;
    let svc = crate::capabilities::vm::service::VmService::new(
        Arc::new(InMemoryPackages::empty()),
        Arc::new(QemuEngine::new()),
        Arc::new(NinePShare::new()),
    );

    // 引擎承载不了：架构不匹配（磁盘根让 supports 的根类型判定通过，只剩架构这一条）。
    let mut wrong_arch = GuestImage::linux(
        "t",
        GuestRoot::Disk(std::path::PathBuf::from("/img/root.qcow2")),
        None,
        None,
    );
    wrong_arch.arch = match GuestArch::host() {
        GuestArch::Amd64 => GuestArch::Arm64,
        GuestArch::Arm64 => GuestArch::Amd64,
    };
    let spec = GuestSpec {
        lease: "w".to_string(),
        image: wrong_arch,
        mounts: Vec::new(),
        net: false,
        engine_path: None,
        share_path: None,
        seed: None,
        isolated: false,
    };
    let why = svc.guest_start(&spec).expect_err("架构不匹配要被拒绝");
    assert!(why.contains("承载不了"), "{}", why);

    // 共享落不上：引擎能承载，但 guest 没声明 9p 客户端。
    let mut no_nine = GuestImage::linux(
        "t",
        GuestRoot::Disk(std::path::PathBuf::from("/img/root.qcow2")),
        None,
        None,
    );
    no_nine.clients.nine_p = false;
    let spec2 = GuestSpec {
        lease: "w".to_string(),
        image: no_nine,
        mounts: Vec::new(),
        net: false,
        engine_path: None,
        share_path: None,
        seed: None,
        isolated: false,
    };
    let why2 = svc.guest_start(&spec2).expect_err("共享落不上要被拒绝");
    assert!(why2.contains("落不到引擎"), "{}", why2);
}

/// 目的：复制共享的进 / 出同步真跑一遍——宿主 tar 打包、经通道搬运、guest 端命令、回拷解包。
#[test]
pub(crate) fn copy_share_syncs_both_ways_through_the_channel() {
    use crate::capabilities::vm::detail::copy::{host_tar, CopyShare};
    use crate::capabilities::vm::ports::{GuestChannel, ShareBackend};

    struct RecordingChannel {
        execs: Vec<String>,
        puts: Vec<(String, usize)>,
        gets: Vec<String>,
        preset_get: Vec<u8>,
    }
    impl GuestChannel for RecordingChannel {
        fn exec(&mut self, run: &GuestRun) -> crate::kernel::api::ToolOutcome {
            self.execs.push(run.command.clone());
            crate::kernel::api::ToolOutcome {
                ok: true,
                output: String::new(),
            }
        }
        fn put_file(&mut self, path: &str, bytes: &[u8]) -> Result<(), String> {
            self.puts.push((path.to_string(), bytes.len()));
            Ok(())
        }
        fn get_file(&mut self, path: &str) -> Result<Vec<u8>, String> {
            self.gets.push(path.to_string());
            Ok(self.preset_get.clone())
        }
    }
    fn channel(preset_get: Vec<u8>) -> RecordingChannel {
        RecordingChannel {
            execs: Vec::new(),
            puts: Vec::new(),
            gets: Vec::new(),
            preset_get,
        }
    }

    let share = CopyShare::new();

    // 进去：宿主目录打成 tar，经通道写进 guest，再在 guest 里解开。
    let src = crate::tests::scratch("copy-sync-in");
    std::fs::write(src.join("a.txt"), b"hello-in").expect("写宿主文件");
    let mut ch = channel(Vec::new());
    let mounts = vec![GuestMount {
        host: src.clone(),
        guest: "/work".to_string(),
        read_only: false,
    }];
    share.sync_in(&mut ch, &mounts).expect("进去要成功");
    assert_eq!(ch.puts.len(), 1, "要把 tar 写进 guest：{:?}", ch.puts);
    assert!(ch.puts[0].1 > 0, "tar 不该是空的");
    assert!(
        ch.execs
            .iter()
            .any(|c| c.contains("tar -xf") && c.contains("'/work'")),
        "guest 端要有解包命令：{:?}",
        ch.execs
    );

    // 回来：guest 打包（通道给出 tar 字节），宿主解到可写挂载目录。
    let guest_dir = crate::tests::scratch("copy-sync-out-guest");
    std::fs::write(guest_dir.join("b.txt"), b"hello-out").expect("写 guest 文件");
    let preset = host_tar(&guest_dir).expect("打包");
    let dst = crate::tests::scratch("copy-sync-out-host");
    let mut ch2 = channel(preset);
    let mounts2 = vec![GuestMount {
        host: dst.clone(),
        guest: "/other".to_string(),
        read_only: false,
    }];
    share.sync_out(&mut ch2, &mounts2).expect("回来要成功");
    assert!(
        ch2.execs
            .iter()
            .any(|c| c.contains("tar -cf") && c.contains("'/other'")),
        "guest 端要有打包命令：{:?}",
        ch2.execs
    );
    assert_eq!(
        std::fs::read(dst.join("b.txt")).expect("回拷的文件"),
        b"hello-out".to_vec(),
        "可写目录要回拷"
    );

    // 只读挂载不回来（不打包、不取文件）。
    let mut ch3 = channel(Vec::new());
    let mounts3 = vec![GuestMount {
        host: dst.clone(),
        guest: "/ro".to_string(),
        read_only: true,
    }];
    share.sync_out(&mut ch3, &mounts3).expect("只读跳过");
    assert!(
        ch3.gets.is_empty() && ch3.execs.is_empty(),
        "只读挂载不回拷：{:?} {:?}",
        ch3.gets,
        ch3.execs
    );

    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(&guest_dir);
    let _ = std::fs::remove_dir_all(&dst);
}

/// 目的：guest.yaml 能声明 id / arch / agent / os / 共享客户端 / 自带运行能力；不认识的值如实拒收。
#[test]
pub(crate) fn guest_manifest_declares_the_image_and_rejects_unknowns() {
    let probe = crate::kernel::detail::HostProbeAdapter;
    let dir = crate::tests::scratch("vm-guest-manifest");
    std::fs::write(dir.join("vmlinuz"), b"x").expect("写内核");
    std::fs::write(
        dir.join("guest.yaml"),
        "id: custom-13\narch: arm64\nagent: qga\nclients: [cifs]\nruntimes: [python, node]\n",
    )
    .expect("写清单");
    let image = classify_base(&dir, &probe).expect("清单合法");
    assert_eq!(image.id, "custom-13");
    assert_eq!(
        image.runtimes,
        vec!["node".to_string(), "python".to_string()],
        "镜像自带的运行能力要读出来（升序去重）"
    );
    assert_eq!(
        image.arch,
        crate::capabilities::vm::domain::guest::GuestArch::Arm64
    );
    assert!(image.clients.cifs && !image.clients.nine_p, "按清单声明");

    std::fs::write(dir.join("guest.yaml"), "clients: [smb]\n").expect("写坏清单");
    assert!(
        classify_base(&dir, &probe).is_err(),
        "不认识的共享客户端要如实拒收"
    );

    std::fs::write(dir.join("guest.yaml"), "arch: riscv\n").expect("写坏 arch");
    assert!(classify_base(&dir, &probe).is_err(), "不认识的 arch 要拒收");

    // os 决定路径方言（路径桥按它选）；同方言才恒等。
    std::fs::write(dir.join("guest.yaml"), "os: windows\n").expect("写 os");
    let w = classify_base(&dir, &probe).expect("os 合法");
    assert_eq!(
        w.os.dialect(),
        crate::capabilities::vm::domain::paths::PathDialect::Windows,
        "os: windows 给出 Windows 方言"
    );
    // 具体换算由路径桥自己的用例锁（宿主路径的解析随平台而变，这里只判方言）。
    std::fs::write(dir.join("guest.yaml"), "os: plan9\n").expect("写坏 os");
    assert!(classify_base(&dir, &probe).is_err(), "不认识的 os 要拒收");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 目的：判出来的镜像带默认 agent 与共享客户端能力；缺客户端就不落对应后端。
#[test]
pub(crate) fn guest_image_carries_agent_and_client_capabilities() {
    use crate::capabilities::vm::ports::ShareBackend;
    let probe = crate::kernel::detail::HostProbeAdapter;
    let dir = crate::tests::scratch("vm-guest-clients");
    std::fs::write(dir.join("vmlinuz"), b"x").expect("写内核");
    let image = classify_base(&dir, &probe).expect("可用");
    assert_eq!(
        image.agent,
        crate::capabilities::vm::domain::guest::GuestAgent::Qga,
        "默认走 QGA 通道"
    );
    assert!(
        image.clients.nine_p && image.clients.virtio_fs,
        "Debian 默认带 9p 与 virtiofs 客户端"
    );
    assert!(
        !image.clients.cifs,
        "默认不带 CIFS 客户端（要镜像装 cifs-utils）"
    );
    assert!(!image.id.is_empty(), "镜像名要有");

    let nine = crate::capabilities::vm::detail::NinePShare::new();
    assert!(nine.supports("qemu", &image), "带 9p 客户端就能落 9p");
    let mut without_nine = image.clone();
    without_nine.clients.nine_p = false;
    assert!(
        !nine.supports("qemu", &without_nine),
        "没声明 9p 客户端就不落 9p"
    );
    assert!(!nine.supports("vz", &image), "9p 只落在 qemu 上");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 目的：机器能力与会话选型分开——vm_capability 不问 base；tier_readiness（保存/开始）才要求 base。
#[test]
pub(crate) fn vm_capability_is_machine_only_while_session_readiness_needs_base() {
    use crate::capabilities::vm::api::{vm_capability, VmInputs};
    use crate::tests::doubles::FixedProbe;
    let qemu = std::path::PathBuf::from("/opt/qemu-system-x86_64");
    let mut probe = FixedProbe {
        hypervisor: true,
        ..Default::default()
    };
    probe.files.push(qemu.clone());
    probe.exes.push("virtiofsd".to_string());
    let q = qemu.to_string_lossy().into_owned();
    let cap = vm_capability(&VmInputs {
        qemu: Some(q.as_str()),
        share_tool: None,
        probe: &probe,
    });
    if cfg!(any(target_os = "windows", target_os = "linux")) {
        assert!(
            cap.ready(),
            "已验平台的机器能力齐就该 ready：{:?}",
            cap.requirements
        );
    } else {
        // macOS：组合还没真机验收，guest 项按数据表判否——这正是「机器能力」与「已验平台」分开的体现。
        assert!(
            cap.requirements.iter().any(|r| r.id == "guest" && !r.met),
            "未验平台的 guest 项要判否：{:?}",
            cap.requirements
        );
    }
    assert!(
        !cap.requirements.iter().any(|r| r.id == "base"),
        "机器能力清单不含会话级 base：{:?}",
        cap.requirements
    );
    let spec = ExecSpec {
        tier: Tier::Vm,
        guest: None,
        ..Default::default()
    };
    let session = exec::tier_readiness(
        &crate::tests::root_dir(),
        &spec,
        Some(q.as_str()),
        None,
        &probe,
    );
    assert!(
        !session.ready(),
        "base 没填，会话级判据不成立：{:?}",
        session.requirements
    );
    assert!(
        session.unmet().iter().any(|r| r.id == "base"),
        "{:?}",
        session.requirements
    );
}

/// 目的：virtiofs 共享后端只在 QEMU + 声明了 virtiofs 客户端的 guest 上落地（缺客户端就是缺，不假装）。
#[test]
pub(crate) fn virtiofs_share_supports_only_qemu_with_virtiofs_client() {
    use crate::capabilities::vm::detail::VirtioFsShare;
    let probe = crate::kernel::detail::HostProbeAdapter;
    let dir = crate::tests::scratch("vm-virtiofs-clients");
    std::fs::write(dir.join("vmlinuz"), b"x").expect("写内核");
    let image = classify_base(&dir, &probe).expect("可用");
    let share = VirtioFsShare::new();
    assert!(
        share.supports("qemu", &image),
        "带 virtiofs 客户端就能落 virtiofs"
    );
    let mut without = image.clone();
    without.clients.virtio_fs = false;
    assert!(
        !share.supports("qemu", &without),
        "没声明 virtiofs 客户端就不落"
    );
    assert!(
        !share.supports("vz", &image),
        "virtiofs 走 QEMU 的 vhost-user-fs"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 目的：virtiofsd 是**外部资源**——装配要给出 vhost-user 设备与 guest 端挂载，释放后进程与套接字都不残留。
/// 约束：不依赖真 virtiofsd：用一个假守护进程顶替（落 pid + 建套接字 + 长睡），钉住起停与清理这条链。
#[test]
#[cfg(unix)]
pub(crate) fn virtiofs_guard_kills_the_daemon_and_removes_the_socket() {
    use crate::capabilities::vm::detail::VirtioFsShare;
    use std::os::unix::fs::PermissionsExt;
    let dir = crate::tests::scratch("vm-virtiofs-stub");
    let stub = dir.join("virtiofsd");
    std::fs::write(
        &stub,
        r#"#!/bin/sh
while [ $# -gt 0 ]; do
  case "$1" in --socket-path) sock=$2; shift 2;; *) shift;; esac
done
echo $$ > "$sock.pid"
: > "$sock"
sleep 300
"#,
    )
    .expect("写假 virtiofsd");
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).expect("置可执行位");
    let share_dir = dir.join("share");
    std::fs::create_dir_all(&share_dir).expect("建共享目录");
    let mounts = vec![
        GuestMount {
            host: share_dir.clone(),
            guest: "/shared".to_string(),
            read_only: false,
        },
        GuestMount {
            host: share_dir.clone(),
            guest: "/ro".to_string(),
            read_only: true,
        },
    ];
    let (plan, mut guard) = VirtioFsShare::new()
        .prepare("qemu", &mounts, Some(stub.to_string_lossy().as_ref()))
        .expect("假守护进程起得来");
    assert_eq!(plan.kind, "virtiofs");
    assert!(
        plan.needs_shared_mem,
        "vhost-user 设备要求 guest RAM 是可共享内存"
    );
    assert!(
        plan.engine_args
            .iter()
            .any(|a| a.starts_with("vhost-user-fs-pci,") && a.contains("tag=m0")),
        "要有 vhost-user-fs 设备：{:?}",
        plan.engine_args
    );
    assert!(
        plan.engine_args
            .iter()
            .any(|a| a.contains("path=") && a.contains("solomni-virtiofs-")),
        "要有 vhost-user 套接字：{:?}",
        plan.engine_args
    );
    assert!(
        plan.guest_scripts
            .iter()
            .any(|s| s.contains("mount -t virtiofs") && s.contains("m0") && s.contains("/shared")),
        "guest 端要挂 virtiofs：{:?}",
        plan.guest_scripts
    );
    assert!(
        plan.guest_scripts
            .iter()
            .any(|s| s.contains("m1") && s.contains("/ro") && s.contains(",ro")),
        "只读挂载要带 ro：{:?}",
        plan.guest_scripts
    );
    let sock = plan
        .engine_args
        .iter()
        .find_map(|a| a.strip_prefix("socket,id=vf0,path="))
        .map(std::path::PathBuf::from)
        .expect("engine_args 里要有 vhost-user 套接字");
    assert!(sock.exists(), "prepare 返回时套接字要在");
    // virtiofsd 在套接字旁落的 pid 文件就是它的进程号（释放时一并删掉）。
    let pid_file = format!("{}.pid", sock.display());
    let pid = std::fs::read_to_string(&pid_file)
        .expect("假守护进程要落 pid")
        .trim()
        .to_string();

    // 释放：进程死、套接字与 pid 文件都没了。
    guard.release();
    assert!(!sock.exists(), "释放后套接字要删掉：{}", sock.display());
    assert!(
        !std::path::Path::new(&pid_file).exists(),
        "释放后 pid 文件也不留"
    );
    let alive = std::process::Command::new("kill")
        .args(["-0", &pid])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(!alive, "释放后 virtiofsd 进程要没了（pid {}）", pid);
    // 幂等：再释放一次不该炸。
    guard.release();
    let _ = std::fs::remove_dir_all(&dir);
}

/// 目的：共享档的"在场"判据与就绪说明同源——登记了 / PATH 里有 virtiofsd 才算 virtiofs 可用，否则如实退到 9p。
/// 约束：共享档缺失**不参与承载判定**（有 9p 回退），只作事实呈现；否则会把性能档缺失误报成档位不可用。
#[test]
pub(crate) fn share_availability_gates_the_effective_combo() {
    use crate::capabilities::vm::domain::exec::{share_available, share_tool_of};
    use crate::tests::doubles::FixedProbe;
    assert_eq!(share_tool_of("virtiofs"), Some("virtiofsd"));
    assert_eq!(share_tool_of("9p"), None, "9p 不需要外部工具");
    let none = FixedProbe::default();
    assert!(
        !share_available("virtiofs", None, &none),
        "PATH 里没有 virtiofsd 就不算在场"
    );
    assert!(share_available("9p", None, &none), "9p 恒在场");
    let mut declared = FixedProbe::default();
    declared.exes.push("virtiofsd".to_string());
    assert!(
        share_available("virtiofs", None, &declared),
        "PATH 里有就算在场"
    );
    let ghost = std::path::PathBuf::from("/nowhere/virtiofsd");
    assert!(
        !share_available(
            "virtiofs",
            Some(ghost.to_string_lossy().as_ref()),
            &declared
        ),
        "登记了路径就必须是那个文件（登记优先于 PATH）"
    );

    // 就绪项：没有 virtiofsd 时 share 项仍 met（不拦档），但要说清回退与怎么补。
    let spec = ExecSpec {
        tier: Tier::Vm,
        guest: None,
        ..Default::default()
    };
    let r = exec::tier_readiness(&crate::tests::root_dir(), &spec, None, None, &none);
    let share = r
        .requirements
        .iter()
        .find(|x| x.id == "share")
        .expect("清单里要有共享档这一项");
    assert!(share.met, "共享档缺失不该拦住虚拟机档（有回退档）");
    if cfg!(windows) {
        // Windows 的已实现链是 smb → copy：缺 smbd 时退回 copy，说明里要同时给出 smb/smbd 与回退。
        assert!(
            share.detail.contains("smb")
                && share.detail.contains("smbd")
                && share.detail.contains("copy"),
            "要说清回退到 copy 与怎么补 smbd：{:?}",
            share
        );
    } else {
        // Linux：virtiofs 优先，缺 virtiofsd 时退回 9p；macOS：默认链是 vz/virtiofs（vz 未实现），
        // 实际落到 qemu/9p——9p 不需要外部工具，没有「怎么补」可写。
        assert!(share.detail.contains("9p"), "要说清落到 9p：{:?}", share);
        if cfg!(target_os = "linux") {
            assert!(
                share.detail.contains("virtiofsd"),
                "要说清怎么补 virtiofsd：{:?}",
                share
            );
        }
    }
}

/// 目的：按平台的组合回退链是数据；引擎 / 共享不在场时逐档回退，都缺才没有组合。
#[test]
pub(crate) fn default_combos_follow_the_platform_and_fall_back() {
    use crate::capabilities::vm::domain::exec::{choose_combo, default_combos};
    let ids = |host: &str| {
        default_combos(host)
            .iter()
            .map(|c| format!("{}/{}", c.engine, c.share))
            .collect::<Vec<_>>()
    };
    assert_eq!(ids("linux")[0], "qemu/virtiofs");
    assert_eq!(ids("macos")[0], "vz/virtiofs");
    assert_eq!(ids("windows")[0], "qemu/smb");

    let engine_ok = |e: &str| e == "qemu";
    let share_ok = |_e: &str, s: &str| s == "9p" || s == "copy";
    let linux = choose_combo(default_combos("linux"), &engine_ok, &share_ok)
        .map(|c| format!("{}/{}", c.engine, c.share));
    assert_eq!(
        linux.as_deref(),
        Some("qemu/9p"),
        "virtiofs 不在场就退到 9p"
    );

    let copy_only = |_e: &str, s: &str| s == "copy";
    let win = choose_combo(default_combos("windows"), &engine_ok, &copy_only)
        .map(|c| format!("{}/{}", c.engine, c.share));
    assert_eq!(
        win.as_deref(),
        Some("qemu/copy"),
        "Windows 上 smb 不在场就退到 copy"
    );

    let none = choose_combo(default_combos("windows"), &|_e| false, &|_e, _s| true);
    assert!(none.is_none(), "引擎不在场就没有组合");
}

/// 目的：已验平台是数据表（不是平台 cfg）：Linux 已验，Windows / macOS 未验。
#[test]
pub(crate) fn verified_platforms_are_a_data_table() {
    assert!(
        crate::capabilities::vm::domain::exec::host_verified("linux"),
        "Linux 已真机验收"
    );
    assert!(
        crate::capabilities::vm::domain::exec::host_verified("windows"),
        "Windows 已真机验收（QEMU/WHPX + QGA 复制；e2e 的 vm 场景通过）"
    );
    assert!(
        !crate::capabilities::vm::domain::exec::host_verified("macos"),
        "macOS 待验"
    );
}

#[test]
pub(crate) fn vm_service_holds_guests_by_lease() {
    let engine = Arc::new(crate::tests::doubles::FakeEngine::new());
    let vm = crate::capabilities::vm::service::VmService::new(
        Arc::new(InMemoryPackages::empty()),
        engine.clone(),
        Arc::new(crate::tests::doubles::FakeShare::new()),
    );
    let spec = GuestSpec {
        lease: "w".to_string(),
        image: GuestImage::linux(
            "test",
            GuestRoot::Dir(std::path::PathBuf::from("/base")),
            Some(std::path::PathBuf::from("/base/vmlinuz")),
            None,
        ),
        mounts: Vec::new(),
        net: false,
        engine_path: None,
        share_path: None,
        seed: None,
        isolated: false,
    };
    vm.guest_start(&spec).expect("替身一定能起");
    let run = GuestRun {
        command: "echo hi".to_string(),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: String::new(),
    };
    assert!(vm.guest_run("w", &run).ok, "有租约就能跑");
    assert!(!vm.guest_run("nope", &run).ok, "没这个租约要如实失败");
    vm.guest_stop("w");
    let calls = engine.calls.lock().expect("锁").clone();
    assert!(calls.iter().any(|c| c == "start:w"), "{:?}", calls);
    assert!(calls.iter().any(|c| c == "run:w:echo hi"), "{:?}", calls);
    assert!(calls.iter().any(|c| c == "stop:w"), "{:?}", calls);
}

/// 目的：真机验收（默认跳过）：用项目内 QEMU 与虚拟根真起一个 guest，在它里面跑命令。
/// 约束：只在 SOLOMNI_VM_LIVE=1 且 SOLOMNI_VM_QEMU 指向真东西、虚拟根（默认 `debian`）在场时执行；否则打印 [探针] 如实跳过。
#[test]
pub(crate) fn vm_live_guest_runs_command_inside_guest() {
    let live = std::env::var("SOLOMNI_VM_LIVE").as_deref() == Ok("1");
    let qemu = std::env::var("SOLOMNI_VM_QEMU").unwrap_or_default();
    // 虚拟根名（默认 debian）；在场与否由 resolve_guest 解析出的那一格判。
    let guest = std::env::var("SOLOMNI_VM_GUEST").unwrap_or_else(|_| "debian".to_string());
    if !live || qemu.trim().is_empty() {
        eprintln!("[探针] 未开启虚拟机档真机验收：设 SOLOMNI_VM_LIVE=1、SOLOMNI_VM_QEMU 后重跑（虚拟根默认 debian，可用 SOLOMNI_VM_GUEST 换）");
        return;
    }
    let probe = crate::kernel::detail::HostProbeAdapter;
    let base_dir = match crate::capabilities::vm::api::resolve_guest(
        &crate::tests::root_dir(),
        &guest,
    ) {
        Ok(d) if probe.is_dir(&d) => d,
        _ => {
            eprintln!("[探针] 本机没有虚拟根「{}」：先 solomni --provision --ensure --yes，或用 SOLOMNI_VM_GUEST 指定", guest);
            return;
        }
    };
    let image = classify_base(&base_dir, &probe).expect("虚拟根要可用");
    // 真机引擎级验收：共享要真进 guest（读得到标记），guest 的写要即时落到宿主（实时共享），收尾不留守护进程。
    let share = crate::tests::scratch("vm-live-share");
    std::fs::create_dir_all(&share).expect("建共享目录");
    std::fs::write(share.join("marker.txt"), "SHARE_MARKER").expect("写标记");
    let guest_dir = "/solomni-vm-live-share";
    // 会话走**每租约可写层**（与产品同一条路）：基础镜像只读，可写层落在临时目录、随 guest 一起删。
    let overlay_dir = crate::tests::scratch("vm-live-overlays");
    let engine = crate::capabilities::vm::detail::QemuEngine::with_overlay_dir(overlay_dir.clone());
    let virtiofsd = std::env::var("SOLOMNI_VM_VIRTIOFSD")
        .ok()
        .filter(|p| !p.trim().is_empty());
    let smbd = std::env::var("SOLOMNI_VM_SMBD")
        .ok()
        .filter(|p| !p.trim().is_empty());
    let spec = GuestSpec {
        lease: "vm-live".to_string(),
        image,
        mounts: vec![GuestMount {
            host: share.clone(),
            guest: guest_dir.to_string(),
            read_only: false,
        }],
        net: false,
        engine_path: Some(qemu),
        share_path: virtiofsd.clone(),
        seed: None,
        isolated: true,
    };
    // 共享后端按平台与在场情况选（与组合根同一份判据）：smb(Windows) > virtiofs > copy(QGA) > 9p。
    let nine = crate::capabilities::vm::detail::NinePShare::new();
    let copy = crate::capabilities::vm::detail::CopyShare::new();
    let virtiofs = crate::capabilities::vm::detail::VirtioFsShare::new();
    let smb = crate::capabilities::vm::detail::SmbShare::new();
    let use_smb = std::env::consts::OS == "windows" && spec.image.clients.cifs && smbd.is_some();
    let use_virtiofs = !use_smb && virtiofsd.is_some() && spec.image.clients.virtio_fs;
    let use_copy = !use_smb
        && !use_virtiofs
        && (std::env::consts::OS == "windows" || !spec.image.clients.nine_p);
    let backend: &dyn crate::capabilities::vm::ports::ShareBackend = if use_smb {
        &smb
    } else if use_virtiofs {
        &virtiofs
    } else if use_copy {
        &copy
    } else {
        &nine
    };
    let tool = if use_smb {
        smbd.as_deref()
    } else {
        virtiofsd.as_deref()
    };
    let (plan, mut guard) = backend
        .prepare("qemu", &spec.mounts, tool)
        .expect("共享计划要能派生");
    let mut guest = engine.boot(&spec, &plan).expect("起 guest");
    // 每租约可写层只对**磁盘根**建（目录根本来就是可写的共享根，产品也走同一条路）。
    if matches!(spec.image.root, GuestRoot::Disk(_)) {
        let overlay = overlay_dir.join("vm-live.qcow2");
        assert!(
            overlay.is_file(),
            "每租约可写层要建出来：{}",
            overlay.display()
        );
    }
    if use_copy {
        // 复制后端没有"挂载"：进去之前先把宿主目录同步到 guest，marker 才在里面。
        backend
            .sync_in(guest.channel(), &spec.mounts)
            .expect("同步宿主目录进去要成功");
    }
    let run = GuestRun {
        command: format!(
            "cat /etc/os-release; test -e '{}' && echo HOST_VISIBLE || echo HOST_HIDDEN; cat {}/marker.txt; echo VM_LIVE_OK > {}/from-guest.txt; echo VM_LIVE_OK_WRITTEN",
            std::env::current_dir().expect("cwd").display(),
            guest_dir,
            guest_dir
        ),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: String::new(),
    };
    let out = guest.channel().exec(&run);
    if use_copy && out.ok {
        // 复制的语义：执行后整棵回拷，宿主才看得到 guest 的写。
        backend
            .sync_out(guest.channel(), &spec.mounts)
            .expect("回拷 guest 目录要成功");
    }
    if use_virtiofs || use_smb {
        // 实时共享：**不等任何回拷**，宿主此刻就该看得到 guest 写的内容。
        let written = std::fs::read_to_string(share.join("from-guest.txt")).unwrap_or_default();
        assert!(
            written.contains("VM_LIVE_OK"),
            "{} 要实时可见：宿主读到 {:?}（guest 输出：{}）",
            backend.id(),
            written,
            out.output
        );
    }
    guest.stop();
    // 同上：只有磁盘根才建了那个可写层，删租约也只对它有"要删掉"这回事。
    if matches!(spec.image.root, GuestRoot::Disk(_)) {
        let overlay = overlay_dir.join("vm-live.qcow2");
        assert!(
            !overlay.is_file(),
            "删租约要把可写层删掉：{}",
            overlay.display()
        );
    }
    // 删租约 = 守护进程与套接字都不留（与产品 service 释放的是同一份凭据）。
    guard.release();
    if use_virtiofs {
        let mut alive: Vec<String> = Vec::new();
        for _ in 0..12 {
            alive = std::process::Command::new("pgrep")
                .args(["-af", "virtiofsd"])
                .output()
                .map(|pg| {
                    String::from_utf8_lossy(&pg.stdout)
                        .lines()
                        .filter(|l| l.contains("--socket-path") && l.contains("solomni-virtiofs-"))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            if alive.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        assert!(alive.is_empty(), "释放后不该残留 virtiofsd：{:?}", alive);
    }
    assert!(out.ok, "guest 里执行要成功：{}", out.output);
    assert!(out.output.contains("VM_LIVE_OK"), "输出：{}", out.output);
    assert!(
        out.output.contains("VM_LIVE_OK_WRITTEN"),
        "guest 要能往共享目录写：{}",
        out.output
    );
    assert!(
        out.output.contains("Debian GNU/Linux"),
        "要在 guest 的 os-release 里证明真跑在 guest 内：{}",
        out.output
    );
    assert!(
        out.output.contains("HOST_HIDDEN"),
        "宿主的项目根不该在 guest 里可达：{}",
        out.output
    );
    assert!(
        out.output.contains("SHARE_MARKER"),
        "额外共享要真进 guest：{}",
        out.output
    );
    let _ = std::fs::remove_dir_all(&share);
}

/// 目的：并发准入——到上限拒绝、同一租约重启放行、0 = 不限制。
#[test]
pub(crate) fn guest_admission_caps_concurrency() {
    let two = vec!["a".to_string(), "b".to_string()];
    assert!(
        exec::admit_guest(&two, "a", 2).is_ok(),
        "同一租约重启不占新名额"
    );
    assert!(exec::admit_guest(&two, "c", 2).is_err(), "到上限要拒");
    assert!(exec::admit_guest(&two, "c", 3).is_ok(), "没到上限放行");
    assert!(exec::admit_guest(&two, "c", 0).is_ok(), "0 = 不限制");
    assert!(exec::admit_guest(&[], "c", 2).is_ok());
}

/// 目的：跨租约的执行要真并行——老的表锁把两个租约串起来（前一个跑完才轮到后一个）；
///   这里两个租约的 exec 必须见面（各自最多等对方 3 秒）。
#[test]
pub(crate) fn guests_on_different_leases_run_concurrently() {
    use crate::capabilities::vm::api::Vm;
    use crate::capabilities::vm::ports::{GuestChannel, GuestVm, SharePlan, VmEngine};
    use std::sync::atomic::AtomicBool;

    #[derive(Clone)]
    struct Rv {
        in_flight: Arc<AtomicUsize>,
        met: Arc<AtomicBool>,
    }
    struct RvChan(Rv);
    impl GuestChannel for RvChan {
        fn exec(&mut self, _run: &GuestRun) -> ToolOutcome {
            self.0.in_flight.fetch_add(1, Ordering::SeqCst);
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while std::time::Instant::now() < deadline {
                if self.0.in_flight.load(Ordering::SeqCst) >= 2 {
                    self.0.met.store(true, Ordering::SeqCst);
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            self.0.in_flight.fetch_sub(1, Ordering::SeqCst);
            ToolOutcome {
                ok: true,
                output: "ok".to_string(),
            }
        }
    }
    struct RvVm(RvChan);
    impl GuestVm for RvVm {
        fn channel(&mut self) -> &mut dyn GuestChannel {
            &mut self.0
        }
        fn stop(&mut self) {}
    }
    struct RvEngine(Rv);
    impl VmEngine for RvEngine {
        fn id(&self) -> &'static str {
            "rv"
        }
        fn supports(&self, _guest: &GuestImage) -> bool {
            true
        }
        fn boot(&self, _spec: &GuestSpec, _share: &SharePlan) -> Result<Box<dyn GuestVm>, String> {
            Ok(Box::new(RvVm(RvChan(self.0.clone()))))
        }
    }

    let rv = Rv {
        in_flight: Arc::new(AtomicUsize::new(0)),
        met: Arc::new(AtomicBool::new(false)),
    };
    let vm = crate::capabilities::vm::service::VmService::new(
        Arc::new(InMemoryPackages::empty()),
        Arc::new(RvEngine(rv.clone())),
        Arc::new(FakeShare::new()),
    );
    for lease in ["lease-a", "lease-b"] {
        let image = GuestImage::linux("t", GuestRoot::Dir(PathBuf::from("/x")), None, None);
        vm.guest_start(&GuestSpec {
            lease: lease.to_string(),
            image,
            mounts: Vec::new(),
            net: false,
            engine_path: None,
            share_path: None,
            seed: None,
            isolated: false,
        })
        .expect("拉起");
    }
    let run = GuestRun {
        command: "true".to_string(),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: String::new(),
    };
    std::thread::scope(|s| {
        for lease in ["lease-a", "lease-b"] {
            let vm = &vm;
            let run = &run;
            s.spawn(move || {
                let _ = vm.guest_run(lease, run);
            });
        }
    });
    assert!(
        rv.met.load(Ordering::SeqCst),
        "两个租约的 exec 要同时在飞（跨租约没被串行化）"
    );
}

/// 目的：路径桥的**反向**——guest 写法译回宿主；不在镜像根里、也不是宿主绝对路径要拒。
#[test]
pub(crate) fn path_bridge_round_trips_guest_and_host() {
    use crate::capabilities::vm::api::PathBridge;
    use crate::capabilities::vm::domain::paths::{guest_to_host, PathDialect};
    use std::path::Path;
    // POSIX 宿主 + Windows guest：桥按**构造的**宿主平台判，与真实平台无关，所以任何平台都跑——
    // CI 的 POSIX 宿主就靠它；只写进 cfg(unix) 分支会让 Windows 本地漏判（丢根就是这么漏过去的）。
    let l = PathBridge::new("linux", PathDialect::Windows);
    let posix = Path::new("/home/u/a");
    let g = l.to_guest(posix);
    assert_eq!(g, "C:\\solomni-host\\home\\u\\a");
    assert_eq!(
        guest_to_host(l, &g),
        Some(posix.to_path_buf()),
        "解出来要落回同一个绝对路径"
    );
    // 同方言（本机档）恒等。
    let id = PathBridge::new("linux", PathDialect::Posix);
    assert_eq!(guest_to_host(id, "/work/x"), Some(PathBuf::from("/work/x")));
    if cfg!(windows) {
        let b = PathBridge::new("windows", PathDialect::Posix);
        let host = Path::new(r"D:\a\b");
        let g = b.to_guest(host);
        assert_eq!(g, "/solomni-host/d/a/b");
        assert_eq!(
            guest_to_host(b, &g),
            Some(host.to_path_buf()),
            "镜像路径要能解回来"
        );
        assert_eq!(
            guest_to_host(b, "/etc/passwd"),
            None,
            "不在镜像根里、也不是宿主绝对路径"
        );
        assert_eq!(
            guest_to_host(b, r"D:\a\b"),
            Some(host.to_path_buf()),
            "宿主写法直通"
        );
    }
}
