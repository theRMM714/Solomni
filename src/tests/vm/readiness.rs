//! 目的：承载判定与就绪说明同源——能不能选、缺什么，用一把尺子。
//! 管：tier_readiness、vm_capability、已验平台表。
//! 不管：共享后端在场（`share.rs`）。
//! 联动：`src/units/business/vm/domain/tier.rs`。

use super::super::prelude::*;

/// 目的：机器能力与会话选型分开——vm_capability 不问 base；tier_readiness（保存/开始）才要求 base。
#[test]
pub(crate) fn vm_capability_is_machine_only_while_session_readiness_needs_base() {
    use crate::tests::doubles::FixedProbe;
    use crate::units::business::vm::api::{VmInputs, vm_capability};
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
    let tools = crate::units::business::vm::api::VmTools {
        qemu: crate::units::business::vm::api::ToolPath::new(Some(&q), None),
        share: crate::units::business::vm::api::ToolPath::default(),
    };
    let cap = vm_capability(&VmInputs {
        tools: &tools,
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
    let spec = TargetSpec {
        side: Side::Guest,
        guest: None,
        ..Default::default()
    };
    let session = exec::tier_readiness(&crate::tests::root_dir(), &spec, &tools, &probe);
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

/// 目的：未登记的策略 id 也要如实说缺什么——注册表不把不认识的策略静默当成可用。
/// 约束：这条判据只在注册表这一层，调用方（`work` / `turn` / `system`）不按策略 id 分支。
#[test]
pub(crate) fn an_unregistered_policy_reports_what_is_missing() {
    let policies = crate::tests::doubles::test_policies();
    let machine = policies.machine("no-such-policy");
    assert!(!machine.ready(), "不认识的策略不能判成可用");
    assert!(
        machine
            .requirements
            .iter()
            .any(|r| r.id == "policy" && r.detail.contains("没有这个执行环境策略")),
        "要如实说缺这一项：{:?}",
        machine.requirements
    );
}
