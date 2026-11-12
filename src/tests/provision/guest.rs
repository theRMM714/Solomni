//! 目的：guest 侧 provision 的用例——运行时标记、共享声明、维护顺序与旧镜像拒修。
//! 管：guest 编排与它写下的磁盘种子。
//! 不管：宿主侧安装（见 `service.rs`）、端口实现（见 `ports.rs`）。
//! 联动：被测实现是 `capabilities::provision::service` 的 guest 分支。

use crate::capabilities::provision::domain::Dep;
use crate::capabilities::provision::domain::{deps, under};
use crate::capabilities::provision::guest::*;
use crate::capabilities::vm::api::classify_base;
use std::path::Path;

/// 目的：造一条 guest-runtime 清单条目（字段取自 dependencies.yaml 的真实形状）。
fn dep(id: &str, os: &str, guest: &str, install: &str, probe: &str) -> Dep {
    Dep {
        id: id.to_string(),
        os: os.to_string(),
        arch: "x86_64".to_string(),
        kind: "guest-runtime".to_string(),
        install: install.to_string(),
        probe: probe.to_string(),
        guest: Some(guest.to_string()),
        family: None,
        toolchain: None,
        source: "guest.debian.image".to_string(),
        strip: 0,
        size_mb: 0,
        sha256: String::new(),
        note: String::new(),
    }
}

/// 目的：造一条磁盘根的条目（Windows 形态）。
fn disk_dep() -> Dep {
    dep(
        "vm.guest-runtime.windows",
        "windows",
        "debian-13",
        ".vm/guest/debian-13",
        ".vm/guest/debian-13/.runtimes-ready",
    )
}

/// 目的：在临时产品根里铺一份磁盘根虚拟根 + QEMU 落点。
fn seed_disk_root(root: &Path) -> std::path::PathBuf {
    let base = root.join(".vm").join("guest").join("debian-13");
    std::fs::create_dir_all(&base).expect("base");
    std::fs::write(base.join("rootfs.qcow2"), b"qcow2").expect("disk");
    std::fs::write(
        base.join("guest.yaml"),
        "id: debian-13-genericcloud-amd64\narch: amd64\nagent: qga\nclients: []\n",
    )
    .expect("yaml");
    let qemu = root
        .join(".vm")
        .join("qemu")
        .join("bin")
        .join("qemu-system-x86_64.exe");
    std::fs::create_dir_all(qemu.parent().expect("parent")).expect("qemu dir");
    std::fs::write(&qemu, b"exe").expect("qemu");
    base
}

/// 目的：没有 vm 能力面时如实拒绝（不假装能编排）。
#[test]
fn guest_runtime_refuses_without_vm() {
    let root = crate::tests::scratch("provision-guest-no-vm");
    let err = install_guest_runtime(
        &root,
        &disk_dep(),
        &crate::kernel::detail::host_probe::HostProbeAdapter,
        None,
        false,
        &mut |_| {},
    )
    .expect_err("要拒");
    assert!(err.contains("vm 能力"), "{}", err);
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：虚拟根不在场时如实拒绝——先装 vm.guest.名字 那一条。
#[test]
fn guest_runtime_refuses_without_image() {
    let root = crate::tests::scratch("provision-guest-no-image");
    let vm = crate::tests::doubles::RecordingVm::default();
    let err = install_guest_runtime(
        &root,
        &disk_dep(),
        &crate::kernel::detail::host_probe::HostProbeAdapter,
        Some(&vm),
        false,
        &mut |_| {},
    )
    .expect_err("要拒");
    assert!(err.contains("虚拟根不可用"), "{}", err);
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：磁盘根（Windows 形态）走完首启 + 维护启动，落两个标记，并把客户端写成 cifs。
#[test]
fn guest_runtime_writes_markers_and_declares_cifs() {
    let root = crate::tests::scratch("provision-guest-ok");
    let base = seed_disk_root(&root);
    let vm = crate::tests::doubles::RecordingVm::default();
    install_guest_runtime(
        &root,
        &disk_dep(),
        &crate::kernel::detail::host_probe::HostProbeAdapter,
        Some(&vm),
        false,
        &mut |_| {},
    )
    .expect("编排");
    assert!(base.join(".qga-ready").is_file(), "首启标记要落");
    assert!(base.join(".runtimes-ready").is_file(), "运行包标记要落");
    let y = std::fs::read_to_string(base.join("guest.yaml")).expect("yaml");
    assert!(y.contains("clients: [cifs]"), "{}", y);
    assert!(y.contains("runtimes: [python, node, cc, gcc]"), "{}", y);
    assert!(y.contains("arch: amd64"), "arch 要用画像认的写法：{}", y);
    // 写出的画像必须仍可被识别（arch 写法错了就会在这里炸），并如实声明装好的运行能力。
    let image = crate::capabilities::vm::api::classify_base(
        &base,
        &crate::kernel::detail::HostProbeAdapter,
    )
    .expect("运行期编排写出的画像要仍可识别");
    assert_eq!(
        image.runtimes,
        vec![
            "cc".to_string(),
            "gcc".to_string(),
            "node".to_string(),
            "python".to_string()
        ],
        "镜像要如实声明它装好的运行能力"
    );
    let runs = vm.runs.lock().expect("锁").clone();
    assert!(
        runs.iter().any(|c| c.contains("cifs-utils")),
        "磁盘根的运行包清单要带 cifs-utils：{:?}",
        runs
    );
    assert!(
        runs.iter().any(|c| c.contains("nodejs.org/dist/v24.11.0")),
        "node 要从固定版本官方二进制装：{:?}",
        runs
    );
    assert!(
        runs.iter().any(|c| c.contains("gcc") && c.contains("g++")),
        "C/C++ 要随 apt 装进 guest：{:?}",
        runs
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：已有运行标记但 Windows 画像缺 cifs 的旧资产只拒绝，不自动修补。
#[test]
fn stale_windows_guest_is_refused_instead_of_repaired() {
    let root = crate::tests::scratch("provision-guest-stale");
    let base = seed_disk_root(&root);
    std::fs::write(base.join(".runtimes-ready"), "old\n").expect("marker");
    let vm = crate::tests::doubles::RecordingVm::default();
    let err = install_guest_runtime(
        &root,
        &disk_dep(),
        &crate::kernel::detail::host_probe::HostProbeAdapter,
        Some(&vm),
        false,
        &mut |_| {},
    )
    .expect_err("旧画像要拒");
    assert!(err.contains("不是当前 Windows 基线"), "{}", err);
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：Linux 磁盘根的维护启动必须先配网——磁盘根没有 cloud-init 首启，网卡没人配，
///   apt 与 Node 下载只能解析失败（真机验收就是这么露的）。
#[test]
fn linux_maintenance_brings_up_the_network_first() {
    use std::fs;
    let root = crate::tests::scratch("provision-guest-net");
    let base = root.join(".vm").join("guest").join("debian");
    fs::create_dir_all(&base).expect("建虚拟根");
    fs::write(base.join("rootfs.ext4"), b"img").expect("磁盘镜像");
    fs::write(
        base.join("guest.yaml"),
        "id: t\nos: linux\narch: amd64\nagent: qga\nclients: [9p, virtiofs]\n",
    )
    .expect("画像");
    let qemu = root
        .join(".vm")
        .join("qemu")
        .join("bin")
        .join("qemu-system-x86_64");
    fs::create_dir_all(qemu.parent().expect("父目录")).expect("建 qemu 目录");
    fs::write(&qemu, b"exe").expect("qemu");
    let vm = crate::tests::doubles::RecordingVm::default();
    let d = dep(
        "vm.guest-runtime.linux",
        "linux",
        "debian",
        ".vm/guest/debian",
        ".vm/guest/debian/.runtimes-ready",
    );
    let mut lines: Vec<String> = Vec::new();
    install_guest_runtime(
        &root,
        &d,
        &crate::kernel::detail::host_probe::HostProbeAdapter,
        Some(&vm),
        true,
        &mut |l| lines.push(l.to_string()),
    )
    .expect("编排");
    let runs = vm.runs.lock().expect("锁").clone();
    for want in [
        "刷新 apt 索引",
        "安装基础工具链",
        "安装 Node",
        "自证 python / node / C-C++",
    ] {
        assert!(
            lines.iter().any(|l| l.contains(want)),
            "进度行缺「{}」：{:?}",
            want,
            lines
        );
    }
    assert!(
        runs.iter().filter(|c| c.contains("apt-get update")).count() == 1
            && runs
                .iter()
                .filter(|c| c.contains("apt-get install"))
                .count()
                == 1,
        "索引与安装要分两条命令：{:?}",
        runs
    );
    assert!(
        runs.iter()
            .any(|c| c.contains("systemd-networkd") && c.contains("10.0.2.15")),
        "Linux 维护启动要先配网：{:?}",
        runs
    );
    assert!(
        !runs.iter().any(|c| c.contains("cifs-utils")),
        "Linux 不需要 cifs-utils：{:?}",
        runs
    );
    let _ = fs::remove_dir_all(&root);
}

/// 目的：真机验收（默认跳过）：Linux 虚拟根的运行期编排真起一次 guest，把自证过的能力写进画像。
/// 约束：只在 SOLOMNI_VM_LIVE=1、本机是 Linux、虚拟根与 QEMU 都在场时执行；否则打印 [探针] 如实跳过。
#[test]
fn provision_linux_runtime_proves_runtimes_live() {
    use crate::kernel::ports::HostProbe;
    let live = std::env::var("SOLOMNI_VM_LIVE").as_deref() == Ok("1");
    if !live || std::env::consts::OS != "linux" {
        eprintln!(
            "[探针] 未开启 Linux 运行期编排的真机验收：设 SOLOMNI_VM_LIVE=1（仅 Linux 宿主）后重跑"
        );
        return;
    }
    let root = crate::tests::root_dir();
    let Some(dep) = deps()
        .expect("清单")
        .into_iter()
        .find(|d| d.id == "vm.guest-runtime.linux")
    else {
        eprintln!("[探针] 清单里没有 vm.guest-runtime.linux");
        return;
    };
    let base = under(&root, &dep.install);
    let probe = crate::kernel::detail::HostProbeAdapter;
    let qemu_ok = qemu_path(&root, &dep.os)
        .map(|p| p.is_file())
        .unwrap_or(false);
    if !probe.is_dir(&base) || !qemu_ok {
        eprintln!(
            "[探针] 虚拟根或 QEMU 不在场：先 solomni --provision --ensure=vm.qemu.linux,vm.guest.linux --yes"
        );
        return;
    }
    // 同一个虚拟根不能被两台 guest 同时写：真机用例之间串行（见 doubles::live_guest_lock）。
    let _live = crate::tests::doubles::live_guest_lock();
    let vm = crate::capabilities::vm::service::VmService::new(
        std::sync::Arc::new(crate::capabilities::vm::detail::QemuEngine::new()),
        vec![std::sync::Arc::new(
            crate::capabilities::vm::detail::NoneShare::new(),
        )],
    );
    let mut sink = |l: &str| eprintln!("[探针] {}", l);
    install_guest_runtime(&root, &dep, &probe, Some(&vm), true, &mut sink)
        .expect("真机运行期编排要成功");
    let image = classify_base(&base, &probe).expect("编排后的画像要仍可识别");
    assert!(
        image.runtimes.iter().any(|r| r == "python"),
        "python 必须自证并写进画像：{:?}",
        image.runtimes
    );
    assert!(base.join(".runtimes-ready").is_file(), "运行包标记要落");
}
