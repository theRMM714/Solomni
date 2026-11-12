//! 目的：共享后端的落地判据与候选挑选。
//! 管：9p/smb/virtiofs/copy 的在场、候选顺序、guest 客户端要求。
//! 不管：引擎参数（`engine.rs`）。
//! 联动：`src/capabilities/vm/detail/share_ninep.rs`、`share_smb.rs`、`share_virtiofs.rs`、`share_none.rs`。

use super::super::prelude::*;
use crate::capabilities::vm::api::Vm;
use crate::capabilities::vm::domain::guest::{classify_base, GuestImage, GuestRun, GuestSpec};
use crate::capabilities::vm::ports::ShareBackend;

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
    use crate::capabilities::vm::domain::guest::GuestMount;
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
        cache_disk: None,
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
            cache_disk: None,
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
        cache_disk: None,
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
