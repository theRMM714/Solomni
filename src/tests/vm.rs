//! 目的：虚拟机档能力的用例测试——执行档位与计划、就绪检测与诊断、guest 装配。
//! 管：本能力的不变式（档位计划、入口判据、承载判定、挂载/叠写脚本）。
//! 不管：工作区与沙箱（`src/tests/workspace.rs`）、conductor 的编排。
//! 联动：契约见 docs/vm/README.md 与 RUNTIME_SPEC.md。
use super::builders::*;
use super::prelude::*;
use crate::capabilities::vm::api::Vm;
use crate::capabilities::vm::domain::guest::{
    classify_base, GuestImage, GuestMount, GuestRun, GuestSpec,
};
use crate::capabilities::vm::ports::{ShareBackend, VmEngine};
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

/// 目的：钉住承载判定这一条事实——界面的"能不能选"与创建/编辑的拒绝走同一把尺子。
/// 约束：现在的判据是**逐项清单**：缺哪几项、每项怎么补，都要能读出来。
#[test]
pub(crate) fn vm_tier_readiness_gates_creation_and_editing() {
    // 用测试探测：本机可起虚拟机、平台共享工具在场；不依赖 runner 有没有 /dev/kvm 或 smbd/virtiofsd。
    let probe = crate::tests::doubles::vm_capable_host_probe();
    // 本机档：任何机器上都能承载（没有 guest 前置）。
    let host = exec::tier_readiness(
        &crate::tests::root_dir(),
        &ExecSpec::default(),
        None,
        None,
        probe.as_ref(),
    );
    assert!(host.ready(), "本机档没有前置条件");
    assert!(host.requirements.is_empty(), "本机档不该有虚拟机前置清单");
    assert!(exec::tier_refusal(
        &crate::tests::root_dir(),
        &ExecSpec::default(),
        None,
        None,
        probe.as_ref()
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
        probe.as_ref(),
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
                        probe.as_ref(),
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
    // 宿主事实：a 的 `build/` 下**什么都没有**（其余地方都算在场）——入口判据只看这一处。
    // 谓词不能只比一个字面路径：Windows 上判据会接连看 `.exe` 等候选（见 entry_check_follows_the_target_shell_rules）。
    let absent = abs(&["a", "build"]);
    let missing =
        exec::missing_entries_for(&modules, std::env::consts::OS, &|p| !p.starts_with(&absent));
    let got: Vec<(&str, &str, &str)> = missing
        .iter()
        .map(|m| (m.module.as_str(), m.tool.as_str(), m.command.as_str()))
        .collect();
    assert_eq!(got, vec![("a", "artifact", "build/indexer")]);
}

#[test]
pub(crate) fn entry_check_follows_the_target_shell_rules() {
    let mut m = module_of("a");
    m.manifest.tools.insert(
        "artifact".to_string(),
        crate::capabilities::modules::api::ToolDecl {
            command: "build/indexer build".to_string(),
            desc: String::new(),
            params: None,
            parallel: false,
        },
    );
    // 只存在 `build/indexer.exe`（Windows 上的落法）：跟着壳的解析走就不算缺；按字面判就会误报。
    let only_exe = |p: &std::path::Path| p.ends_with("build/indexer.exe");
    assert!(
        crate::capabilities::vm::domain::exec::missing_entries_for(
            &[m.clone()],
            "windows",
            &only_exe
        )
        .is_empty(),
        "Windows：`build/indexer` 由 cmd 按 PATHEXT 落到 `.exe`，不该判缺"
    );
    assert_eq!(
        crate::capabilities::vm::domain::exec::missing_entries_for(
            &[m.clone()],
            "linux",
            &only_exe
        )
        .len(),
        1,
        "unix 没有扩展名解析：字面不在就是缺"
    );
    // 只有字面那个（unix 的落法）：两种平台都算在场。
    let only_literal = |p: &std::path::Path| p.ends_with("build/indexer");
    assert!(crate::capabilities::vm::domain::exec::missing_entries_for(
        &[m.clone()],
        "linux",
        &only_literal
    )
    .is_empty());
    assert!(crate::capabilities::vm::domain::exec::missing_entries_for(
        &[m],
        "windows",
        &only_literal
    )
    .is_empty());
}

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
        timeout_secs: 30,
        output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
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
            timeout_secs: 30,
            output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
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
    use crate::capabilities::packages::api::PlannedPackage;
    use crate::capabilities::vm::domain::exec::ExecPlan;
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
            std::path::PathBuf::from("/base/rootfs.ext4"),
            Some(std::path::PathBuf::from("/base/vmlinuz")),
            Some(std::path::PathBuf::from("/base/initrd.img")),
        ),
        mounts,
        net: false,
        engine_path: None,
        share_path: None,
        shares: Vec::new(),
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
        args.iter()
            .any(|a| a == "file=/base/rootfs.ext4,if=virtio,format=raw"),
        "根是磁盘镜像（virtio 磁盘）：{:?}",
        args
    );
    assert!(
        !args.iter().any(|a| a.contains("fsbase")),
        "根是磁盘镜像，不该再有 9p 基础根共享：{:?}",
        args
    );
    let accel = args
        .iter()
        .position(|a| a.as_str() == "-machine")
        .map(|i| args[i + 1].as_str());
    assert_eq!(
        accel,
        Some(
            crate::capabilities::vm::detail::qemu::accelerators()
                .expect("本平台有硬件加速（虚拟机档只用硬件加速）")
        ),
        "加速器按平台给（只用硬件加速，不退回软件模拟）"
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
            std::path::PathBuf::from("/base/rootfs.ext4"),
            Some(std::path::PathBuf::from("/base/vmlinuz")),
            None,
        ),
        mounts: Vec::new(),
        net,
        engine_path: None,
        share_path: None,
        shares: Vec::new(),
        seed: None,
        isolated: false,
    };
    let expected = if cfg!(windows) {
        "accel=whpx"
    } else if cfg!(target_os = "macos") {
        "accel=hvf"
    } else if cfg!(target_os = "linux") {
        "accel=kvm"
    } else {
        panic!("本平台没有接入虚拟机档：accelerators() 应当是 None");
    };
    assert_eq!(
        accelerators(),
        Some(expected),
        "加速器表按平台给：只有硬件加速"
    );
    assert!(
        !accelerators().unwrap_or_default().contains("tcg"),
        "不退回软件模拟：加速器参数里永远不出现 tcg"
    );

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

/// 目的：磁盘镜像可以没有内核（靠镜像自带引导）；内核有没有都不影响"根必须是磁盘镜像"。
#[test]
pub(crate) fn disk_base_may_boot_without_a_kernel() {
    let probe = crate::kernel::detail::HostProbeAdapter;
    let dir = crate::tests::scratch("vm-guest-nokernel");
    std::fs::write(dir.join("rootfs.ext4"), b"img").expect("写磁盘镜像");
    let image = classify_base(&dir, &probe).expect("没有内核也能判出来");
    assert!(image.kernel.is_none(), "没有 vmlinuz 就是 None");
    assert_eq!(
        image.disk,
        dir.join("rootfs.ext4"),
        "判出来的根就是那份磁盘镜像"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 目的：没有内核的磁盘镜像不传 -kernel / -append，交给镜像自带引导（固件启动）。
#[test]
pub(crate) fn disk_root_without_kernel_boots_via_firmware() {
    use crate::capabilities::vm::detail::qemu::engine_args;
    let spec = GuestSpec {
        lease: "w".to_string(),
        image: GuestImage::linux(
            "test",
            std::path::PathBuf::from("/img/root.qcow2"),
            None,
            None,
        ),
        mounts: Vec::new(),
        net: false,
        engine_path: None,
        share_path: None,
        shares: Vec::new(),
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
        args.iter()
            .any(|a| a == "file=/img/root.qcow2,if=virtio,format=qcow2"),
        "磁盘根照给：{:?}",
        args
    );
}

/// 目的：根走 virtio 磁盘（三平台通用），不再依赖 9p 的 base 标签。
#[test]
pub(crate) fn disk_root_uses_a_virtio_disk_not_9p() {
    use crate::capabilities::vm::detail::qemu::engine_args;
    let spec = GuestSpec {
        lease: "w".to_string(),
        image: GuestImage::linux(
            "test",
            std::path::PathBuf::from("/img/root.qcow2"),
            Some(std::path::PathBuf::from("/img/vmlinuz")),
            None,
        ),
        mounts: Vec::new(),
        net: false,
        engine_path: None,
        share_path: None,
        shares: Vec::new(),
        seed: None,
        isolated: false,
    };
    let args = engine_args(&spec, "opts", false, false, None, &[]);
    assert!(
        args.iter()
            .any(|a| a == "file=/img/root.qcow2,if=virtio,format=qcow2"),
        "根要接成 virtio 磁盘：{:?}",
        args
    );
    assert!(
        args.iter().any(|a| a.contains("root=/dev/vda")),
        "内核命令行指向磁盘根：{:?}",
        args
    );
    assert!(
        !args.iter().any(|a| a.contains("fsbase")),
        "不该再有 9p 基础根共享：{:?}",
        args
    );
}

/// 目的：同一租约里 guest 与共享凭据同生命周期；每次执行前后走共享的同步钩子。
#[test]
pub(crate) fn share_syncs_around_each_run_and_releases_on_stop() {
    let engine = Arc::new(crate::tests::doubles::FakeEngine::new());
    let share = Arc::new(crate::tests::doubles::FakeShare::new());
    let vm = crate::capabilities::vm::service::VmService::new(engine.clone(), vec![share.clone()]);
    let spec = GuestSpec {
        lease: "w".to_string(),
        image: GuestImage::linux(
            "test",
            std::path::PathBuf::from("/base/rootfs.ext4"),
            Some(std::path::PathBuf::from("/base/vmlinuz")),
            None,
        ),
        mounts: Vec::new(),
        net: false,
        engine_path: None,
        share_path: None,
        shares: Vec::new(),
        seed: None,
        isolated: false,
    };
    vm.guest_start(&spec).expect("起 guest");
    let run = GuestRun {
        command: "echo hi".to_string(),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: String::new(),
        timeout_secs: 30,
        output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
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

/// 目的：内部 none 档只给供给编排：空计划、无引擎参数、无挂载脚本。
#[test]
pub(crate) fn none_share_plans_no_mounts_or_engine_args() {
    use crate::capabilities::vm::detail::NoneShare;
    use crate::capabilities::vm::ports::ShareBackend;
    let share = NoneShare::new();
    assert!(share.supports("qemu", &GuestImage::linux("t", "/d".into(), None, None)));
    let (plan, _guard) = share.prepare("qemu", &[], None).expect("空计划");
    assert_eq!(plan.kind, "none");
    assert!(plan.engine_args.is_empty());
    assert!(plan.guest_scripts.is_empty());
    assert!(!plan.needs_net && !plan.needs_shared_mem);
    let mounts = vec![crate::capabilities::vm::domain::guest::GuestMount {
        host: std::path::PathBuf::from("/host/work"),
        guest: "/work".to_string(),
        read_only: false,
    }];
    assert!(
        share.prepare("qemu", &mounts, None).is_err(),
        "none 只给无挂载的供给编排"
    );
}

/// 目的：补齐面专用 vm 必须自带内部 `none` 档——运行期编排显式点名它，漏装就会在起 guest 前被判「没有候选」。
/// 约束：替身引擎的 id 伪装成 qemu（`NoneShare` 只承载 qemu），只验候选装配，不启真 VM。
#[test]
pub(crate) fn provision_vm_holds_internal_none_share() {
    use crate::tests::doubles::FakeEngine;

    struct QemuDouble(FakeEngine);
    impl VmEngine for QemuDouble {
        fn id(&self) -> &'static str {
            "qemu"
        }
        fn supports(&self, guest: &GuestImage) -> bool {
            self.0.supports(guest)
        }
        fn boot(
            &self,
            spec: &GuestSpec,
            share: &crate::capabilities::vm::ports::SharePlan,
        ) -> Result<Box<dyn crate::capabilities::vm::ports::GuestVm>, String> {
            self.0.boot(spec, share)
        }
    }

    let vm = crate::provision_vm_with(Arc::new(QemuDouble(FakeEngine::new())));
    let lease = "provision-test";
    let spec = GuestSpec {
        lease: lease.to_string(),
        image: GuestImage::linux(
            "test",
            std::path::PathBuf::from("/base/rootfs.qcow2"),
            None,
            None,
        ),
        mounts: Vec::new(),
        net: true,
        engine_path: None,
        share_path: None,
        shares: vec!["none"],
        seed: None,
        isolated: false,
    };
    vm.guest_start(&spec)
        .expect("补齐面要能用内部 none 档起 guest");
    vm.guest_stop(lease);
}

/// 目的：复制后端不产生引擎参数，只在 guest 里建目录；宿主侧 tar 打包/解包真跑通。
#[test]
pub(crate) fn copy_share_plans_mkdirs_and_tar_round_trips_on_the_host() {
    use crate::capabilities::vm::detail::copy::{host_tar, host_untar, CopyShare};
    use crate::capabilities::vm::ports::ShareBackend;
    let share = CopyShare::new();
    assert!(share.is_emergency(), "copy 是 emergency 档，空候选不自动选");
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

/// 目的：虚拟根里那份磁盘镜像被认出来（三平台通用），就绪项如实说明，引擎参数走 virtio 磁盘。
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
    assert_eq!(
        image.disk,
        dir.join("rootfs.ext4"),
        "判出来的根就是那份磁盘镜像"
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
        "就绪项要说明是磁盘镜像：{:?}",
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
            shares: Vec::new(),
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
            .any(|a| a.starts_with("file=") && a.ends_with(",if=virtio,format=raw")),
        "根要接成 virtio 磁盘：{:?}",
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
        Arc::new(QemuEngine::new()),
        vec![Arc::new(NinePShare::new())],
    );

    // 引擎承载不了：架构不匹配（根是磁盘镜像，三平台通用，只剩架构这一条）。
    let mut wrong_arch =
        GuestImage::linux("t", std::path::PathBuf::from("/img/root.qcow2"), None, None);
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
        shares: Vec::new(),
        seed: None,
        isolated: false,
    };
    let why = svc.guest_start(&spec).expect_err("架构不匹配要被拒绝");
    assert!(why.contains("承载不了"), "{}", why);

    // 共享落不上：引擎能承载，但 guest 没声明 9p 客户端。
    let mut no_nine =
        GuestImage::linux("t", std::path::PathBuf::from("/img/root.qcow2"), None, None);
    no_nine.clients.nine_p = false;
    let spec2 = GuestSpec {
        lease: "w".to_string(),
        image: no_nine,
        mounts: Vec::new(),
        net: false,
        engine_path: None,
        share_path: None,
        shares: Vec::new(),
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
    std::fs::write(dir.join("rootfs.ext4"), b"img").expect("写磁盘镜像");
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
    std::fs::write(dir.join("rootfs.ext4"), b"img").expect("写磁盘镜像");
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
    probe
        .exes
        .push(if cfg!(windows) { "smbd" } else { "virtiofsd" }.to_string());
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
    std::fs::write(dir.join("rootfs.ext4"), b"img").expect("写磁盘镜像");
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
/// 约束：共享档是硬前置；Linux 缺 virtiofsd 仍算可用（9p），Windows 缺 smbd 则判否。copy 不进用户判据。
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
    // 文件在、但不可执行（权限位丢了）：不算可用——否则组合选它，到启动那一步才炸。
    let noexec = std::path::PathBuf::from("/vm/virtiofsd");
    let mut plain = FixedProbe::default();
    plain.files.push(noexec.clone());
    plain.not_executable.push(noexec.clone());
    assert!(
        !share_available("virtiofs", Some(noexec.to_string_lossy().as_ref()), &plain),
        "登记的那个不可执行就不算在场（如实回退，别到启动才失败）"
    );
    let mut exec_ok = FixedProbe::default();
    exec_ok.files.push(noexec.clone());
    assert!(
        share_available(
            "virtiofs",
            Some(noexec.to_string_lossy().as_ref()),
            &exec_ok
        ),
        "能执行才算在场"
    );

    // 登记了但不可执行：就绪文案要说清"不能执行"（与"没找到"是两回事，补法也不同）。
    if cfg!(target_os = "linux") {
        let r = exec::tier_readiness(
            &crate::tests::root_dir(),
            &ExecSpec {
                tier: Tier::Vm,
                guest: None,
                ..Default::default()
            },
            None,
            Some(noexec.to_string_lossy().as_ref()),
            &plain,
        );
        let detail = &r
            .requirements
            .iter()
            .find(|x| x.id == "share")
            .expect("共享档这一项要在")
            .detail;
        assert!(
            detail.contains("不能执行") && detail.contains("9p"),
            "要说清登记的那个不能执行、已回退：{:?}",
            detail
        );
    }
    // 就绪项：共享档是硬前置。Linux 缺 virtiofsd 仍可用 9p；Windows 缺 smbd 直接判否。
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
    if cfg!(windows) {
        assert!(!share.met, "Windows 缺 smbd：共享档必须判否");
        assert!(
            share.detail.contains("smb") && share.detail.contains("smbd"),
            "要说清缺 smbd：{:?}",
            share
        );
        assert!(
            !share.detail.contains("copy") && !share.how.contains("copy"),
            "用户档文案不许再拿 copy 兜底：{:?}",
            share
        );
        assert!(!share.how.is_empty(), "判否要给补齐办法：{:?}", share);
    } else {
        assert!(
            share.met,
            "9p 不需要外部工具，Linux/macOS 仍可用：{:?}",
            share
        );
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

/// 目的：共享档在**起 guest 那一刻**按候选挑：同一个 service 实例下，落得上的那一档被用、同步也只走它。
/// 约束：候选 id 认不出 / 都落不上时如实拒绝，并说出试过哪些——不猜、不静默换档。
#[test]
pub(crate) fn share_is_picked_per_guest_at_start() {
    use crate::tests::doubles::NamedShare;
    let slow = Arc::new(NamedShare::new("slow", false));
    let fast = Arc::new(NamedShare::new("fast", true));
    let vm = crate::capabilities::vm::service::VmService::new(
        Arc::new(crate::tests::doubles::FakeEngine::new()),
        vec![slow.clone(), fast.clone()],
    );
    let spec = |lease: &str, shares: Vec<&'static str>| GuestSpec {
        lease: lease.to_string(),
        image: GuestImage::linux(
            "test",
            std::path::PathBuf::from("/base/rootfs.ext4"),
            Some(std::path::PathBuf::from("/base/vmlinuz")),
            None,
        ),
        mounts: Vec::new(),
        net: false,
        engine_path: None,
        share_path: None,
        shares,
        seed: None,
        isolated: false,
    };
    // 第一档落不上 → 用第二档（顺序就是候选给的顺序）。
    vm.guest_start(&spec("w1", vec!["slow", "fast"]))
        .expect("第二档落得上就该起来");
    assert_eq!(
        fast.calls(),
        vec!["prepare"],
        "实际用的要是 fast：{:?}",
        slow.calls()
    );
    // 另一个租约只给了落不上的一档：如实拒绝，并说出试过哪个。
    let why = vm
        .guest_start(&spec("w2", vec!["slow"]))
        .expect_err("只有落不上的一档就要拒绝");
    assert!(why.contains("slow"), "拒绝理由要说试过哪一档：{}", why);
    // 候选里有本 service 不持有的 id：当作没有候选，而不是 new 一个出来。
    let why2 = vm
        .guest_start(&spec("w3", vec!["nope"]))
        .expect_err("认不出的候选不许猜");
    assert!(why2.contains("没有候选"), "{}", why2);
    // 执行：同步走**这一台**选中的那一档。
    let run = GuestRun {
        command: "echo hi".to_string(),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: String::new(),
        timeout_secs: 30,
        output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
    };
    assert!(vm.guest_run("w1", &run).ok, "执行要成功");
    assert_eq!(fast.calls(), vec!["prepare", "sync_in", "sync_out"]);
    assert!(
        slow.calls().is_empty(),
        "没被选中的档不该参与：{:?}",
        slow.calls()
    );
}

/// 目的：按平台的用户共享链是数据；起 guest 的候选只收"用户档 + 工具在场"的那些，顺序就是优先序。
#[test]
pub(crate) fn share_chain_follows_the_platform_and_skips_absent_tools() {
    use crate::capabilities::vm::domain::exec::{default_combos, share_chain};
    use crate::tests::doubles::FixedProbe;
    let ids = |host: &str| {
        default_combos(host)
            .iter()
            .map(|c| format!("{}/{}", c.engine, c.share))
            .collect::<Vec<_>>()
    };
    assert_eq!(ids("linux")[0], "qemu/virtiofs");
    assert_eq!(ids("macos")[0], "vz/virtiofs");
    assert_eq!(ids("windows")[0], "qemu/smb");

    // 空环境：要外部工具的档不在候选里；9p 不需要外部工具，Windows 没有用户档。
    let bare = FixedProbe::default();
    assert_eq!(share_chain("linux", None, &bare), vec!["9p"]);
    assert_eq!(share_chain("windows", None, &bare), Vec::<&str>::new());
    assert_eq!(
        share_chain("macos", None, &bare),
        vec!["9p"],
        "vz 未实现：macOS 的候选从 qemu 那一档起"
    );

    // 工具在场：性能更高的那一档排到前面（Linux 没 virtiofsd 就退 9p；Windows 只有 smb）。
    let mut with_tools = FixedProbe::default();
    with_tools.exes.push("virtiofsd".to_string());
    with_tools.exes.push("smbd".to_string());
    assert_eq!(
        share_chain("linux", None, &with_tools),
        vec!["virtiofs", "9p"]
    );
    assert_eq!(share_chain("windows", None, &with_tools), vec!["smb"]);
    // 登记了路径就以登记的为准（不是在 PATH 上碰运气）。
    assert_eq!(
        share_chain("linux", Some("no/such/virtiofsd"), &with_tools),
        vec!["9p"],
        "登记了一个不在场的路径：那一档不进候选"
    );
    // copy 永远不会出现在用户链里；它只由进程级隐藏开关显式启用。
    for host in ["linux", "windows", "macos", "unknown"] {
        assert!(
            !share_chain(host, None, &with_tools).contains(&"copy"),
            "copy 不得进用户链：{host}"
        );
    }
}

/// 目的：测试 vm 面同时持有 9p 与 smb 替身——Linux/macOS 与 Windows 的真实候选都能落到已持有的档上。
#[test]
pub(crate) fn test_vm_holds_a_share_for_each_platform_chain() {
    use crate::capabilities::vm::domain::guest::{GuestImage, GuestSpec};
    let svc = crate::tests::doubles::test_vm();
    for share in ["9p", "smb"] {
        let spec = GuestSpec {
            lease: format!("w-{share}"),
            image: GuestImage::linux(
                "test",
                std::path::PathBuf::from("/base/rootfs.ext4"),
                None,
                None,
            ),
            mounts: Vec::new(),
            net: false,
            engine_path: None,
            share_path: None,
            shares: vec![share],
            seed: None,
            isolated: false,
        };
        svc.guest_start(&spec)
            .unwrap_or_else(|why| panic!("测试 vm 面要持有 {share} 替身：{why}"));
    }
}

/// 目的：copy 的开关只认 `copy` 这个值；空/其它值都不算打开。
#[test]
pub(crate) fn copy_switch_only_opens_for_the_copy_value() {
    use crate::capabilities::vm::domain::exec::copy_share_setting;
    assert!(copy_share_setting(Some("copy")));
    assert!(copy_share_setting(Some(" COPY ")));
    assert!(!copy_share_setting(Some("1")));
    assert!(!copy_share_setting(Some("")));
    assert!(!copy_share_setting(None));
}

/// 目的：空候选永远不自动选 emergency；只有显式写出 copy 才允许。
#[test]
pub(crate) fn emergency_share_is_not_picked_without_an_explicit_candidate() {
    use crate::capabilities::vm::domain::guest::{GuestImage, GuestSpec};
    use crate::capabilities::vm::service::VmService;
    use crate::tests::doubles::NamedShare;

    let emergency = Arc::new(NamedShare::new("copy", true).emergency());
    let user = Arc::new(NamedShare::new("9p", true));
    let svc = VmService::new(
        Arc::new(crate::tests::doubles::FakeEngine::new()),
        vec![emergency.clone(), user.clone()],
    );
    let spec = |lease: &str, shares: Vec<&'static str>| GuestSpec {
        lease: lease.to_string(),
        image: GuestImage::linux(
            "test",
            std::path::PathBuf::from("/base/rootfs.ext4"),
            None,
            None,
        ),
        mounts: Vec::new(),
        net: false,
        engine_path: None,
        share_path: None,
        shares,
        seed: None,
        isolated: false,
    };
    svc.guest_start(&spec("w1", Vec::new()))
        .expect("空候选应该只用用户档");
    assert_eq!(user.calls(), vec!["prepare"]);
    assert!(emergency.calls().is_empty(), "copy 不该被自动选中");
    svc.guest_start(&spec("w2", vec!["copy"]))
        .expect("显式候选要允许 emergency");
    assert_eq!(emergency.calls(), vec!["prepare"]);
}

/// 目的：共享承载判据只认引擎 + guest 客户端；copy 只对 Linux guest 成立。
#[test]
pub(crate) fn share_carries_matches_engine_and_guest_clients() {
    use crate::capabilities::vm::domain::exec::share_carries;
    use crate::capabilities::vm::domain::guest::{GuestImage, GuestOs};
    let image = GuestImage::linux(
        "test",
        std::path::PathBuf::from("/base/rootfs.ext4"),
        None,
        None,
    );
    assert!(share_carries("9p", "qemu", &image));
    assert!(share_carries("virtiofs", "qemu", &image));
    assert!(
        !share_carries("smb", "qemu", &image),
        "默认 Debian 没有 cifs"
    );
    assert!(
        share_carries("copy", "qemu", &image),
        "copy 只作应急，但 Linux guest 能落"
    );
    assert!(
        !share_carries("virtiofs", "vz", &image),
        "virtiofs 只落 QEMU"
    );
    assert!(!share_carries("copy", "vz", &image));

    let mut win = image.clone();
    win.os = GuestOs::Windows;
    win.clients.nine_p = false;
    win.clients.virtio_fs = false;
    win.clients.cifs = true;
    assert!(share_carries("smb", "qemu", &win));
    assert!(!share_carries("9p", "qemu", &win));
    assert!(
        !share_carries("copy", "qemu", &win),
        "copy 的脚本是 POSIX，不接 Windows guest"
    );
}

/// 目的：自定义 guest 没声明任何可用客户端时，起 guest 前就拒绝，不等到挂载失败。
#[test]
pub(crate) fn a_guest_without_supported_clients_is_rejected_before_boot() {
    use crate::capabilities::vm::api::tier_readiness;
    use crate::kernel::api::{ExecSpec as VmSpec, Tier};
    let root = crate::tests::scratch("vm-guest-share-readiness");
    let probe = crate::kernel::detail::HostProbeAdapter;
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "amd64"
    };
    let write_guest = |name: &str, clients: &str| {
        let dir = root.join(".vm").join("guest").join(name);
        std::fs::create_dir_all(&dir).expect("建 guest 目录");
        std::fs::write(dir.join("rootfs.ext4"), b"img").expect("写磁盘镜像");
        std::fs::write(
            dir.join("guest.yaml"),
            format!(
                "id: {name}\narch: {arch}\nos: linux\nagent: qga\nclients: [{clients}]\nruntimes: [python]\n"
            ),
        )
        .expect("写 guest.yaml");
    };
    write_guest("good", "9p, virtiofs, cifs");
    write_guest("bad", "");
    // 给一个可执行的共享工具路径：Windows 按文件存在判，Unix 要执行位。
    let tool = root.join("share-tool");
    std::fs::write(&tool, b"stub").expect("写工具占位");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).expect("加执行位");
    }
    let tool = tool.to_string_lossy().into_owned();
    let spec = |guest: &str| VmSpec {
        tier: Tier::Vm,
        guest: Some(guest.to_string()),
        ..Default::default()
    };
    let find = |guest: &str| {
        let readiness = tier_readiness(&root, &spec(guest), None, Some(tool.as_str()), &probe);
        readiness
            .requirements
            .into_iter()
            .find(|r| r.id == "share-guest")
            .expect("要有 guest 级共享判据")
    };
    let good = find("good");
    assert!(good.met, "声明了可用客户端的 guest 要能承载：{:?}", good);
    let bad = find("bad");
    assert!(!bad.met, "没声明客户端的 guest 必须拒绝：{:?}", bad);
    assert!(
        bad.detail.contains("未声明") && bad.how.contains("guest.yaml"),
        "拒绝要说清缺什么、怎么补：{:?}",
        bad
    );
    let _ = std::fs::remove_dir_all(&root);
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
        "Windows 已真机验收（QEMU/WHPX + SMB guestfwd；e2e 的 vm 场景通过）"
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
        engine.clone(),
        vec![Arc::new(crate::tests::doubles::FakeShare::new())],
    );
    let spec = GuestSpec {
        lease: "w".to_string(),
        image: GuestImage::linux(
            "test",
            std::path::PathBuf::from("/base/rootfs.ext4"),
            Some(std::path::PathBuf::from("/base/vmlinuz")),
            None,
        ),
        mounts: Vec::new(),
        net: false,
        engine_path: None,
        share_path: None,
        shares: Vec::new(),
        seed: None,
        isolated: false,
    };
    vm.guest_start(&spec).expect("替身一定能起");
    let run = GuestRun {
        command: "echo hi".to_string(),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: String::new(),
        timeout_secs: 30,
        output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
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
    // 同一个虚拟根不能被两台 guest 同时写：真机用例之间串行（见 doubles::live_guest_lock）。
    let _live = crate::tests::doubles::live_guest_lock();
    let image = classify_base(&base_dir, &probe).expect("虚拟根要可用");
    // 真机引擎级验收：共享要真进 guest（读得到标记），guest 的写要即时落到宿主（实时共享），收尾不留守护进程。
    let share = crate::tests::scratch("vm-live-share");
    std::fs::create_dir_all(&share).expect("建共享目录");
    std::fs::write(share.join("marker.txt"), "SHARE_MARKER").expect("写标记");
    std::fs::create_dir_all(share.join("a")).expect("建 A 沙箱");
    std::fs::write(share.join("a").join("ok.txt"), "A_OK").expect("写 A 文件");
    std::fs::create_dir_all(share.join("b")).expect("建 B 沙箱");
    std::fs::write(share.join("b").join("secret.txt"), "B_SECRET").expect("写 B 文件");
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
        shares: Vec::new(),
        seed: None,
        isolated: true,
    };
    // 共享后端按平台用户链选（与组合根同一份判据）：smb(Windows) > virtiofs > 9p。
    // 没有可用的用户档就如实跳过探针——copy 是开发/应急通道，不在这里兜底。
    let nine = crate::capabilities::vm::detail::NinePShare::new();
    let virtiofs = crate::capabilities::vm::detail::VirtioFsShare::new();
    let smb = crate::capabilities::vm::detail::SmbShare::new();
    let use_smb = std::env::consts::OS == "windows" && spec.image.clients.cifs && smbd.is_some();
    let use_virtiofs = !use_smb && virtiofsd.is_some() && spec.image.clients.virtio_fs;
    let use_nine =
        !use_smb && !use_virtiofs && std::env::consts::OS != "windows" && spec.image.clients.nine_p;
    if !use_smb && !use_virtiofs && !use_nine {
        eprintln!(
            "[探针] 没有可用的用户共享档（Windows 需要 smbd；Linux/macOS 需要 9p 或 virtiofsd）：跳过真机验收"
        );
        return;
    }
    let backend: &dyn crate::capabilities::vm::ports::ShareBackend = if use_smb {
        &smb
    } else if use_virtiofs {
        &virtiofs
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
    // 根永远是磁盘镜像：每租约都该建出一份可写层。
    {
        let overlay = overlay_dir.join("vm-live.qcow2");
        assert!(
            overlay.is_file(),
            "每租约可写层要建出来：{}",
            overlay.display()
        );
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
            timeout_secs: 30,
            output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
    };
    let out = guest.channel().exec(&run);
    // per-agent 围栏：A 只拿到自己的沙箱，B 的沙箱必须被 guest 内 helper 拒绝。
    let access = crate::capabilities::vm::domain::guest::GuestAccess {
        agent: "a".to_string(),
        rw: vec![format!("{guest_dir}/a")],
        ro: Vec::new(),
        ro_tree: Vec::new(),
        private: format!("{guest_dir}/a"),
        cwd: format!("{guest_dir}/a"),
        net: true,
    };
    let fenced = guest.channel().exec_fenced(
        &GuestRun {
            command: format!(
                "cat {guest_dir}/a/ok.txt; echo '---'; cat {guest_dir}/b/secret.txt",
                guest_dir = guest_dir
            ),
            args_json: "{}".to_string(),
            env: Vec::new(),
            cwd: String::new(),
            timeout_secs: 30,
            output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
        },
        &access,
    );
    assert!(
        !fenced.ok,
        "读 B 的沙箱必须被拒绝（能看到就说明围栏没生效）：{}",
        fenced.output
    );
    assert!(fenced.output.contains("A_OK"), "{}", fenced.output);
    assert!(
        fenced.output.contains("Permission denied") || fenced.output.contains("权限"),
        "B 的沙箱要被 Landlock 拒绝：{}",
        fenced.output
    );

    // 取消：sleep 30 起跑后由 host 置位取消旗，helper 收 SIGTERM 杀子进程组，不能等满 timeout。
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flipper = std::sync::Arc::clone(&cancel);
    let thread = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        flipper.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    let started = std::time::Instant::now();
    let cancelled = guest.channel().exec_fenced_cancellable(
        &GuestRun {
            command: "sleep 30; echo SHOULD_NOT_FINISH".to_string(),
            args_json: "{}".to_string(),
            env: Vec::new(),
            cwd: String::new(),
            timeout_secs: 30,
            output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
        },
        &access,
        Some(&cancel),
    );
    let _ = thread.join();
    assert!(!cancelled.ok, "取消后命令不能报成功：{}", cancelled.output);
    assert!(
        cancelled.output.contains("用户停止"),
        "回执要说明用户停止：{}",
        cancelled.output
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(15),
        "取消要及时，不能等满 30 秒：{:?}",
        started.elapsed()
    );

    // 实时共享：挂载式后端**不等任何回拷**，宿主此刻就该看得到 guest 写的内容。
    let written = std::fs::read_to_string(share.join("from-guest.txt")).unwrap_or_default();
    assert!(
        written.contains("VM_LIVE_OK"),
        "{} 要实时可见：宿主读到 {:?}（guest 输出：{}）",
        backend.id(),
        written,
        out.output
    );
    guest.stop();
    // 同上：删租约要把那份可写层也删掉。
    {
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

/// 目的：真机验收（默认跳过）：同一基础镜像**并行起两台** guest，各自叠一份可写层，基础镜像谁都不写。
/// 约束：只在 SOLOMNI_VM_LIVE=1 且 SOLOMNI_VM_QEMU 指向真东西、虚拟根（默认 `debian`）在场时执行；
///   同一个虚拟根不能被两台 guest 同时写：真机用例之间串行（见 doubles::live_guest_lock）。
#[test]
pub(crate) fn vm_live_two_guests_share_one_base_without_writing_it() {
    let live = std::env::var("SOLOMNI_VM_LIVE").as_deref() == Ok("1");
    let qemu = std::env::var("SOLOMNI_VM_QEMU").unwrap_or_default();
    let guest = std::env::var("SOLOMNI_VM_GUEST").unwrap_or_else(|_| "debian".to_string());
    if !live || qemu.trim().is_empty() {
        eprintln!(
            "[探针] 未开启并发可写层的真机验收：设 SOLOMNI_VM_LIVE=1、SOLOMNI_VM_QEMU 后重跑"
        );
        return;
    }
    let probe = crate::kernel::detail::HostProbeAdapter;
    let base_dir =
        match crate::capabilities::vm::api::resolve_guest(&crate::tests::root_dir(), &guest) {
            Ok(d) if probe.is_dir(&d) => d,
            _ => {
                eprintln!(
                    "[探针] 本机没有虚拟根「{}」：先 solomni --provision --ensure --yes",
                    guest
                );
                return;
            }
        };
    let _live = crate::tests::doubles::live_guest_lock();
    let image = classify_base(&base_dir, &probe).expect("虚拟根要可用");
    let base_mtime = std::fs::metadata(&image.disk)
        .and_then(|m| m.modified())
        .expect("读基础镜像 mtime");
    let overlay_dir = crate::tests::scratch("vm-live-overlays-two");
    let engine = crate::capabilities::vm::detail::QemuEngine::with_overlay_dir(overlay_dir.clone());
    let backend = crate::capabilities::vm::detail::NoneShare::new();
    let (plan, _guard) = backend
        .prepare("qemu", &[], None)
        .expect("共享计划要能派生");
    let spec = |lease: &str| GuestSpec {
        lease: lease.to_string(),
        image: image.clone(),
        mounts: Vec::new(),
        net: false,
        engine_path: Some(qemu.clone()),
        share_path: None,
        shares: Vec::new(),
        seed: None,
        isolated: true,
    };
    let mut a = engine
        .boot(&spec("vm-live-a"), &plan)
        .expect("起第一台 guest");
    let mut b = engine
        .boot(&spec("vm-live-b"), &plan)
        .expect("起第二台 guest");
    // 两台各自一份可写层：谁也不写基础镜像。
    for lease in ["vm-live-a", "vm-live-b"] {
        let ov = overlay_dir.join(format!("{lease}.qcow2"));
        assert!(ov.is_file(), "每租约可写层要在场：{}", ov.display());
    }
    let run = |tag: &str| GuestRun {
        command: format!("echo {tag}"),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: String::new(),
        timeout_secs: 30,
        output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
    };
    let out_a = a.channel().exec(&run("TWO_A"));
    let out_b = b.channel().exec(&run("TWO_B"));
    assert!(
        out_a.ok && out_a.output.contains("TWO_A"),
        "第一台要能跑：{}",
        out_a.output
    );
    assert!(
        out_b.ok && out_b.output.contains("TWO_B"),
        "第二台要能跑：{}",
        out_b.output
    );
    a.stop();
    b.stop();
    for lease in ["vm-live-a", "vm-live-b"] {
        let ov = overlay_dir.join(format!("{lease}.qcow2"));
        assert!(!ov.is_file(), "删租约要把可写层删掉：{}", ov.display());
    }
    let after = std::fs::metadata(&image.disk)
        .and_then(|m| m.modified())
        .expect("读基础镜像 mtime");
    assert_eq!(base_mtime, after, "两台 guest 跑完基础镜像 mtime 不该变");
    let _ = std::fs::remove_dir_all(&overlay_dir);
}

/// 目的：真机验收（默认跳过）：**不依赖任何共享后端**，只验 guest 内的取消与超时。
/// 约束：原来唯一覆盖取消的 `vm_live_guest_runs_command_inside_guest` 把取消和 SMB 共享绑在一起，
///   共享挂载失败时连取消脚本都跑不到；这条只走 NoneShare + guest 内 /tmp 围栏，
///   取消机制本身不该被共享链路的故障挡住。
/// 约束：只在 SOLOMNI_VM_LIVE=1 且 SOLOMNI_VM_QEMU 指向真东西、虚拟根（默认 `debian`）在场时执行；否则打印 [探针] 如实跳过。
/// 约束：取消回执含「用户停止」且 15 秒内返回、guest 内没有残留的 sleep；`timeout_secs` 到点仍照常收口。
///   取消路径上 helper 自己的 stderr 可能被 QGA 丢掉（时有时无），所以"信号处理器生效"用**无孤儿 + 快速返回**证明；
///   超时路径的 helper stderr 稳定可见，直接断言它。
#[test]
pub(crate) fn vm_live_cancel_kills_the_guest_process_tree() {
    let live = std::env::var("SOLOMNI_VM_LIVE").as_deref() == Ok("1");
    let qemu = std::env::var("SOLOMNI_VM_QEMU").unwrap_or_default();
    let guest = std::env::var("SOLOMNI_VM_GUEST").unwrap_or_else(|_| "debian".to_string());
    if !live || qemu.trim().is_empty() {
        eprintln!("[探针] 未开启取消的真机验收：设 SOLOMNI_VM_LIVE=1、SOLOMNI_VM_QEMU 后重跑（虚拟根默认 debian，可用 SOLOMNI_VM_GUEST 换）");
        return;
    }
    let probe = crate::kernel::detail::HostProbeAdapter;
    let base_dir = match crate::capabilities::vm::api::resolve_guest(
        &crate::tests::root_dir(),
        &guest,
    ) {
        Ok(d) if probe.is_dir(&d) => d,
        _ => {
            eprintln!(
                "[探针] 本机没有虚拟根「{}」：先 solomni --provision --ensure --yes，或用 SOLOMNI_VM_GUEST 指定",
                guest
            );
            return;
        }
    };
    // 同一个虚拟根不能被两台 guest 同时写：真机用例之间串行（见 doubles::live_guest_lock）。
    let _live = crate::tests::doubles::live_guest_lock();
    let image = classify_base(&base_dir, &probe).expect("虚拟根要可用");
    let overlay_dir = crate::tests::scratch("vm-live-cancel-overlays");
    let engine = crate::capabilities::vm::detail::QemuEngine::with_overlay_dir(overlay_dir.clone());
    // 不挂任何共享：这条用例只验取消 / 超时，共享链路的问题归 SMB 挂载那条缺口。
    use crate::capabilities::vm::ports::ShareBackend;
    let backend = crate::capabilities::vm::detail::NoneShare::new();
    let (plan, _guard) = backend
        .prepare("qemu", &[], None)
        .expect("共享计划要能派生");
    let spec = GuestSpec {
        lease: "vm-live-cancel".to_string(),
        image,
        mounts: Vec::new(),
        net: false,
        engine_path: Some(qemu),
        share_path: None,
        shares: Vec::new(),
        seed: None,
        isolated: true,
    };
    let mut guest = engine.boot(&spec, &plan).expect("起 guest");
    // guest 内的允许集：/tmp 读写，其它一切按 Landlock 拒绝。
    let access = crate::capabilities::vm::domain::guest::GuestAccess {
        agent: "cancel".to_string(),
        rw: vec!["/tmp".to_string()],
        ro: Vec::new(),
        ro_tree: Vec::new(),
        private: "/tmp".to_string(),
        cwd: "/tmp".to_string(),
        net: false,
    };
    let run = |cmd: &str, secs: u64| GuestRun {
        command: cmd.to_string(),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: "/tmp".to_string(),
        timeout_secs: secs,
        output_limit: crate::capabilities::vm::domain::guest::OutputLimit::HeadAndTail,
    };

    // 取消：sleep 30 起跑后由 host 置位取消旗，helper 收 SIGTERM 杀子进程组，不能等满 timeout。
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flip = std::sync::Arc::clone(&cancel);
    let thread = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        flip.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    let started = std::time::Instant::now();
    let cancelled = guest.channel().exec_fenced_cancellable(
        &run("sleep 30; echo SHOULD_NOT_FINISH", 30),
        &access,
        Some(&cancel),
    );
    let _ = thread.join();
    let elapsed = started.elapsed();
    eprintln!(
        "[探针] 取消：ok={} 耗时={:?} 输出={:?}",
        cancelled.ok, elapsed, cancelled.output
    );
    assert!(!cancelled.ok, "取消后命令不能报成功：{}", cancelled.output);
    assert!(
        cancelled.output.contains("用户停止"),
        "回执要说明用户停止：{}",
        cancelled.output
    );
    assert!(
        elapsed < std::time::Duration::from_secs(15),
        "取消要及时，不能等满 30 秒：{:?}",
        elapsed
    );
    // 无孤儿：helper 的信号处理器杀掉的是**整棵进程组**；只杀 helper 自己会留下这个 sleep。
    let alive = guest.channel().exec(&run("pgrep -x sleep || true", 10));
    eprintln!("[探针] 取消后残留 sleep：{:?}", alive.output);
    assert!(
        alive.output.trim().is_empty(),
        "取消后不该有残留的 guest 进程：{:?}",
        alive.output
    );

    // 超时：没有取消旗时，timeout_secs 到点由 helper 杀树并给出「超时」。
    let started = std::time::Instant::now();
    let timed_out = guest
        .channel()
        .exec_fenced_cancellable(&run("sleep 30", 2), &access, None);
    let elapsed = started.elapsed();
    eprintln!(
        "[探针] 超时：ok={} 耗时={:?} 输出={:?}",
        timed_out.ok, elapsed, timed_out.output
    );
    assert!(!timed_out.ok, "超时后命令不能报成功：{}", timed_out.output);
    assert!(
        timed_out.output.contains("超时"),
        "回执要说明超时：{}",
        timed_out.output
    );
    assert!(
        timed_out
            .output
            .contains("solomni-guest-fence: 工具进程超时"),
        "helper 的收口行要在回执里：{}",
        timed_out.output
    );
    assert!(
        elapsed < std::time::Duration::from_secs(12),
        "两秒的超时要很快收口：{:?}",
        elapsed
    );

    guest.stop();
    assert!(
        !overlay_dir.join("vm-live-cancel.qcow2").is_file(),
        "删租约要把可写层也删掉：{}",
        overlay_dir.join("vm-live-cancel.qcow2").display()
    );
    let _ = std::fs::remove_dir_all(&overlay_dir);
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
        Arc::new(RvEngine(rv.clone())),
        vec![Arc::new(FakeShare::new())],
    );
    for lease in ["lease-a", "lease-b"] {
        let image = GuestImage::linux("t", PathBuf::from("/x/rootfs.qcow2"), None, None);
        vm.guest_start(&GuestSpec {
            lease: lease.to_string(),
            image,
            mounts: Vec::new(),
            net: false,
            engine_path: None,
            share_path: None,
            shares: Vec::new(),
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
        timeout_secs: 30,
        output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
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
            None,
            "虚拟机档只认 guest 写法，宿主路径不再直通"
        );
    }
}

/// 目的：per-agent 允许集按挂载表翻成 guest 写法（跨方言也一样）。
#[test]
pub(crate) fn guest_access_translates_the_fence_into_guest_paths() {
    use crate::capabilities::vm::domain::guest::{GuestAccess, GuestMount};
    use crate::kernel::api::FenceSpec;
    let mounts = vec![
        GuestMount {
            host: std::path::PathBuf::from("/work"),
            guest: "/guest/work".to_string(),
            read_only: false,
        },
        GuestMount {
            host: std::path::PathBuf::from("/mods"),
            guest: "/guest/mods".to_string(),
            read_only: true,
        },
    ];
    let fence = FenceSpec {
        agent: "a".to_string(),
        lease: "w--a".to_string(),
        rw: vec![std::path::PathBuf::from("/work/a")],
        ro: vec![std::path::PathBuf::from("/work/ro")],
        ro_tree: vec![std::path::PathBuf::from("/mods/m1")],
        private: std::path::PathBuf::from("/work/a"),
        cwd: std::path::PathBuf::from("/mods/m1"),
        net: false,
    };
    let access = GuestAccess::from_fence(
        &fence,
        crate::capabilities::vm::domain::guest::GuestOs::Linux,
        &mounts,
    );
    assert_eq!(access.agent, "a");
    assert_eq!(access.rw, vec!["/guest/work/a".to_string()]);
    assert_eq!(access.ro, vec!["/guest/work/ro".to_string()]);
    assert_eq!(access.ro_tree, vec!["/guest/mods/m1".to_string()]);
    assert_eq!(access.cwd, "/guest/mods/m1");
}

/// 目的：guest 通道没有实现围栏时，必须 fail-closed，不能悄悄按无围栏执行。
#[test]
pub(crate) fn a_channel_without_fencing_refuses_fenced_runs() {
    use crate::capabilities::vm::domain::guest::{GuestAccess, GuestRun};
    use crate::capabilities::vm::ports::GuestChannel;
    struct Bare;
    impl GuestChannel for Bare {
        fn exec(&mut self, _run: &GuestRun) -> crate::kernel::api::ToolOutcome {
            crate::kernel::api::ToolOutcome {
                ok: true,
                output: "bare".to_string(),
            }
        }
    }
    let access = GuestAccess {
        agent: "a".to_string(),
        rw: Vec::new(),
        ro: Vec::new(),
        ro_tree: Vec::new(),
        private: String::new(),
        cwd: String::new(),
        net: true,
    };
    let run = GuestRun {
        command: "true".to_string(),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: String::new(),
        timeout_secs: 30,
        output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
    };
    let out = Bare.exec_fenced(&run, &access);
    assert!(!out.ok, "没实现围栏必须失败");
    assert!(out.output.contains("围栏"), "{}", out.output);
}

/// 目的：助手脚本必须真的走 Landlock + no_new_privs，并把私有沙箱设成 HOME/TMP。
#[test]
pub(crate) fn guest_helper_is_a_fail_closed_landlock_launcher() {
    let script = crate::capabilities::vm::detail::guest_helper::HELPER_SCRIPT;
    assert!(script.contains("landlock_create_ruleset"), "{}", script);
    assert!(script.contains("landlock_restrict_self"), "{}", script);
    assert!(
        script.contains("PR_SET_NO_NEW_PRIVS") || script.contains("prctl(38"),
        "{}",
        script
    );
    assert!(script.contains("TMPDIR"), "{}", script);
    assert!(
        script.contains("env_names"),
        "环境白名单要按模块隐秘字段名注入：{}",
        script
    );
    assert!(
        script.contains("subprocess.Popen") && script.contains("start_new_session"),
        "要用独立进程组启动，才能杀整棵树：{}",
        script
    );
    assert!(
        script.contains("os.killpg")
            && script.contains("timeout_secs")
            && script.contains("policy.get(\"timeout_secs\""),
        "超时要按本次 timeout_secs 杀进程组：{}",
        script
    );
    assert!(
        script.contains("signal.SIGTERM")
            && script.contains("on_term")
            && script.contains("CHILD")
            && script.contains("os.killpg"),
        "host 取消要经 SIGTERM 杀掉子进程组：{}",
        script
    );
    assert!(
        script.contains("/etc/ld.so.cache") && !script.contains("for base in (\n        \"/usr\",\n        \"/bin\",\n        \"/sbin\",\n        \"/lib\",\n        \"/lib64\",\n        \"/etc\","),
        "只读基线要与本机 Linux 对齐，不能整棵 /etc：{}",
        script
    );
    assert_eq!(
        crate::capabilities::vm::detail::guest_helper::HELPER_TIMEOUT_SECS,
        30,
        "guest 超时与本机守门进程同一口径"
    );
    assert!(
        crate::capabilities::vm::detail::guest_helper::HELPER_PATH.contains("solomni-guest-fence"),
        "助手路径要稳定"
    );
}

/// 目的：VmService 的围栏执行要把 agent 与 guest 路径交给通道，并记录启动时装载的运行包。
#[test]
pub(crate) fn vm_service_runs_fenced_and_records_packages() {
    use crate::capabilities::vm::domain::guest::{GuestMount, GuestSpec};
    use crate::capabilities::vm::service::VmService;
    use crate::kernel::api::FenceSpec;
    use std::sync::Arc;
    let engine = Arc::new(crate::tests::doubles::FakeEngine::new());
    let share = Arc::new(crate::tests::doubles::NamedShare::new("9p", true));
    let svc = VmService::new(engine.clone(), vec![share]);
    let spec = GuestSpec {
        lease: "w".to_string(),
        image: GuestImage::linux(
            "test",
            std::path::PathBuf::from("/base/rootfs.ext4"),
            None,
            None,
        ),
        mounts: vec![GuestMount {
            host: std::path::PathBuf::from("/work"),
            guest: "/guest/work".to_string(),
            read_only: false,
        }],
        net: false,
        engine_path: None,
        share_path: None,
        shares: Vec::new(),
        seed: None,
        isolated: false,
    };
    svc.guest_start(&spec).expect("起替身 guest");
    svc.guest_set_packages("w", vec!["python@3.12".to_string()]);
    assert_eq!(
        svc.guest_packages("w"),
        Some(vec!["python@3.12".to_string()])
    );
    let fence = FenceSpec {
        agent: "a".to_string(),
        lease: "w--a".to_string(),
        rw: vec![std::path::PathBuf::from("/work/a")],
        ro: Vec::new(),
        ro_tree: Vec::new(),
        private: std::path::PathBuf::from("/work/a"),
        cwd: std::path::PathBuf::from("/work/a"),
        net: false,
    };
    let out = svc.guest_run_fenced(
        "w",
        &GuestRun {
            command: "echo hi".to_string(),
            args_json: "{}".to_string(),
            env: Vec::new(),
            cwd: "/work/a".to_string(),
            timeout_secs: 30,
            output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
        },
        &fence,
    );
    assert!(out.ok, "{}", out.output);
    let calls = engine.calls.lock().expect("锁").clone();
    assert!(
        calls.iter().any(|c| c.contains("fenced:w:a:/guest/work/a")),
        "围栏执行要带 agent 与 guest 路径：{:?}",
        calls
    );
}

/// 目的：共享 guest 只挂工作树与模块根两个稳定根，新 agent/模块按状态出现，而不是启动时逐项枚举。
#[test]
pub(crate) fn stable_roots_cover_the_work_tree_and_module_tree() {
    use crate::capabilities::workspace::api::{Sandbox, Sandboxes};
    use std::collections::BTreeMap;
    let sb = |agent: &str, module: &str| Sandbox {
        work_name: "w".to_string(),
        session: "w".to_string(),
        agent: agent.to_string(),
        shared: std::path::PathBuf::from("/session/w/work"),
        private: std::path::PathBuf::from("/session/w").join(agent),
        modules: BTreeMap::from([(
            module.to_string(),
            std::path::PathBuf::from("/modules").join(module),
        )]),
        modules_with_userdata: Default::default(),
        permissions: Default::default(),
        path_codec: None,
    };
    let sandboxes = Sandboxes {
        shared: std::path::PathBuf::from("/session/w/work"),
        list: vec![sb("a", "m1"), sb("b", "m2")],
        codec: None,
    };
    let (rw, ro_tree) =
        crate::capabilities::conductor::service::guest_job::stable_roots(&sandboxes);
    assert!(
        rw.contains(&std::path::PathBuf::from("/session/w")),
        "工作树根要整棵挂：{:?}",
        rw
    );
    assert!(
        rw.contains(&std::path::PathBuf::from("/modules")),
        "模块树根要整棵挂：{:?}",
        rw
    );
    assert!(
        !rw.iter().any(|p| p.ends_with("a") || p.ends_with("b")),
        "不再逐 agent 枚举：{:?}",
        rw
    );
    assert!(ro_tree.is_empty(), "模块 userdata 要可写，稳定根进 rw");
}

/// 目的：新子会话只在活 guest 缺包时才重建；包已覆盖就直接加入，不做运行包动态加载。
#[test]
pub(crate) fn package_rebuild_only_happens_when_the_running_guest_is_missing_one() {
    use crate::capabilities::conductor::service::needs_guest_packages_rebuild;
    let want = vec!["python@3.12".to_string()];
    assert!(!needs_guest_packages_rebuild(false, &want, None));
    assert!(!needs_guest_packages_rebuild(
        true,
        &want,
        Some(&["python@3.12".to_string()])
    ));
    assert!(needs_guest_packages_rebuild(
        true,
        &want,
        Some(&["node@22".to_string()])
    ));
    assert!(!needs_guest_packages_rebuild(true, &[], Some(&[])));
    assert!(needs_guest_packages_rebuild(true, &want, None));
}

/// 目的：per-call 断网事实从 FenceSpec 进 GuestAccess（helper 再据此 unshare 网络命名空间）。
#[test]
pub(crate) fn guest_access_carries_the_per_call_net_flag() {
    use crate::capabilities::vm::api::GuestOs;
    use crate::capabilities::vm::domain::guest::GuestAccess;
    use crate::kernel::api::FenceSpec;
    use std::path::PathBuf;

    let base = FenceSpec {
        agent: "a".to_string(),
        lease: "w".to_string(),
        rw: Vec::new(),
        ro: Vec::new(),
        ro_tree: Vec::new(),
        private: PathBuf::new(),
        cwd: PathBuf::new(),
        net: false,
    };
    assert!(!GuestAccess::from_fence(&base, GuestOs::Linux, &[]).net);
    let mut open = base.clone();
    open.net = true;
    assert!(GuestAccess::from_fence(&open, GuestOs::Linux, &[]).net);
}
