//! 目的：shell 的工具面与围栏契约——工具面只由席位面决定，档位只管网络开关与围栏 net。
//! 管：两档都发放 shell、runtime roots 只进宿主成员工具面。
//! 不管：真实进程 / 平台围栏（在 kernel 的 process 测试）与 VM 引擎实现。
//! 联动：systools/tools.yaml 的 shell 声明、tools/domain/shell.rs、session 的 engine/tools。
use super::super::prelude::*;
use crate::units::pure::permission::api::{PermissionDefaults, ProfileId};
use crate::units::tools::api::ToolBook;

/// 目的：档位不再裁剪工具面——两档都给 shell，差别只在网络。
#[test]
pub(crate) fn profile_does_not_change_the_tool_face() {
    let book: ToolBook = crate::tests::doubles::test_systools().tools;
    assert!(book.contains_key("shell"), "shell 要在工具书里");
    let face = vec!["shell".to_string()];
    let ids = crate::units::conversation::session::api::face_tools(face, &book);
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
    let plan = crate::units::catalog::packages::api::PackagePlan::default();
    let host = core.turn.member_face(
        core.registry.as_ref(),
        &[],
        &sb,
        BTreeMap::new(),
        false,
        crate::units::llm::api::ToolMode::Envelope,
        "solo",
        crate::units::pure::exec_model::api::Tier::Host,
        String::new(),
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
        crate::units::llm::api::ToolMode::Envelope,
        "solo",
        crate::units::pure::exec_model::api::Tier::Vm,
        "guest".to_string(),
        &plan,
    );
    assert!(
        !vm.runtime_roots.contains(&abs(&["base"])),
        "宿主基础工具链根不能进 guest 围栏"
    );
}
