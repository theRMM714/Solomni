//! 目的：入口判据——宿主产物在 guest 里跑不了，必须在预检就判否。
//! 管：跨平台判据、补齐面指向虚拟根。
//! 不管：路径桥（`guestpath.rs`）。
//! 联动：`src/capabilities/vm/domain/guest.rs` 与 `RUNTIME_SPEC.md`。

use super::super::builders::*;
use super::super::prelude::*;
use crate::capabilities::vm::domain::guest::classify_base;

/// 目的：跨平台判入口要看**文件形式**——宿主产物（PE）在 linux guest 里跑不了，必须在预检就判否。
#[test]
pub(crate) fn runs_on_target_checks_the_file_form_not_just_the_name() {
    use crate::capabilities::vm::domain::exec::runs_on_target;
    let dir = crate::tests::scratch("entry-form");
    let write = |name: &str, bytes: &[u8]| {
        let p = dir.join(name);
        std::fs::write(&p, bytes).expect("写入口样例");
        p
    };
    let elf = write("elf.bin", b"\x7fELF\x02\x01\x01\x00rest");
    let pe = write("pe.bin", b"MZ\x90\x00rest");
    let script = write("script.bin", b"#!/bin/sh\necho hi\n");
    let plain = write("plain.bin", b"just text\n");
    assert!(runs_on_target(&elf, "linux"), "ELF 在 linux 上能跑");
    assert!(!runs_on_target(&pe, "linux"), "PE 不是 linux 能执行的东西");
    assert!(runs_on_target(&script, "linux"), "带 shebang 的脚本能跑");
    assert!(
        !runs_on_target(&plain, "linux"),
        "没有 shebang 的文本不是可执行入口"
    );
    assert!(runs_on_target(&pe, "windows"), "PE 是 Windows 的可执行体");
    assert!(!runs_on_target(&elf, "windows"), "ELF 在 Windows 上跑不了");
    // 批处理按扩展名认（Windows 的壳这么做）。
    let bat = write("build.cmd", b"@echo off\r\n");
    assert!(
        runs_on_target(&bat, "windows"),
        "批处理是 Windows 的可执行入口"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 目的：同一份"只有 .exe"的现场——Windows 宿主上判本机档算在场，判 linux guest 就必须算缺。
/// 约束：名字与形式两道都判：`.exe` 在 linux 里连名字都不认；即便有人把 PE 复制成无扩展名的 `build/indexer`，
///   形式那一道也要拒绝（这正是这次现场踩到的坑）。
#[test]
pub(crate) fn entry_check_rejects_host_artifacts_for_a_guest_of_another_platform() {
    use crate::capabilities::vm::domain::exec::{missing_entries_for, runs_on_target};
    let dir = crate::tests::scratch("entry-cross");
    let build = dir.join("build");
    std::fs::create_dir_all(&build).expect("建 build/");
    std::fs::write(build.join("indexer.exe"), b"MZ\x90\x00").expect("写 Windows 产物");
    let mut m = module_of("a");
    m.root = dir.clone();
    m.manifest.tools.insert(
        "build".to_string(),
        crate::capabilities::modules::api::ToolDecl {
            command: "build/indexer build".to_string(),
            desc: String::new(),
            params: None,
            parallel: false,
        },
    );
    // Windows 宿主 + 本机档：`.exe` 在场即可（宿主自己跑）。
    let host_ok = |p: &std::path::Path| p.is_file();
    assert!(missing_entries_for(&[m.clone()], "windows", &host_ok).is_empty());
    // Windows 宿主 + linux guest：名字不认，必须算缺。
    assert_eq!(
        missing_entries_for(&[m.clone()], "linux", &host_ok).len(),
        1
    );
    // 就算把同一个 PE 复制成无扩展名的 `build/indexer`：名字过了，形式也要拒。
    std::fs::copy(build.join("indexer.exe"), build.join("indexer")).expect("复制成字面名");
    let with_form = |p: &std::path::Path| p.is_file() && runs_on_target(p, "linux");
    assert_eq!(
        missing_entries_for(&[m.clone()], "linux", &with_form).len(),
        1,
        "PE 复制成 build/indexer 也不能算 linux guest 的可用入口"
    );
    // 真给它一份 ELF：这才放行。
    std::fs::write(build.join("indexer"), b"\x7fELF\x02\x01\x01\x00").expect("写 ELF 产物");
    assert!(missing_entries_for(&[m], "linux", &with_form).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 目的：虚拟机档的"怎么补"要指到虚拟根里编——宿主上编出来的产物在 guest 里跑不了。
#[test]
pub(crate) fn diagnose_text_points_a_guest_session_at_building_inside_the_guest() {
    let diags = [Diagnosis::MissingEntry {
        module: "indexer".to_string(),
        tool: "build".to_string(),
        command: "build/indexer".to_string(),
        buildable: true,
    }];
    let host = exec::diagnose_text_for(&diags, None);
    assert!(host.contains("--build=indexer"), "{}", host);
    assert!(!host.contains("--guest="), "本机档不该提虚拟根：{}", host);
    let guest = exec::diagnose_text_for(&diags, Some("windows"));
    assert!(
        guest.contains("--build=indexer --guest=windows"),
        "虚拟机档要在那个虚拟根里编：{}",
        guest
    );
    assert!(guest.contains("虚拟根里"), "说清为什么：{}", guest);
}
#[test]
pub(crate) fn exec_entry_missing_refuses_before_any_tier() {
    let missing = vec![crate::capabilities::vm::api::MissingEntry {
        module: "a".to_string(),
        tool: "build".to_string(),
        command: "build/indexer".to_string(),
        buildable: true,
    }];
    // 本机档：入口判据是唯一要说的。
    let host = exec::plan(&ExecSpec::default(), Vec::new(), &missing, false).unwrap_err();
    assert!(
        matches!(host.as_slice(), [Diagnosis::MissingEntry { .. }]),
        "{:?}",
        host
    );
    // 虚拟机档：入口判据同样先于别的判据。
    let vm = exec::plan(&vm_spec(), Vec::new(), &missing, false).unwrap_err();
    assert!(
        matches!(vm.first(), Some(Diagnosis::MissingEntry { .. })),
        "入口判据要排在最前：{:?}",
        vm
    );
    let said = exec::diagnose_text_for(&vm, None);
    assert!(
        said.contains("build/indexer") && said.contains("--build=a"),
        "模块声明了构建方法时，拒绝理由要指到 --build：{}",
        said
    );
    // 没有这份事实时照旧成立：入口判据不牵连别的东西。
    exec::plan(&ExecSpec::default(), Vec::new(), &[], false).expect("本机档照旧可成立");
}

/// 目的：模块入口落进 guest 的本地副本——文件戳不变就复用上次落点，读不到 / 没活 guest / 越界入口都拒绝。
#[test]
pub(crate) fn stage_exec_reuses_the_copy_when_the_file_stamp_is_unchanged() {
    use crate::capabilities::vm::api::{GuestImage, Vm};
    use crate::capabilities::vm::domain::guest::GuestSpec;
    use crate::capabilities::vm::service::VmService;
    use crate::tests::doubles::{FakeEngine, NamedShare};
    use std::sync::Arc;

    let dir = crate::tests::scratch("stage-exec");
    let host = dir.join("entry.sh");
    std::fs::write(&host, b"#!/bin/sh\nexit 0\n").expect("写入口样例");
    let vm = VmService::new(
        Arc::new(FakeEngine::new()),
        vec![Arc::new(NamedShare::new("9p", true))],
    );
    let spec = GuestSpec {
        lease: "w-stage".to_string(),
        image: GuestImage::linux("t", std::path::PathBuf::from("/img/root.qcow2"), None, None),
        mounts: Vec::new(),
        net: false,
        engine_path: None,
        share_path: None,
        shares: Vec::new(),
        seed: None,
        isolated: true,
        cache_disk: None,
    };
    vm.guest_start(&spec).expect("替身 guest 要能起来");

    assert!(
        vm.stage_exec("w-stage", "m", &dir.join("missing.sh"), "entry.sh")
            .is_err(),
        "读不到模块入口要拒绝"
    );
    let dest = vm
        .stage_exec("w-stage", "m", &host, "entry.sh")
        .expect("入口要能落进 guest");
    assert_eq!(
        vm.stage_exec("w-stage", "m", &host, "entry.sh")
            .expect("同一文件戳要复用"),
        dest,
        "文件没变就不重复落盘"
    );
    assert!(
        vm.stage_exec("w-stage", "m", &host, "../escape.sh")
            .is_err(),
        "越界入口要拒绝"
    );
    assert!(
        vm.stage_exec("no-lease", "m", &host, "entry.sh").is_err(),
        "没有活 guest 要拒绝"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
pub(crate) fn package_setup_scripts_apply_prefix_bins_and_system_payloads() {
    use crate::capabilities::packages::api::PlannedPackage;
    use crate::capabilities::vm::domain::exec::ExecPlan;
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
    let plan = exec::plan(&ExecSpec::default(), Vec::new(), &[], false)
        .expect("本机档的 ExecPlan 不装 guest 包，不会因缺包失败");
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
pub(crate) fn guest_base_requires_a_disk_image() {
    let probe = crate::kernel::detail::HostProbeAdapter;
    let dir = crate::tests::scratch("vm-guest-base");
    // 空目录：没有磁盘镜像（会话的可写层没地方叠）。
    let why = classify_base(&dir, &probe).expect_err("没有磁盘镜像不算基础根");
    assert!(why.contains("磁盘镜像"), "理由要说清缺什么：{why}");
    // 只有内核也不算：根必须是磁盘镜像（内核可有可无）。
    std::fs::write(dir.join("vmlinuz"), b"x").expect("写内核");
    assert!(
        classify_base(&dir, &probe).is_err(),
        "只有内核、没有磁盘镜像仍不算基础根"
    );
    // 放磁盘镜像：可用，内核与 initrd 一并认出来。
    std::fs::write(dir.join("rootfs.ext4"), b"img").expect("写磁盘镜像");
    let base = classify_base(&dir, &probe).expect("有磁盘镜像就该可用");
    assert_eq!(base.disk, dir.join("rootfs.ext4"));
    assert_eq!(base.kernel, Some(dir.join("vmlinuz")));
    assert!(base.initrd.is_none(), "没放 initrd 就是没有");
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
