//! 目的：shell 的权限与派发契约测试——scope 派生的围栏、档位决定、runtime roots 与 guest 派发。
//! 管：workspace 拒绝 external、external 档 ask、all_reach 不问、view 不发放、按次 profile 与 guest 超时。
//! 不管：真实进程 / 平台围栏（在 kernel 的 process 测试）与 VM 引擎实现。
//! 联动：systools/tools.yaml 的 shell 声明、tools/domain/shell.rs、collab 的 tool_loop。
use super::super::builders::*;
use super::super::prelude::*;
use crate::capabilities::collab::service::tool_loop::{ConfirmGate, ToolConfirm, run_batch};
use crate::capabilities::permission::api::{Effect, PermissionDefaults, ProfileId};
use crate::capabilities::session::api::{OPT_TOOL_ALLOW, SessionEvent};
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
    assert!(
        done[0].3.is_none(),
        "替身回执没有 [shell] 收尾行 = 不算真跑过，事实不发明"
    );
}

/// 目的：shell 回执末尾带 cwd 与本次耗时（模型与回放共用同一行）。
#[test]
fn shell_receipt_carries_cwd_and_elapsed_after_a_real_run() {
    // 真进程的回执由执行层写「[shell] 退出码 N」；替身只回这一行，判据就落在同一条契约上。
    let runner = Arc::new(ShellRunner::new("[shell] 退出码 0", true));
    let mut mt = member(Arc::clone(&runner));
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
    let out = &done[0].1;
    assert!(out.ok, "{}", out.output);
    let (cwd, secs) =
        crate::capabilities::tools::api::cwd_elapsed(&out.output).expect("回执要有落点与耗时");
    assert_eq!(cwd, mt.sandbox.private.display().to_string());
    let facts = done[0].3.as_ref().expect("要带结构化事实");
    assert_eq!(facts.cwd, cwd, "事实与回执行同一个落点");
    assert_eq!(facts.elapsed_secs(), secs, "事实与回执行同一次计时");
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
    assert!(
        !done[0].1.output.contains("[shell] cwd "),
        "没执行的调用不追加落点与耗时：{}",
        done[0].1.output
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
        crate::capabilities::vm::domain::guest::OutputLimit::Shell,
        "shell 要显式标成 Shell 回执，通道才会产出同形状态行"
    );
    assert_eq!(runs[0].0, "echo guest", "脚本原样送进 guest，不再套包装");
}

/// 目的：虚拟机档的超时回执也要有执行层状态行，并由工具层补 cwd 与耗时（与正常回执同形）。
#[test]
fn vm_timeout_receipt_gets_cwd_and_elapsed_trailer() {
    let runner = Arc::new(ShellRunner::new("host", true));
    let mut mt = member(Arc::clone(&runner));
    mt.tier = crate::kernel::api::Tier::Vm;
    mt.guest_lease = "lease".to_string();
    let slot = Arc::new(crate::capabilities::session::api::ShellSlot::default());
    mt.shell_live = Some(Arc::clone(&slot));
    let vm = Arc::new(crate::tests::doubles::RecordingVm::default());
    *vm.fenced_result.lock().expect("锁") = Some(ToolOutcome {
        ok: false,
        output: "[shell] 超时（2 秒）：工具进程与它拉起的整棵进程树已被终止".to_string(),
    });
    mt.vm = vm.clone();
    let cancel = AtomicBool::new(false);
    let mut sink = |_: SessionEvent| {};
    let done = run_batch(
        &mut mt,
        &shell_plan(r#"{"command":"sleep 5","timeout_secs":2}"#),
        None,
        None,
        &cancel,
        &mut sink,
    );
    let out = &done[0].1;
    assert!(!out.ok, "超时不能报成功：{}", out.output);
    assert!(
        out.output.contains("[shell] 超时（2 秒）"),
        "状态行要在回执里：{}",
        out.output
    );
    let (cwd, secs) = crate::capabilities::tools::api::cwd_elapsed(&out.output)
        .expect("超时回执也要有落点与耗时");
    assert_eq!(cwd, mt.sandbox.private.display().to_string());
    assert_eq!(secs, 0, "刚跑完的耗时按墙钟秒");
    assert_eq!(
        crate::capabilities::tools::api::exit_code(&out.output),
        None,
        "超时没有命令退出码"
    );
    assert_eq!(
        crate::capabilities::tools::api::shell_status(&out.output),
        Some(crate::capabilities::tools::domain::shell::ShellStatus::Timeout)
    );
    let live = slot.get().expect("状态栏快照");
    assert_eq!(live.status.as_deref(), Some("timeout"));
    assert_eq!(live.exit_code, None);
    let facts = done[0].3.as_ref().expect("超时也要带结构化事实");
    assert_eq!(facts.status.as_deref(), Some("timeout"));
    assert_eq!(facts.exit_code, None, "超时没有命令退出码");
    assert_eq!(facts.fence_prep_ms, None, "虚拟机档没有本机围栏准备");
    assert_eq!(live.elapsed_secs, Some(facts.elapsed_secs()));
}

/// 目的：虚拟机档回执的 cwd 用本环境（guest）写法，不再露宿主写法。
#[test]
fn vm_shell_receipt_uses_the_environment_path_form() {
    use crate::capabilities::vm::api::PathBridge;
    use crate::capabilities::vm::domain::paths::PathDialect;
    use crate::kernel::ports::PathCodec;
    let bridge = Arc::new(PathBridge::new("windows", PathDialect::Posix));
    let runner = Arc::new(ShellRunner::new("host", true));
    let mut mt = member(Arc::clone(&runner));
    mt.tier = crate::kernel::api::Tier::Vm;
    mt.guest_lease = "lease".to_string();
    mt.sandbox.path_codec = Some(bridge.clone());
    let vm = Arc::new(crate::tests::doubles::RecordingVm::default());
    *vm.fenced_result.lock().expect("锁") = Some(ToolOutcome {
        ok: true,
        output: "[shell] 退出码 0".to_string(),
    });
    mt.vm = vm.clone();
    let cancel = AtomicBool::new(false);
    let mut sink = |_: SessionEvent| {};
    let done = run_batch(
        &mut mt,
        &shell_plan(r#"{"command":"pwd"}"#),
        None,
        None,
        &cancel,
        &mut sink,
    );
    let out = &done[0].1;
    assert!(out.ok, "{}", out.output);
    let (cwd, _) =
        crate::capabilities::tools::api::cwd_elapsed(&out.output).expect("回执要有落点与耗时");
    assert_eq!(cwd, bridge.to_view(&mt.sandbox.private));
    assert!(
        !cwd.contains("\\"),
        "虚拟机档回执的 cwd 不能是宿主写法：{}",
        cwd
    );
    let facts = done[0].3.as_ref().expect("要带结构化事实");
    assert_eq!(
        facts.cwd,
        bridge.to_view(&mt.sandbox.private),
        "事实也用 guest 写法"
    );
    assert_eq!(facts.fence_prep_ms, None);
    assert_eq!(facts.status.as_deref(), Some("exit"));
    assert!(!facts.truncated);
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

/// 目的：这一批里跑过 shell 时，回合末把沙箱里不是 write / edit 落下的改动采纳进共享区，
///   并把回执（提交号 + 路径）挂在那条工具结果后面；只读批次不触发整盘扫描。
#[test]
pub(crate) fn shell_batch_adopts_sandbox_changes_into_the_shared_area() {
    use crate::capabilities::workspace::api::Workspace;
    use crate::capabilities::workspace::detail::{FsWorkStore, FsWorkspace};
    use crate::capabilities::workspace::service::WorkspaceService;

    let root = crate::tests::scratch("tool-shell-adopt");
    let ws = Arc::new(WorkspaceService::new(
        Arc::new(FsWorkspace::new(root.join("session"))),
        Arc::new(FsWorkStore::new()),
    ));
    ws.bind("w", None).expect("绑定");
    ws.prepare("w", &["a".to_string()]).expect("准备");
    let sandbox = root.join("session").join("w").join("a");
    let shared = ws.roots("w", &[]).expect("布局").shared;
    // 模拟 shell 直接写沙箱（不经过 write 工具，因此没有 tool_loop 的自动提交）。
    std::fs::write(sandbox.join("made-by-shell.txt"), b"hi").expect("写沙箱");

    let runner = Arc::new(ShellRunner::new("ok", true));
    let mut mt = member(Arc::clone(&runner));
    mt.sandbox.work_name = "w".to_string();
    mt.sandbox.agent = "a".to_string();
    mt.sandbox.session = "w--a".to_string();
    mt.sandbox.private = sandbox.clone();
    mt.sandbox.shared = shared.clone();
    let ws_dyn: Arc<dyn Workspace + Send + Sync> = ws.clone();
    mt.workspace = Some(ws_dyn);

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
    let out = &done[0].1.output;
    assert!(out.contains("[沙箱改动已提交"), "回执要给出提交号：{}", out);
    assert!(
        out.contains("made-by-shell.txt"),
        "回执要列出被采纳的路径：{}",
        out
    );
    assert_eq!(
        std::fs::read(shared.join("made-by-shell.txt")).unwrap(),
        b"hi",
        "沙箱改动必须真的写进共享区"
    );
    // 第二次同一批：沙箱与主线一致 → 不再有提交回执。
    let done = run_batch(
        &mut mt,
        &shell_plan(r#"{"command":"echo hi","timeout_secs":5}"#),
        None,
        None,
        &cancel,
        &mut sink,
    );
    assert!(done[0].1.ok);
    assert!(
        !done[0].1.output.contains("[沙箱改动已提交"),
        "没有改动就不该再提交：{}",
        done[0].1.output
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：私有沙箱为空（cwd 没有落点）时如实拒绝，不落进程、不发明事实。
#[test]
pub(crate) fn shell_without_a_private_sandbox_refuses_without_running() {
    let runner = Arc::new(ShellRunner::new("ok", true));
    let mut mt = member(Arc::clone(&runner));
    mt.sandbox.private = std::path::PathBuf::new();
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
    assert!(!done[0].1.ok);
    assert!(
        done[0].1.output.contains("没有私有沙箱"),
        "{}",
        done[0].1.output
    );
    assert!(
        runner.calls.lock().expect("锁").is_empty(),
        "没有落点就不该起进程"
    );
    assert!(done[0].3.is_none(), "没执行就不带结构化事实");
}

/// 目的：虚拟机档没有 guest 租约时如实拒绝，不落进程、不发明事实。
#[test]
pub(crate) fn vm_shell_without_a_guest_lease_refuses_without_running() {
    let runner = Arc::new(ShellRunner::new("host", true));
    let mut mt = member(Arc::clone(&runner));
    mt.tier = crate::kernel::api::Tier::Vm;
    assert!(mt.guest_lease.is_empty(), "替身默认没有租约");
    let vm = Arc::new(crate::tests::doubles::RecordingVm::default());
    mt.vm = vm.clone();
    let cancel = AtomicBool::new(false);
    let mut sink = |_: SessionEvent| {};
    let done = run_batch(
        &mut mt,
        &shell_plan(r#"{"command":"echo guest","timeout_secs":5}"#),
        None,
        None,
        &cancel,
        &mut sink,
    );
    assert!(!done[0].1.ok);
    assert!(
        done[0].1.output.contains("没有 guest 租约"),
        "{}",
        done[0].1.output
    );
    assert!(
        runner.calls.lock().expect("锁").is_empty(),
        "VM 档也不该走宿主 runner"
    );
    assert!(
        vm.fenced_runs.lock().expect("锁").is_empty(),
        "没有租约就不该派进 guest"
    );
    assert!(done[0].3.is_none(), "没执行就不带结构化事实");
}

/// 目的：仅可查看档拒绝执行类工具（deny 不弹卡、不落进程）。
#[test]
pub(crate) fn view_profile_denies_shell_without_asking_or_running() {
    let runner = Arc::new(ShellRunner::new("ok", true));
    let mut mt = with_profile(member(Arc::clone(&runner)), ProfileId::View);
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
    assert!(!done[0].1.ok);
    assert!(
        done[0].1.output.contains("不允许这次调用"),
        "{}",
        done[0].1.output
    );
    assert!(
        runner.calls.lock().expect("锁").is_empty(),
        "deny 不该落进程"
    );
    assert!(done[0].3.is_none(), "没执行就不带结构化事实");
}

/// 目的：external 档需要用户确认，但没有可回答的前端时按拒绝处理。
#[test]
pub(crate) fn ask_without_a_confirming_frontend_is_refused() {
    let runner = Arc::new(ShellRunner::new("ok", true));
    let mut mt = with_profile(member(Arc::clone(&runner)), ProfileId::External);
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
        done[0].1.output.contains("需要用户确认"),
        "{}",
        done[0].1.output
    );
    assert!(
        runner.calls.lock().expect("锁").is_empty(),
        "没有前端就不该落进程"
    );
    assert!(done[0].3.is_none(), "没执行就不带结构化事实");
}

/// 目的：闸门已解析过参数，执行入口的单点防御仍要如实拒绝坏参数（不起进程、不发明事实）。
#[test]
pub(crate) fn shell_entry_refuses_unparsable_args() {
    use crate::capabilities::collab::service::tool_loop::{CallGrant, run_shell};
    use crate::capabilities::permission::api::{Decision, Reach, ReachSet};
    let runner = Arc::new(ShellRunner::new("ok", true));
    let mt = member(Arc::clone(&runner));
    let grant = CallGrant {
        reach: ReachSet::from([Reach::Private]),
        decision: Decision::Allow,
    };
    let cancel = AtomicBool::new(false);
    let (out, facts) = run_shell(&mt, "shell", "不是 JSON", None, &grant, &cancel);
    assert!(!out.ok);
    assert!(out.output.contains("参数不是合法 JSON"), "{}", out.output);
    assert!(
        runner.calls.lock().expect("锁").is_empty(),
        "参数坏就不该起进程"
    );
    assert!(facts.is_none(), "没执行就不带结构化事实");
}

/// 目的：装配缺了 shell 声明时，执行入口如实说"工具不可用"，不起进程。
#[test]
pub(crate) fn shell_entry_refuses_when_the_declaration_is_missing() {
    use crate::capabilities::collab::service::tool_loop::{CallGrant, run_shell};
    use crate::capabilities::permission::api::{Decision, Reach, ReachSet};
    let runner = Arc::new(ShellRunner::new("ok", true));
    let mut mt = member(Arc::clone(&runner));
    mt.builtin_tools.remove("shell");
    let grant = CallGrant {
        reach: ReachSet::from([Reach::Private]),
        decision: Decision::Allow,
    };
    let cancel = AtomicBool::new(false);
    let (out, facts) = run_shell(
        &mt,
        "shell",
        r#"{"command":"echo hi"}"#,
        None,
        &grant,
        &cancel,
    );
    assert!(!out.ok);
    assert!(
        runner.calls.lock().expect("锁").is_empty(),
        "没有声明就不该起进程"
    );
    assert!(facts.is_none(), "没执行就不带结构化事实");
}

/// 目的：闸门认不出这次调用（未知工具）时如实拒绝，不起进程。
#[test]
pub(crate) fn unknown_tool_is_refused_before_running() {
    let runner = Arc::new(ShellRunner::new("ok", true));
    let mut mt = member(Arc::clone(&runner));
    // 没有模块可归属：未知工具在闸门就判死（有模块时那属于派发消歧，走另一条路）。
    mt.modules.clear();
    let cancel = AtomicBool::new(false);
    let mut sink = |_: SessionEvent| {};
    let done = run_batch(
        &mut mt,
        &[(None, "no-such-tool".to_string(), "{}".to_string())],
        None,
        None,
        &cancel,
        &mut sink,
    );
    assert!(!done[0].1.ok);
    assert!(
        done[0].1.output.contains("未知工具"),
        "{}",
        done[0].1.output
    );
    assert!(
        runner.calls.lock().expect("锁").is_empty(),
        "未知工具不该落进程"
    );
}
