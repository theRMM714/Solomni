//! 目的：承载判定与就绪说明同源——能不能选、缺什么，用一把尺子。
//! 管：tier_readiness、vm_capability、已验平台表。
//! 不管：共享后端在场（`share.rs`）。
//! 联动：`src/capabilities/vm/domain/tier.rs`。

use super::super::prelude::*;
use crate::capabilities::vm::api::Vm;
use crate::capabilities::vm::domain::guest::{GuestImage, GuestRun, GuestSpec};

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
    assert!(
        exec::tier_refusal(
            &crate::tests::root_dir(),
            &ExecSpec::default(),
            None,
            None,
            probe.as_ref()
        )
        .is_none()
    );

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

/// 目的：机器能力与会话选型分开——vm_capability 不问 base；tier_readiness（保存/开始）才要求 base。
#[test]
pub(crate) fn vm_capability_is_machine_only_while_session_readiness_needs_base() {
    use crate::capabilities::vm::api::{VmInputs, vm_capability};
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
        cache_disk: None,
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
