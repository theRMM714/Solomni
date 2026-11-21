//! 目的：shell 的权限与派发契约测试——scope 派生的围栏、档位决定、runtime roots 与 guest 派发。
//! 管：workspace 拒绝 external、external 档 ask、all_reach 不问、view 不发放、按次 profile 与 guest 超时。
//! 不管：真实进程 / 平台围栏（在 kernel 的 process 测试）与 VM 引擎实现。
//! 联动：systools/tools.yaml 的 shell 声明、tools/domain/shell.rs、session 的 engine/tools。
use super::super::prelude::*;
use crate::capabilities::permission::api::{Effect, PermissionDefaults, ProfileId};
use crate::capabilities::tools::api::ToolBook;

/// 目的：view 档不发放 shell、workspace 及以上发放。
#[test]
pub(crate) fn view_does_not_offer_shell_but_workspace_does() {
    let book: ToolBook = crate::tests::doubles::test_systools().tools;
    let schema = book.get("shell").expect("shell 声明");
    let offers = |profile: ProfileId| {
        let d = PermissionDefaults {
            profile,
            ..Default::default()
        };
        d.to_permission_set()
            .offers("shell", Effect::Exec, &schema.max_reach(), schema.scope)
    };
    assert!(!offers(ProfileId::View), "仅可查看档不发放 shell");
    assert!(offers(ProfileId::Workspace));
    assert!(offers(ProfileId::External));
    assert!(offers(ProfileId::AllReach));
}

/// 目的：基础工具链根只进宿主成员工具面。
#[test]
pub(crate) fn runtime_base_roots_are_only_put_on_host_member_tools() {
    let core = core_with(Vec::new(), gw(BTreeMap::new(), vec!["[]".into()]))
        .with_base_roots(vec![abs(&["base"])]);
    let sb = test_sandbox("a", &[]);
    let plan = crate::capabilities::catalog::packages::api::PackagePlan::default();
    let host = core.turn.member_face(
        core.registry.as_ref(),
        &[],
        &sb,
        BTreeMap::new(),
        false,
        crate::capabilities::llm::api::ToolMode::Envelope,
        "solo",
        crate::kernel::api::types::Tier::Host,
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
        crate::capabilities::llm::api::ToolMode::Envelope,
        "solo",
        crate::kernel::api::types::Tier::Vm,
        "guest".to_string(),
        &plan,
    );
    assert!(
        !vm.runtime_roots.contains(&abs(&["base"])),
        "宿主基础工具链根不能进 guest 围栏"
    );
}
