//! 核心（应用服务）测试：跨能力编排——核心推荐、核心操作载荷、会话中心与生成驱动。
//! 归属判据：钉的是**应用服务的不变式**（会话中心与编排）；其余按能力落各自的文件。
mod ask_user;
mod proxy;
use super::builders::*;
use super::prelude::*;

/// 造一台"根在给定目录、补齐面报给定清单"的核心（只为验外部工具路径的取法）。
fn conductor_rooted(
    root: &std::path::Path,
    settings: InMemorySettings,
    provision: Arc<dyn crate::capabilities::provision::api::Provision + Send + Sync>,
) -> Conductor {
    let gateway: Arc<dyn ChatGateway + Send + Sync> = Arc::new(gw(BTreeMap::new(), vec![]));
    let llm = test_llm(
        Arc::clone(&gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    Conductor::new(
        root.to_path_buf(),
        registry_service(settings, Arc::clone(&llm)),
        test_history(),
        test_workspace(Arc::new(InMemoryWorkspace::new())),
        test_vm(Arc::new(InMemoryPackages::empty())),
        provision,
        test_modules(Arc::new(VecSource(vec![module_of("a")]))),
        llm,
        test_tools_svc_with(
            Arc::new(SilentRunner),
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        ),
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        crate::tests::doubles::test_host_probe(),
        Arc::new(crate::capabilities::residents::api::NoResidents),
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    )
}

/// 目的：造一台"能起虚拟机会话"的核心：探针按用例声明作答，VM 引擎可控（慢 / 失败）。
/// 约束：与它的两条用例同一道门——本平台 guest 后端尚未真机验收时没有调用方。
#[cfg(not(target_os = "macos"))]
fn conductor_for_vm(
    probe: Arc<dyn crate::kernel::ports::HostProbe + Send + Sync>,
    engine: crate::tests::doubles::FakeEngine,
    modules: Vec<Module>,
) -> Conductor {
    let gateway: Arc<dyn ChatGateway + Send + Sync> = Arc::new(gw(BTreeMap::new(), vec![]));
    let llm = test_llm(
        Arc::clone(&gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    Conductor::new(
        crate::tests::root_dir(),
        registry_service(InMemorySettings::with_tier(Tier::Vm), Arc::clone(&llm)),
        test_history(),
        test_workspace(Arc::new(InMemoryWorkspace::new())),
        crate::tests::doubles::test_vm_with_engine(Arc::new(InMemoryPackages::empty()), engine),
        test_provision(),
        test_modules(Arc::new(VecSource(modules))),
        llm,
        test_tools_svc_with(
            Arc::new(SilentRunner),
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        ),
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        probe,
        Arc::new(crate::capabilities::residents::api::NoResidents),
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    )
}

/// 目的：探针按"机器能力齐 + 虚拟根在场"作答（虚拟根的真文件由 `test_guest` 造）。
/// 约束：与它的两条用例同一道门——本平台 guest 后端尚未真机验收时没有调用方。
#[cfg(not(target_os = "macos"))]
fn vm_capable_probe(
    guest: &str,
) -> std::sync::Arc<dyn crate::kernel::ports::HostProbe + Send + Sync> {
    crate::tests::test_guest(guest);
    crate::tests::doubles::vm_capable_host_probe()
}

/// 目的：创建虚拟机档会话时**核心线程不陪着等 guest**——起 guest（几十秒到几分钟）在调用方线程上跑。
/// 约束：这段判据要虚拟机档真能建起来；本平台 guest 后端尚未真机验收（`VERIFIED_HOSTS` 只有 linux / windows），
///   所以只在已验平台跑——macOS 验收后去掉这个 cfg 并同步那张表。
#[cfg(not(target_os = "macos"))]
#[test]
pub(crate) fn creating_a_vm_session_does_not_block_the_core() {
    use crate::capabilities::conductor::api::ConductorHandle;
    use crate::tests::doubles::FakeEngine;
    const BOOT: std::time::Duration = std::time::Duration::from_millis(400);
    let guest = crate::tests::test_guest("core-free");
    let probe = vm_capable_probe(&guest);
    let engine = FakeEngine {
        boot_delay: Some(BOOT),
        ..Default::default()
    };
    let core = conductor_for_vm(probe, engine.clone(), vec![module_of("a")]);
    let handle = ConductorHandle::spawn(core).expect("起核心");
    let ops = crate::capabilities::conductor::api::Ops::from_handle(&handle);
    let mut spec = work("w-core-free", WorkMode::Single, &["a"]);
    spec.tier = Tier::Vm;
    spec.guest = Some(guest.clone());
    let creator = {
        let ops = ops.clone();
        std::thread::spawn(move || ops.sessions.create_work(spec))
    };
    // 让 boot 真的开始，再问核心一件小事：它必须立刻回，而不是等 400ms。
    std::thread::sleep(std::time::Duration::from_millis(80));
    let t0 = std::time::Instant::now();
    let answered = ops.core.tier_choices();
    let waited = t0.elapsed();
    let created = creator.join().expect("线程没崩").expect("建会话");
    assert!(answered.is_ok(), "核心要能应答");
    assert!(
        waited < BOOT / 2,
        "起 guest 期间核心仍要立刻应答（实测等了 {waited:?}，boot 本身 {BOOT:?}）"
    );
    assert_eq!(created.0.sid, "w-core-free");
    assert!(
        engine
            .calls
            .lock()
            .expect("锁")
            .iter()
            .any(|c| c.starts_with("start:")),
        "guest 真的要起来"
    );
}

/// 目的：guest 起不来时**什么都不留**：会话没建、历史里没有它。
/// 约束：同 `creating_a_vm_session_does_not_block_the_core`——要虚拟机档真能建起来，只在已验平台上判。
#[cfg(not(target_os = "macos"))]
#[test]
pub(crate) fn a_failed_guest_boot_leaves_no_session_behind() {
    use crate::capabilities::conductor::api::ConductorHandle;
    use crate::tests::doubles::FakeEngine;
    let guest = crate::tests::test_guest("boot-fail");
    let probe = vm_capable_probe(&guest);
    let engine = FakeEngine {
        boot_error: Some("起不来（测试注入）".to_string()),
        ..Default::default()
    };
    let core = conductor_for_vm(probe, engine, vec![module_of("a")]);
    let handle = ConductorHandle::spawn(core).expect("起核心");
    let ops = crate::capabilities::conductor::api::Ops::from_handle(&handle);
    let mut spec = work("w-boot-fail", WorkMode::Single, &["a"]);
    spec.tier = Tier::Vm;
    spec.guest = Some(guest);
    let err = ops
        .sessions
        .create_work(spec)
        .expect_err("起不来要如实拒绝");
    assert!(err.contains("起不来"), "{}", err);
    assert!(ops.history.open("w-boot-fail").is_err(), "会话记录不该留下");
    assert!(
        ops.history.list().expect("历史").is_empty(),
        "历史里不该留下这条会话"
    );
    // 占名也要撤掉：同名重试能通过（而不是被"工作名已存在"挡住）。
    let mut again = work("w-boot-fail", WorkMode::Single, &["a"]);
    again.tier = Tier::Host;
    ops.sessions
        .create_work(again)
        .expect("同一名字在本机档上应该建得起来（失败那次没占住名字）");
}
/// 目的：虚拟机档的外部工具路径**每次判据取用时现算**：设置里没登记就采用补齐面报的"已装在项目内"那一个。
/// 约束：补齐作业（Web 或另一个终端）装完就该立刻可用——"装完还得重启"会让用户以为补齐没成功；
///   显式登记永远优先，登记了就不看项目内那份（登记是用户的意思）。
#[test]
pub(crate) fn vm_tool_paths_follow_provisioned_packages_without_restart() {
    let probe = ".vm/qemu/bin/qemu-system-x86_64";
    let root = crate::tests::scratch("vm-tool-fallback");
    let exe = root
        .join(".vm")
        .join("qemu")
        .join("bin")
        .join("qemu-system-x86_64");
    std::fs::create_dir_all(exe.parent().expect("父目录")).expect("建目录");
    std::fs::write(&exe, b"stub").expect("写占位可执行文件");
    let provision = test_provision_with(vec![installed_dep("vm.qemu.linux", probe)]);

    // 没登记：按项目内补齐的那一个判 → 这一项成立，且文案说清是按哪条路找到的。
    let core = conductor_rooted(
        &root,
        InMemorySettings::with_tier(Tier::Host),
        Arc::clone(&provision),
    );
    let choices = core.tier_choices();
    let qemu = choices
        .vm_requirements
        .iter()
        .find(|r| r.id == "qemu")
        .expect("qemu 是一项前置");
    assert!(qemu.met, "没登记时按项目内补齐的那一个判：{}", qemu.detail);
    assert!(
        qemu.detail.contains("项目内"),
        "要说清不是 PATH 找到的：{}",
        qemu.detail
    );

    // 显式登记优先：登记了一个不在场的路径 → 这一项不成立（不去偷偷用项目内那份）。
    let registered = conductor_rooted(
        &root,
        InMemorySettings::with_qemu_path("no/such/qemu"),
        Arc::clone(&provision),
    );
    let qemu = registered
        .tier_choices()
        .vm_requirements
        .into_iter()
        .find(|r| r.id == "qemu")
        .expect("qemu 是一项前置");
    assert!(!qemu.met, "登记优先：{:?}", qemu);
    let _ = std::fs::remove_dir_all(&root);
}

/// **工具轮的思维链随该轮的工具行落档**：流式结束后仍可查看（不再"流式完就丢"）。
/// 行由**唯一那处**构造函数产出（单 agent 与讨论席共用），所以这里直接问它。
#[test]
pub(crate) fn tool_round_reasoning_lands_on_the_tool_line() {
    let tool = crate::capabilities::session::api::ToolCallView {
        speaker: "a".to_string(),
        module: String::new(),
        name: "read".to_string(),
        ok: true,
        args: "{}".to_string(),
        output: "内容".to_string(),
        raw: String::new(),
        call_id: String::new(),
        reply: 0,
    };
    let round = crate::capabilities::collab::service::round::Round {
        reply: 1,
        // 工具轮没有正文：思维链不能另造一条空回答行，只能挂到工具行上。
        text: String::new(),
        reasoning: "先思考".to_string(),
        text_msgs: Vec::new(),
        tool: Some(crate::capabilities::collab::service::round::ToolRun {
            view: tool,
            msgs: Vec::new(),
        }),
        finish: String::new(),
        error: None,
        verb: None,
        degraded: false,
    };
    let next = std::cell::Cell::new(0u64);
    let lines = crate::capabilities::collab::service::round::build_round_lines(
        "a", &round, false, &next, None,
    );
    assert_eq!(lines.len(), 1, "工具轮没有正文时只出工具行：{lines:?}");
    assert_eq!(
        lines[0].reasoning.as_deref(),
        Some("先思考"),
        "工具轮的思维链要随该行落档：{lines:?}"
    );
}

/// 裁决的**建议由核心 AI 给**（随 plan 那一次调用一起产出，不额外花调用）：
/// 它出现在推的 Decision 事件里（前端卡片上的"建议"就是它）。
#[test]
pub(crate) fn plan_review_carries_the_core_advice() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec!["{\"type\":\"agree\",\"text\":\"同意\"}".to_string()],
    );
    let mut core = core_with(
        vec![module_of("a")],
        gw(
            member,
            vec![
                "{\"type\":\"tool\",\"name\":\"plan\",\"args\":{\"plan\":\"方案：做\",\"advice\":\"我建议开工\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做\",\"objective\":\"做完\",\"assignee\":\"a\",\"deps\":[]}]}}".to_string(),
            ],
        ),
    );
    let sid = core
        .create_work(collab_work("w", &["a"], false, "任务"))
        .unwrap()
        .sid;
    let events = answer_card(&mut core, &sid, OPT_BEGIN, "").unwrap();
    let advice = events.iter().find_map(|e| match e {
        SessionEvent::DecisionCard { card, gate, .. } if gate == "plan_review" => {
            Some(card.message.detail.clone())
        }
        _ => None,
    });
    assert_eq!(
        advice.as_deref(),
        Some("我建议开工"),
        "方案待审的裁决要带上核心 AI 给的建议：{:?}",
        events.iter().map(|e| e.to_json()).collect::<Vec<_>>()
    );
}

#[test]
pub(crate) fn work_upload_conflict_and_safe_name() {
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec!["[]".into()]));
    let sid = core
        .create_work(work("up", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    assert!(core.work_upload(&sid, "x.txt", b"hello", false).unwrap());
    assert!(
        !core.work_upload(&sid, "x.txt", b"again", false).unwrap(),
        "同名不覆盖"
    );
    assert!(
        core.work_upload(&sid, "x.txt", b"again", true).unwrap(),
        "显式覆盖"
    );
    assert!(core
        .work_upload("ghost", "x.txt", b"x", false)
        .unwrap_err()
        .contains("无此会话"));
    assert!(core
        .work_upload(&sid, "../evil.txt", b"x", false)
        .unwrap_err()
        .contains("路径分隔符"));
}

/// 回档主会话 → 各 agent 会话按**同一个回合 id** 同步截断（见 session-model.md 五）。
#[test]
pub(crate) fn rewinding_the_main_session_truncates_agent_sessions_by_turn() {
    // 从事件里读转录行（id / 文本 / 回合 id）。
    fn lines_of(evs: &[serde_json::Value]) -> Vec<(u64, String, u64)> {
        let mut out = Vec::new();
        for ev in evs {
            if let Some(ls) = ev.get("lines").and_then(|l| l.as_array()) {
                for l in ls {
                    if let Ok(v) = serde_json::from_value::<
                        crate::capabilities::session::api::LineView,
                    >(l.clone())
                    {
                        out.push((v.id, v.render(), v.turn));
                    }
                }
            }
        }
        out
    }

    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "{\"type\":\"say\",\"text\":\"我先说\"}".to_string(),
            "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
        ],
    );
    let mut core = core_with(
        vec![module_of("a")],
        gw(
            member,
            vec![
                "{\"type\":\"tool\",\"name\":\"plan\",\"args\":{\"plan\":\"方案\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做\",\"objective\":\"做\",\"assignee\":\"a\",\"deps\":[]}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"verdict\",\"args\":{\"clear\":true,\"why\":\"照他说的开工\"}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"node_verdict\",\"args\":{\"verdicts\":[{\"node\":\"n1-1\",\"ok\":true,\"note\":\"够用\"}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"checklist\",\"args\":{\"items\":[{\"item\":\"做\",\"status\":\"pass\"}]}}".to_string(),
            ],
        ),
    );
    let sid = core
        .create_work(collab_work("w", &["a"], false, "做个东西"))
        .unwrap()
        .sid;
    answer_card(&mut core, &sid, OPT_BEGIN, "").unwrap();
    let child = format!("{}--a", sid);

    // 主会话里第一条**带回合**的发言行（开场那次）——回档就保留到它。
    let (_, main_evs) = core.history_open(&sid).unwrap();
    let (line_id, turn) = lines_of(&main_evs)
        .into_iter()
        .find(|(_, text, t)| text.contains("[a:say]") && *t > 0)
        .map(|(id, _, t)| (id, t))
        .expect("开场该有带回合的发言行");

    let (_, before) = core.history_open(&child).unwrap();
    let max_before = lines_of(&before)
        .iter()
        .map(|(_, _, t)| *t)
        .max()
        .unwrap_or(0);
    assert!(
        max_before > turn,
        "回档前 agent 会话该有更晚的回合：{before:?}"
    );

    core.rewind(
        &sid,
        crate::capabilities::conductor::api::RewindTarget::Delete(line_id + 1),
    )
    .unwrap();

    let (_, after) = core.history_open(&child).unwrap();
    let max_after = lines_of(&after)
        .iter()
        .map(|(_, _, t)| *t)
        .max()
        .unwrap_or(0);
    assert!(
        max_after <= turn,
        "回档后 agent 会话该截到同一回合（小于等于 {turn}，实为 {max_after}）：{after:?}"
    );
}

#[test]
pub(crate) fn repeated_malformed_envelopes_each_get_a_failed_row() {
    // 非法信封**没有次数上限**了：每个都各记一条失败行回注给模型，直到它不再发信封（或用户停）。
    const N: usize = 12; // 远大于任何合理上限：证明没有次数上限
    let broken = broken_tool(&s(&["w", "work", "README.md"]));
    let mut script: Vec<String> = (0..N).map(|_| broken.clone()).collect();
    script.push("到此为止。".to_string());
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), script);
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "一直错", l)).unwrap();
    let rows = transcript_rows(&events);
    assert_eq!(
        rows.iter().filter(|r| r.2).count(),
        N,
        "每次非法信封各记一条 + 末行正文：{:?}",
        rows
    );
    assert!(
        rows.last()
            .map(|r| r.1.contains("到此为止。"))
            .unwrap_or(false),
        "模型给出正文就收尾：{:?}",
        rows
    );
    assert!(
        !rows.iter().any(|r| r.1.contains("\"type\"")),
        "JSON 不进文本：{:?}",
        rows
    );
}

#[test]
pub(crate) fn prose_then_unclosed_tool_envelope_is_malformed_and_keeps_prose() {
    // 正文在前、坏信封在后（未闭合）：也要判 malformed，且正文照常显示、JSON 不上屏。
    let raw = "好的。{\"type\":\"tool\",\"name\":\"write\",\"args\":{\"path\":\"a\"}";
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            raw.to_string(),
            "{\"type\":\"say\",\"text\":\"改好了\"}".to_string(),
        ],
    );
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "写一下", l)).unwrap();
    let rows = transcript_rows(&events);
    assert_eq!(
        rows.iter().filter(|r| r.2).count(),
        1,
        "应有一条 tool 行：{:?}",
        rows
    );
    assert!(
        !rows.iter().any(|r| r.1.contains("\"type\"")),
        "JSON 绝不能上屏：{:?}",
        rows
    );
    assert!(
        rows.iter().any(|r| r.1 == "[a] 好的。"),
        "坏信封之前的正文要照常显示：{:?}",
        rows
    );
    let views = tool_views(&events);
    assert!(!views[0].ok && views[0].name == "write", "{:?}", views[0]);
    assert!(
        views[0].args.contains("\"path\":\"a\""),
        "参数尽力打捞：{:?}",
        views[0].args
    );
    assert!(
        views[0].output.contains("还差 }"),
        "未闭合要说清还差哪个字符：{}",
        views[0].output
    );
    // 历史：assistant(原文) + [工具结果]（模型下一轮能自己改）
    let h = core.single_history(&sid).unwrap();
    assert!(h.iter().any(|m| m.role == "assistant" && m.content == raw));
    assert!(
        h.iter()
            .any(|m| m.role == "user" && m.content.contains("[工具结果] write")),
        "{:?}",
        h
    );
}

#[test]
pub(crate) fn prose_then_tool_envelope_keeps_prose_line_and_rebuilds_identically() {
    // 同一轮里「先写正文再发信封」：正文与思维链要落进文本行，且正文行不得含 JSON；
    // 重建上下文必须与实时历史逐条一致（工具轮的文本行不另推 assistant）。
    let hist = Arc::new(InMemoryHistory::new());
    let io = Arc::new(InMemorySysIo::new());
    let n = s(&["w", "a", "n.txt"]);
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec![
        format!("我先看看这个文件。{{\"type\":\"tool\",\"module\":\"a\",\"name\":\"write\",\"args\":{{\"path\":\"{}\",\"content\":\"hi\"}}}}", n),
        "{\"type\":\"say\",\"text\":\"写好了\"}".to_string(),
    ]);
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(member, vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "记一笔", l)).unwrap();
    let rows = transcript_rows(&events);
    assert_eq!(
        rows.len(),
        4,
        "用户行 / 正文行 / tool 行 / 答复行：{:?}",
        rows
    );
    assert!(
        rows[1].1.contains("我先看看这个文件。"),
        "正文要落进文本行：{:?}",
        rows[1]
    );
    assert!(!rows[1].1.contains('{'), "正文行不得含 JSON：{:?}", rows[1]);
    assert!(
        !rows[1].2 && rows[2].2 && !rows[3].2,
        "tool 行紧随正文行：{:?}",
        rows
    );
    assert!(
        !rows[2].1.contains('{'),
        "tool 行也不含 JSON：{:?}",
        rows[2]
    );
    assert_eq!(
        io.get(&["w", "a", "n.txt"]).as_deref(),
        Some("hi"),
        "工具真的跑了"
    );

    let live_h = core.single_history(&sid).unwrap();
    assert!(
        live_h.iter().any(|m| m.role == "assistant"
            && m.content.contains("我先看看这个文件。")
            && m.content.contains("\"type\":\"tool\"")),
        "这一轮的 assistant 消息 = assistant(raw)，正文与信封都在：{:?}",
        live_h
    );
    assert!(
        live_h
            .iter()
            .any(|m| m.role == "user" && m.content.contains("[工具结果] write")),
        "{:?}",
        live_h
    );

    // 「重启」：同一份落盘历史交给新的 Conductor，重建后必须与实时历史逐条一致。
    drop(core);
    let mut core2 = core_with_all(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    core2
        .rewind(
            &sid,
            crate::capabilities::conductor::api::RewindTarget::Delete(4),
        )
        .unwrap(); // 保留全部 4 行
    let rebuilt = core2.single_history(&sid).expect("重建后应在内存里");
    let key = |h: &[Msg]| {
        h.iter()
            .map(|m| (m.role.clone(), m.content.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        key(&rebuilt),
        key(&live_h),
        "重建上下文必须与实时历史逐条一致"
    );
}

#[test]
pub(crate) fn tool_envelope_after_prose_runs_and_json_never_shows() {
    // 正文之后跟信封：**照常执行**（没有强制收尾），显示文本取信封之外的正文，JSON 绝不进转录。
    let runner = Arc::new(RecordingRunner {
        calls: Mutex::new(Vec::new()),
        out: "ok".into(),
        ok: true,
    });
    const N: usize = 10; // 远大于任何合理上限：证明没有调用次数上限
    let mut script: Vec<String> = (0..N).map(|_| TOOL_CALL.to_string()).collect();
    script.push("到此为止。{\"type\":\"tool\",\"name\":\"grep\",\"args\":{}}".to_string());
    // 脚本替身会**重复最后一条**：末条必须是"不再发起工具调用"的正文，否则就是死循环。
    script.push("收工。".to_string());
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), script);
    let mut mod_a = module_of("a");
    mod_a
        .manifest
        .tools
        .insert("grep".to_string(), decl("python tools/grep.py"));
    let mut core = core_with_runner(
        vec![mod_a],
        gw(member, vec!["[]".into()]),
        Arc::clone(&runner),
    );
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "跑满", l)).unwrap();
    assert_eq!(
        runner.calls.lock().expect("锁").len(),
        N + 1,
        "每次调用都执行（正文之后的那个信封也执行）"
    );
    let rows = transcript_rows(&events);
    assert_eq!(rows.iter().filter(|r| r.2).count(), N + 1, "{:?}", rows);
    // 信封之外的正文如实收录成一条正文行（信封本身走工具行，绝不当正文显示）。
    assert!(
        rows.iter().any(|r| !r.2 && r.1.contains("到此为止。")),
        "信封之外的正文要如实收录：{:?}",
        rows
    );
    let last = rows.last().expect("应有末行");
    assert!(
        last.1.contains("收工。") && !last.1.contains('{') && !last.1.contains("\"type\""),
        "末行是模型的收尾正文且不含 JSON：{:?}",
        last
    );
    assert!(
        !rows.iter().any(|r| r.1.contains("\"type\"")),
        "整条转录都不得出现 JSON：{:?}",
        rows
    );
}

/// 一次回复里的**多个**原生调用：实时历史与重建历史必须逐条一致（含 tool_calls 与 tool_call_id）。
/// 钉住这一条：实时与重建都要**每条调用各推一条回执**，不能实时只推第一条、重建却每条都推。
#[test]
pub(crate) fn native_multi_call_rebuilds_identically_to_live() {
    use crate::capabilities::llm::api::ToolCall;
    let hist = Arc::new(InMemoryHistory::new());
    let io = Arc::new(InMemorySysIo::new());
    io.seed(&["w", "a", "a.txt"], "A1\nA2\n");
    io.seed(&["w", "a", "b.txt"], "B1\nB2\n");
    let a = s(&["w", "a", "a.txt"]);
    let b = s(&["w", "a", "b.txt"]);
    let scripts = || {
        vec![
            NativeStep::Calls(vec![
                ToolCall {
                    id: "c1".to_string(),
                    name: "read".to_string(),
                    args_json: format!("{{\"path\":\"{}\"}}", a),
                },
                ToolCall {
                    id: "c2".to_string(),
                    name: "read".to_string(),
                    args_json: format!("{{\"path\":\"{}\"}}", b),
                },
            ]),
            NativeStep::Text("{\"type\":\"say\",\"text\":\"读完了\"}".to_string()),
        ]
    };
    let mut core = native_core(
        NativeGateway {
            scripts: Mutex::new(vec![scripts()]),
        },
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    core.registry_mut()
        .probe_model_tools("m")
        .expect("探测（把这条通道判成原生）");
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "读两个文件", l)).unwrap();
    let live = core.single_history(&sid).unwrap();

    // 转录行：两条工具行同属一次回复（回复号 = 该回复第一条工具行的 id）。
    let views = tool_views(&events);
    assert_eq!(views.len(), 2, "两个调用两条工具行");
    assert_eq!(
        views[0].reply, views[1].reply,
        "同一回复的两条工具行必须同号"
    );
    assert_eq!(views[0].call_id, "c1", "工具行记下供应商给的调用 id");
    assert_eq!(views[1].call_id, "c2");

    // 「重启」：同一份落盘历史交给新核心，按转录重建上下文——必须与实时逐条一致。
    drop(core);
    let mut core2 = native_core(
        NativeGateway {
            scripts: Mutex::new(Vec::new()),
        },
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    core2.registry_mut().probe_model_tools("m").expect("探测");
    let rows = transcript_rows(&events).len() as u64;
    core2
        .rewind(
            &sid,
            crate::capabilities::conductor::api::RewindTarget::Delete(rows),
        )
        .unwrap();
    let rebuilt = core2.single_history(&sid).unwrap();
    let key = |h: &[Msg]| {
        h.iter()
            .map(|m| {
                (
                    m.role.clone(),
                    m.content.clone(),
                    m.tool_calls.len(),
                    m.tool_call_id.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        key(&rebuilt),
        key(&live),
        "重建上下文必须与实时历史逐条一致（含 tool_calls / tool_call_id）"
    );
    assert_eq!(
        live.iter().filter(|m| !m.tool_calls.is_empty()).count(),
        1,
        "一次回复只推一条助手消息"
    );
}
