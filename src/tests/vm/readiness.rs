//! 目的：承载判定与就绪说明同源——能不能选、缺什么，用一把尺子。
//! 管：tier_readiness、vm_capability、已验平台表。
//! 不管：共享后端在场（`share.rs`）。
//! 联动：`src/units/vm/domain/tier.rs`。

use super::super::prelude::*;

/// 目的：机器能力与会话选型分开——vm_capability 不问 base；tier_readiness（保存/开始）才要求 base。
#[test]
pub(crate) fn vm_capability_is_machine_only_while_session_readiness_needs_base() {
    use crate::tests::doubles::FixedProbe;
    use crate::units::vm::api::{VmInputs, vm_capability};
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
    let tools = crate::units::vm::api::VmTools {
        qemu: crate::units::vm::api::ToolPath::new(Some(&q), None),
        share: crate::units::vm::api::ToolPath::default(),
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
    let spec = ExecSpec {
        tier: Tier::Vm,
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
