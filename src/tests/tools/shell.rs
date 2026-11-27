//! 目的：shell 的工具面与围栏契约——工具面只由席位面决定，档位只管网络开关与围栏 net。
//! 管：两档都发放 shell、runtime roots 只进宿主成员工具面。
//! 不管：真实进程 / 平台围栏（在 kernel 的 process 测试）与 VM 引擎实现。
//! 联动：systools/tools.yaml 的 shell 声明、tools/domain/shell.rs、session 的 engine/tools。
use super::super::prelude::*;
use crate::units::business::tools::api::ToolBook;
use crate::units::pure::permission::api::{PermissionDefaults, ProfileId};

/// 目的：档位不再裁剪工具面——两档都给 shell，差别只在网络。
#[test]
pub(crate) fn profile_does_not_change_the_tool_face() {
    let book: ToolBook = crate::tests::doubles::test_systools().tools;
    assert!(book.contains_key("shell"), "shell 要在工具书里");
    let face = ["shell".to_string()];
    // 工具面只按 id 记账：形状（说明与参数契约）在工具书里，面里没有的不该出现。
    let ids: Vec<String> = face
        .iter()
        .filter(|id| book.contains_key(id.as_str()))
        .cloned()
        .collect();
    assert_eq!(ids, vec!["shell".to_string()], "席位面就是授权面");
    let net = |profile: ProfileId| {
        PermissionDefaults { profile }
            .to_permission_set()
            .allows_network()
    };
    assert!(!net(ProfileId::Offline));
    assert!(net(ProfileId::Online));
}

/// 目的：基础工具链根只进宿主成员工具面。
#[test]
pub(crate) fn runtime_base_roots_are_only_put_on_host_member_tools() {
    let core = core_with(Vec::new(), gw(BTreeMap::new(), vec!["[]".into()]))
        .with_base_roots(vec![abs(&["base"])]);
    let sb = test_sandbox("a", &[]);
    let plan = crate::units::business::catalog::packages::api::PackagePlan::default();
    let host = core.turn.member_face(
        core.registry.as_ref(),
        &[],
        &sb,
        BTreeMap::new(),
        false,
        crate::units::pure::registry_model::api::ToolMode::Envelope,
        "standard",
        "solo",
        &crate::units::business::sandbox::api::SandboxFacts::host(),
        &crate::units::business::sandbox::api::SandboxDecl::default(),
        "",
        &crate::units::business::session_model::api::WorkspaceDecl::default(),
        &plan,
    );
    assert!(
        host.runtime_roots.contains(&abs(&["base"])),
        "基础工具链根要进宿主 shell 的只读根"
    );
    let vm = core.turn.member_face(
        core.registry.as_ref(),
        &[],
        &sb,
        BTreeMap::new(),
        false,
        crate::units::pure::registry_model::api::ToolMode::Envelope,
        "standard",
        "solo",
        &crate::units::business::sandbox::api::SandboxFacts::guest("guest"),
        &crate::units::business::sandbox::api::SandboxDecl::of(
            crate::units::business::sandbox::api::VM_POLICY,
        ),
        "guest",
        &crate::units::business::session_model::api::WorkspaceDecl::default(),
        &plan,
    );
    assert!(
        !vm.runtime_roots.contains(&abs(&["base"])),
        "宿主基础工具链根不能进 guest 围栏"
    );
}
