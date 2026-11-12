//! 目的：guest.yaml 的声明与镜像能力判定。
//! 管：id/arch/agent/os/客户端声明、默认能力。
//! 不管：就绪检测（`readiness.rs`）。
//! 联动：`RUNTIME_SPEC.md`。

use crate::capabilities::vm::domain::guest::classify_base;

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
