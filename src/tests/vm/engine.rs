//! 目的：QEMU 引擎的启动参数、磁盘与加速器装配。
//! 管：引擎参数、磁盘/内核组合、租约钩子、内部 none 档、复制后端。
//! 不管：入口判据（`entry.rs`）、共享候选（`share.rs`）。
//! 联动：`src/capabilities/vm/detail/qemu.rs` 与 `docs/vm/README.md`。

use super::super::prelude::*;
use crate::capabilities::vm::api::Vm;
use crate::capabilities::vm::domain::guest::{
    classify_base, GuestImage, GuestMount, GuestRun, GuestSpec,
};
use crate::capabilities::vm::ports::{VmEngine};

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
        cache_disk: None,
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
        cache_disk: None,
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
        cache_disk: None,
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
        cache_disk: None,
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

    let vm = crate::entry::assembly::provision_vm_with(Arc::new(QemuDouble(FakeEngine::new())));
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
        cache_disk: None,
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
            cache_disk: None,
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
        cache_disk: None,
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
        cache_disk: None,
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
