//! 目的：shell 的权限与派发契约测试——scope 派生的围栏、档位决定、runtime roots 与 guest 派发。
//! 管：workspace 拒绝 external、external 档 ask、all_reach 不问、view 不发放、按次 profile 与 guest 超时。
//! 不管：真实进程 / 平台围栏（在 kernel 的 process 测试）与 VM 引擎实现。
//! 联动：systools/tools.yaml 的 shell 声明、tools/domain/shell.rs、collab 的 tool_loop。
use super::super::builders::*;
use super::super::prelude::*;
use crate::capabilities::collab::service::tool_loop::{run_batch, ConfirmGate, ToolConfirm};
use crate::capabilities::permission::api::{Effect, PermissionDefaults, ProfileId};
use crate::capabilities::session::api::{SessionEvent, OPT_TOOL_ALLOW};
use crate::capabilities::tools::api::ToolBook;
use std::sync::atomic::{AtomicBool, AtomicUsize};

fn shell_plan(args: &str) -> Vec<(Option<String>, String, String)> {
    vec![(None, "shell".to_string(), args.to_string())]
}

fn member(runner: Arc<ShellRunner>) -> MemberTools {
    let m = member_with_tools("a", Vec::new(), runner);
    m.tools.expect("工具环境")
}

fn with_profile(mut mt: MemberTools, profile: ProfileId) -> MemberTools {
    let defaults = PermissionDefaults {
        profile,
        ..Default::default()
    };
    mt.sandbox.permissions = defaults.to_permission_set();
    mt.fence = crate::kernel::api::FenceSpec::for_session(&mt.sandbox, &[], false);
    mt
}

/// 目的：sandbox shell 只派生私有沙箱与工具链只读根。
#[test]
pub(crate) fn sandbox_shell_only_gets_the_private_sandbox_and_runtime_roots() {
    let runner = Arc::new(ShellRunner::new("ok", true));
    let mut mt = member(Arc::clone(&runner));
    mt.runtime_roots = vec![abs(&["tc"])];
    let slot = Arc::new(crate::capabilities::session::api::ShellSlot::default());
    mt.shell_live = Some(Arc::clone(&slot));
    let cancel = AtomicBool::new(false);
    let mut sink = |_: SessionEvent| {};
    let done = run_batch(
        &mut mt,
        &shell_plan(r#"{"command":"echo hi","timeout_secs":5}"#),
        None,
        None,
        &cancel,
        &mut sink,
    );
    assert!(done[0].1.ok, "{}", done[0].1.output);
    let calls = runner.calls.lock().expect("锁");
    assert_eq!(calls.len(), 1);
    let one = &calls[0];
    assert_eq!(one.script, "echo hi");
    assert_eq!(one.timeout, Duration::from_secs(5));
    assert_eq!(
        (one.fence.agent.as_str(), one.fence.lease.as_str()),
        (mt.sandbox.agent.as_str(), mt.sandbox.session.as_str()),
        "容器身份 = 会话 + agent + 可达集合（不再按次重派）"
    );
    assert_eq!(one.fence.cwd, mt.sandbox.private, "cwd 固定私有沙箱");
    assert_eq!(one.fence.rw, vec![mt.sandbox.private.clone()]);
    assert!(
        one.fence.ro_tree.contains(&abs(&["tc"])),
        "运行根要只读放行"
    );
    assert!(
        !one.fence.ro_tree.contains(&mt.sandbox.shared),
        "不碰共享区"
    );
    assert!(!one.fence.net, "sandbox 不联网");
    assert!(!one.cancel_set);
    assert!(one.path.is_none(), "零模块 agent 没有运行包 PATH");
    let live = slot.get().expect("状态栏快照");
    assert_eq!(live.command, "echo hi");
    assert_eq!(live.scope, "sandbox");
    assert!(live.finished_at.is_some(), "跑完要写结束时间");
    assert_eq!(live.ok, Some(true));
    assert!(live.output.contains("ok"), "回执预览要进快照");
}

/// 目的：workspace 档拒绝 external 且不弹卡、不落进程。
#[test]
pub(crate) fn workspace_rejects_external_without_asking_or_running() {
    let runner = Arc::new(ShellRunner::new("ok", true));
    let mut mt = member(Arc::clone(&runner));
    let cancel = AtomicBool::new(false);
    let mut sink = |_: SessionEvent| {};
    let done = run_batch(
        &mut mt,
        &shell_plan(r#"{"command":"cat ../work/x","scope":"external"}"#),
        None,
        None,
        &cancel,
        &mut sink,
    );
    assert!(!done[0].1.ok);
    assert!(
        done[0].1.output.contains("可达范围"),
        "{}",
        done[0].1.output
    );
    assert!(
        runner.calls.lock().expect("锁").is_empty(),
        "越界请求不落进程"
    );
}

/// 目的：external 档每命令 ask、all_reach 不问且围栏含四项。
#[test]
pub(crate) fn external_profile_asks_and_all_reach_does_not() {
    let runner = Arc::new(ShellRunner::new("ok", true));
    let mut mt = with_profile(member(Arc::clone(&runner)), ProfileId::External);
    let host = abs(&["host-read"]);
    mt.fence =
        crate::kernel::api::FenceSpec::for_session(&mt.sandbox, std::slice::from_ref(&host), true);
    let cancel = AtomicBool::new(false);
    let asked = Arc::new(AtomicUsize::new(0));
    let marked = Arc::clone(&asked);
    let mut confirm = |req: &ToolConfirm, _s: &mut dyn FnMut(SessionEvent)| -> String {
        assert_eq!(req.tool, "shell");
        marked.fetch_add(1, Ordering::Relaxed);
        OPT_TOOL_ALLOW.to_string()
    };
    let mut gate = ConfirmGate {
        allow_turn: Arc::new(AtomicBool::new(false)),
        confirm: &mut confirm,
    };
    let mut sink = |_: SessionEvent| {};
    let done = run_batch(
        &mut mt,
        &shell_plan(r#"{"command":"cat shared/x","scope":"external"}"#),
        Some(&mut gate),
        None,
        &cancel,
        &mut sink,
    );
    assert!(done[0].1.ok, "{}", done[0].1.output);
    assert_eq!(asked.load(Ordering::Relaxed), 1, "external 档每命令 ask");
    let calls = runner.calls.lock().expect("锁");
    assert_eq!(calls.len(), 1);
    let one = &calls[0];
    assert_eq!(
        (one.fence.agent.as_str(), one.fence.lease.as_str()),
        (mt.sandbox.agent.as_str(), mt.sandbox.session.as_str()),
        "external 与 sandbox 共用会话身份，根集合不同才分 SID"
    );
    assert!(
        one.fence.ro_tree.contains(&mt.sandbox.shared),
        "共享区只读可达"
    );
    assert!(one.fence.ro.contains(&host), "主机只读根可达");
    assert!(one.fence.net, "external 加网络（全局开关在测试里为真）");

    // 全可达：不问，直接跑。
    drop(calls);
    let runner2 = Arc::new(ShellRunner::new("ok", true));
    let mut all = with_profile(member(Arc::clone(&runner2)), ProfileId::AllReach);
    all.fence = crate::kernel::api::FenceSpec::for_session(&all.sandbox, &[], false);
    let mut sink2 = |_: SessionEvent| {};
    let done2 = run_batch(
        &mut all,
        &shell_plan(r#"{"command":"cat shared/x","scope":"external"}"#),
        None,
        None,
        &cancel,
        &mut sink2,
    );
    assert!(done2[0].1.ok, "{}", done2[0].1.output);
    assert_eq!(runner2.calls.lock().expect("锁").len(), 1);
}

/// 目的：模块工具的容器身份归会话（lease = 会话 id），调用结束**不**做按次释放（授权按会话持有）。
#[test]
pub(crate) fn module_tools_hold_the_session_lease_and_do_not_release_per_call() {
    let runner = Arc::new(RecordingRunner::new("ok", true));
    let fence = Arc::new(crate::tests::doubles::RecordingFence::new());
    let mut m = member_with_tools("m0", Vec::new(), Arc::clone(&runner));
    let mut mt = m.tools.take().expect("工具环境");
    let runner_dyn: Arc<dyn crate::kernel::ports::ProcessRunner> = runner.clone();
    let fence_dyn: Arc<dyn crate::kernel::ports::FenceHost + Send + Sync> = fence.clone();
    mt.tools = crate::tests::doubles::test_tools_svc_with(
        runner_dyn,
        Arc::new(crate::tests::doubles::InMemorySysIo::new()),
        fence_dyn,
    );
    let cancel = AtomicBool::new(false);
    let plan = |_: &str| vec![(Some("m0".to_string()), "grep".to_string(), "{}".to_string())];
    let mut sink = |_: SessionEvent| {};
    for _ in 0..2 {
        let done = run_batch(&mut mt, &plan("a"), None, None, &cancel, &mut sink);
        assert!(done[0].1.ok, "{}", done[0].1.output);
    }
    assert!(
        fence.released.lock().expect("锁").is_empty()
            && fence.leases.lock().expect("锁").is_empty(),
        "调用结束不做按次释放：授权按会话持有"
    );
    assert!(
        fence.lease_releases.lock().expect("锁").is_empty(),
        "调用结束也不按会话撤（那发生在会话改配/删除时）"
    );
}

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

/// 目的：VM 档 shell 走 guest 并按次传 timeout / head+tail。
#[test]
pub(crate) fn guest_shell_goes_through_the_guest_helper_with_per_call_timeout() {
    let runner = Arc::new(ShellRunner::new("host", true));
    let mut mt = member(Arc::clone(&runner));
    mt.tier = crate::kernel::api::Tier::Vm;
    mt.guest_lease = "lease".to_string();
    let vm = Arc::new(crate::tests::doubles::RecordingVm::default());
    mt.vm = vm.clone();
    let cancel = AtomicBool::new(false);
    let mut sink = |_: SessionEvent| {};
    let done = run_batch(
        &mut mt,
        &shell_plan(r#"{"command":"echo guest","timeout_secs":7}"#),
        None,
        None,
        &cancel,
        &mut sink,
    );
    assert!(done[0].1.ok);
    assert!(
        runner.calls.lock().expect("锁").is_empty(),
        "VM 档不走宿主 runner"
    );
    let runs = vm.fenced_runs.lock().expect("锁");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].1, 7, "按次 timeout 要送进 guest");
    assert_eq!(
        runs[0].2,
        crate::capabilities::vm::domain::guest::OutputLimit::HeadAndTail
    );
    assert!(runs[0].0.contains("echo guest"), "{}", runs[0].0);
    assert!(
        runs[0].0.contains("[shell] 退出码"),
        "guest 脚本要带回退出码：{}",
        runs[0].0
    );
}

/// 目的：基础工具链根只进宿主成员工具面。
#[test]
pub(crate) fn conductor_base_roots_are_only_put_on_host_member_tools() {
    let core = core_with(Vec::new(), gw(BTreeMap::new(), vec!["[]".into()]))
        .with_base_roots(vec![abs(&["base"])]);
    let sb = test_sandbox("a", &[]);
    let plan = crate::capabilities::packages::api::PackagePlan::default();
    let host = core.tools_env(
        &[],
        &sb,
        BTreeMap::new(),
        false,
        crate::capabilities::llm::api::ToolMode::Envelope,
        "solo",
        crate::kernel::api::Tier::Host,
        String::new(),
        &plan,
    );
    assert!(
        host.runtime_roots.contains(&abs(&["base"])),
        "基础工具链根要进宿主 shell 的只读根"
    );
    let vm = core.tools_env(
        &[],
        &sb,
        BTreeMap::new(),
        false,
        crate::capabilities::llm::api::ToolMode::Envelope,
        "solo",
        crate::kernel::api::Tier::Vm,
        "guest".to_string(),
        &plan,
    );
    assert!(
        !vm.runtime_roots.contains(&abs(&["base"])),
        "宿主基础工具链根不能进 guest 围栏"
    );
}
