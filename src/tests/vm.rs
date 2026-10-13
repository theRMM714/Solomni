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
        &ExecSpec::default(),
        None,
        &crate::kernel::detail::HostProbeAdapter,
    );
    assert!(host.ready(), "本机档没有前置条件");
    assert!(host.requirements.is_empty(), "本机档不该有虚拟机前置清单");
    assert!(exec::tier_refusal(
        &ExecSpec::default(),
        None,
        &crate::kernel::detail::HostProbeAdapter
    )
    .is_none());

    // 虚拟机档：guest 本体按**已验证平台**放开（Linux 已验，Windows / macOS 待验），其余项按事实；这里基础根是假的。
    let ghost = ExecSpec {
        tier: Tier::Vm,
        base: Some("definitely-not-a-real-base-root".to_string()),
        ..Default::default()
    };
    let r = exec::tier_readiness(&ghost, None, &crate::kernel::detail::HostProbeAdapter);
    assert!(!r.ready(), "前置不齐就不成立：{:?}", r);
    let guest_item = r
        .requirements
        .iter()
        .find(|x| x.id == "guest")
        .expect("清单里要有 guest 这一项");
    assert_eq!(
        guest_item.met,
        cfg!(target_os = "linux"),
        "guest 项按已验证平台放开：{:?}",
        guest_item
    );
    let combo = {
        let c = crate::capabilities::vm::domain::exec::default_combos(std::env::consts::OS)[0];
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
    let why = exec::tier_refusal(&ghost, None, &crate::kernel::detail::HostProbeAdapter)
        .expect("不成立就要给可读理由");
    assert!(why.contains("虚拟机档现在不可用"), "{}", why);

    // 基础根在场：base 这一项要认出来；再故意登记一个不存在的 QEMU 路径，验证仍不成立（不随机器变）。
    let dir = crate::tests::base_dir("tier-readiness");
    let real = ExecSpec {
        tier: Tier::Vm,
        base: Some(dir.to_string_lossy().into_owned()),
        ..Default::default()
    };
    let r2 = exec::tier_readiness(
        &real,
        Some("definitely-not-qemu.exe"),
        &crate::kernel::detail::HostProbeAdapter,
    );
    let base_item = r2
        .requirements
        .iter()
        .find(|x| x.id == "base")
        .expect("清单里要有基础根这一项");
    assert!(base_item.met, "在场的基础根要认出来：{:?}", base_item);
    assert!(!r2.ready(), "缺 QEMU 一项就不成立");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 目的：QEMU 检测——登记了就用登记的路径，没登记就看 PATH；产品不自带、不下载。
#[test]
pub(crate) fn vm_requirements_report_qemu_registration() {
    let base = crate::tests::scratch("vm-req-qemu");
    let base_str = base.to_string_lossy().into_owned();
    let spec = ExecSpec {
        tier: Tier::Vm,
        base: Some(base_str.clone()),
        ..Default::default()
    };

    // 登记了一个不存在的路径：必须报"找不到"，并给出怎么补。
    let r = exec::tier_readiness(
        &spec,
        Some("definitely-not-qemu.exe"),
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
        &spec,
        Some(fake.to_string_lossy().as_ref()),
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
pub(crate) fn exec_host_tier_ignores_packages() {
    let modules = vec![module_with_runtimes("a", &["python"])];
    let plan = exec::plan(&ExecSpec::default(), &modules, &Library::default())
        .expect("本机档不装载运行包，不会因缺包失败");
    assert_eq!(plan.tier, Tier::Host);
    assert!(plan.packages.is_empty());
    assert!(plan.base.is_none());
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
        exec::vm_diagnoses(&modules, &empty, &spec),
        vec![Diagnosis::Missing {
            module: "a".to_string(),
            capability: "python".to_string()
        }]
    );
    let un = exec::unavailable(&spec, &modules, &empty);
    assert_eq!(
        un.get("a"),
        Some(&vec!["python".to_string()]),
        "虚拟机档缺包 = 该模块工具不可用"
    );
    assert!(
        exec::unavailable(&ExecSpec::default(), &modules, &empty).is_empty(),
        "本机档一律可用"
    );
    // 缺包不拦会话：计划里没有可装载的包，那一步的降级由 unavailable 收口。
    let partial = exec::plan(&spec, &modules, &empty).expect("缺包不该拦会话");
    assert!(partial.packages.is_empty());
    let said = exec::diagnose_text(&exec::vm_diagnoses(&modules, &empty, &spec));
    assert!(
        said.contains("runtimes/"),
        "缺包的说法要告诉用户把包放哪：{}",
        said
    );
    // 多版本且未定版 = 不替用户选
    let two = Library::build(
        vec![pkg("python", "3.12.4"), pkg("python", "3.11.9")],
        Vec::new(),
    );
    assert!(exec::vm_diagnoses(&modules, &two, &spec)
        .iter()
        .any(|d| matches!(d, Diagnosis::Ambiguous { .. })));
    // 定版指定的版本不在库里 = 如实报
    let bad = ExecSpec {
        pins: BTreeMap::from([("python".to_string(), "9.9".to_string())]),
        ..vm_spec()
    };
    assert!(exec::vm_diagnoses(&modules, &two, &bad)
        .iter()
        .any(|d| matches!(d, Diagnosis::UnknownPin { .. })));
    // 多版本未定版 = 选型不成立：派生计划如实拒绝（这是用户要解决的选型问题，不是"缺包"）
    let refused = exec::plan(&spec, &modules, &two).unwrap_err();
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
    assert!(exec::vm_diagnoses(&modules, &two, &ok).is_empty());
    assert_eq!(exec::plan(&ok, &modules, &two).unwrap().packages.len(), 1);
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
    let plan = exec::plan(&vm_spec(), &modules, &lib).expect("虚拟机档可成立");
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
    assert_eq!(plan.base.as_deref(), Some("base-linux"));
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

#[test]
pub(crate) fn guest_mounts_prefix_packages_and_fence_roots() {
    use crate::capabilities::vm::domain::exec::{ExecPlan, PlannedPackage};
    use crate::capabilities::vm::domain::guest::mounts_from;
    let plan = ExecPlan {
        tier: Tier::Vm,
        net: false,
        base: None,
        packages: vec![PlannedPackage {
            id: "python".to_string(),
            version: "3.12.4".to_string(),
            kind: "prefix".to_string(),
            prefix: "opt/rt/py".to_string(),
            source_dir: std::path::PathBuf::from("/rt/py"),
        }],
    };
    let fence = crate::kernel::api::FenceSpec::standalone(
        std::path::Path::new("/mods/a"),
        Some(std::path::Path::new("/work/x")),
        false,
    );
    let mounts = mounts_from(&plan, &fence);
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
        .prepare("qemu", &mounts)
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
    };
    let args = crate::capabilities::vm::detail::qemu::engine_args(
        &spec,
        "host=127.0.0.1,port=1,server=on,wait=off",
        false,
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

    let plain = engine_args(&spec(false), "opts", false);
    assert!(
        plain.contains(&"none".to_string()) && !plain.iter().any(|a| a.contains("netdev")),
        "没有隧道需求就不给网卡：{:?}",
        plain
    );
    let tunnel = engine_args(&spec(false), "opts", true);
    assert!(
        tunnel.iter().any(|a| a == "user,id=n0,restrict=on"),
        "共享隧道要受限网卡（出站全断）：{:?}",
        tunnel
    );
    let online = engine_args(&spec(true), "opts", false);
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
    };
    let args = engine_args(&spec, "opts", false);
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
    };
    let args = engine_args(&spec, "opts", false);
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
    let (plan, _guard) = share.prepare("qemu", &mounts).expect("复制计划");
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
    let dir = crate::tests::scratch("vm-guest-disk");
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
        base: Some(dir.to_string_lossy().into_owned()),
        ..Default::default()
    };
    let req = exec::tier_readiness(&spec, None, &probe);
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
        },
        "opts",
        false,
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

/// 目的：guest.yaml 能声明 id / arch / agent / 共享客户端；不认识的值如实拒收。
#[test]
pub(crate) fn guest_manifest_declares_the_image_and_rejects_unknowns() {
    let probe = crate::kernel::detail::HostProbeAdapter;
    let dir = crate::tests::scratch("vm-guest-manifest");
    std::fs::write(dir.join("vmlinuz"), b"x").expect("写内核");
    std::fs::write(
        dir.join("guest.yaml"),
        "id: custom-13\narch: arm64\nagent: qga\nclients: [cifs]\n",
    )
    .expect("写清单");
    let image = classify_base(&dir, &probe).expect("清单合法");
    assert_eq!(image.id, "custom-13");
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
        !crate::capabilities::vm::domain::exec::host_verified("windows"),
        "Windows 待验（宿主↔guest 路径一致未解决，见 tests/gaps.yaml）"
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

/// 目的：真机验收（默认跳过）：用项目内 QEMU 与基础根真起一个 guest，在它里面跑命令。
/// 约束：只在 SOLOMNI_VM_LIVE=1 且 SOLOMNI_VM_QEMU / SOLOMNI_VM_BASE 指向真东西时执行；否则打印 [探针] 如实跳过。
#[test]
pub(crate) fn vm_live_guest_runs_command_inside_guest() {
    let live = std::env::var("SOLOMNI_VM_LIVE").as_deref() == Ok("1");
    let qemu = std::env::var("SOLOMNI_VM_QEMU").unwrap_or_default();
    let base_dir = std::env::var("SOLOMNI_VM_BASE").unwrap_or_default();
    if !live || qemu.trim().is_empty() || base_dir.trim().is_empty() {
        eprintln!("[探针] 未开启虚拟机档真机验收：设 SOLOMNI_VM_LIVE=1、SOLOMNI_VM_QEMU、SOLOMNI_VM_BASE 后重跑");
        return;
    }
    let probe = crate::kernel::detail::HostProbeAdapter;
    let image = classify_base(std::path::Path::new(base_dir.trim()), &probe).expect("基础根要可用");
    // 额外共享要真进 guest：用一份带标记的临时目录验证（9p 靠挂载，copy 靠同步）。
    let share = crate::tests::scratch("vm-live-share");
    std::fs::create_dir_all(&share).expect("建共享目录");
    std::fs::write(share.join("marker.txt"), "SHARE_MARKER").expect("写标记");
    let guest_dir = "/solomni-vm-live-share";
    let engine = crate::capabilities::vm::detail::QemuEngine::new();
    let spec = GuestSpec {
        lease: "vm-live".to_string(),
        image,
        mounts: vec![GuestMount {
            host: share.clone(),
            guest: guest_dir.to_string(),
            read_only: true,
        }],
        net: false,
        engine_path: Some(qemu),
    };
    // 共享后端按平台选（与组合根一致）：Windows 宿主没有 virtfs，用 QGA 复制；其余平台用 9p。
    let use_copy = std::env::consts::OS == "windows" || !spec.image.clients.nine_p;
    let nine = crate::capabilities::vm::detail::NinePShare::new();
    let copy = crate::capabilities::vm::detail::CopyShare::new();
    let backend: &dyn crate::capabilities::vm::ports::ShareBackend =
        if use_copy { &copy } else { &nine };
    let (plan, _guard) = backend
        .prepare("qemu", &spec.mounts)
        .expect("共享计划要能派生");
    let mut guest = engine.boot(&spec, &plan).expect("起 guest");
    if use_copy {
        // 复制后端没有"挂载"：进去之前先把宿主目录同步到 guest，marker 才在里面。
        backend
            .sync_in(guest.channel(), &spec.mounts)
            .expect("同步宿主目录进去要成功");
    }
    let run = GuestRun {
        command: format!(
            "cat /etc/os-release; test -e '{}' && echo HOST_VISIBLE || echo HOST_HIDDEN; cat {}/marker.txt; echo VM_LIVE_OK",
            std::env::current_dir().expect("cwd").display(),
            guest_dir
        ),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: String::new(),
    };
    let out = guest.channel().exec(&run);
    guest.stop();
    assert!(out.ok, "guest 里执行要成功：{}", out.output);
    assert!(out.output.contains("VM_LIVE_OK"), "输出：{}", out.output);
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
        "额外 9p 共享要真挂进 guest：{}",
        out.output
    );
    let _ = std::fs::remove_dir_all(&share);
}
