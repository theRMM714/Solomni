//! 入站契约（`conductor::api`）的契约测试：命令/事件模型、能力分面、停止语义、panic 隔离。
//! 这一层不碰 HTTP；HTTP 侧（路由目录与逐路由契约）另见本目录的 routes。

use super::doubles::{collab_work, module_of};
use super::{gated_ops, ops_with, single_work, slow_ops};
use crate::capabilities::conductor::api::{
    Acted, Action, AgentInstance, SessionEdit, SessionEvent, WorkMode, WorkSpec,
};
use crate::capabilities::conductor::api::{ConductorHandle, Ops, Output};
use crate::capabilities::workspace::api::Module;
use crate::kernel::api::Tier;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 用内存装配起一个核心手柄（核心从此有自己的线程、自己的状态）。
fn spawn(modules: Vec<Module>, core_script: Vec<&str>) -> ConductorHandle {
    ops_with(modules, core_script).0
}

// ---------- 命令在自己的线程上跑，状态只被它碰 ----------

#[test]
fn commands_run_on_the_core_thread_and_changes_are_visible() {
    let handle = spawn(vec![module_of("a")], Vec::new());
    let ops = Ops::from_handle(&handle);
    assert!(
        ops.registry.settings().expect("读设置").streaming,
        "预置设置读得到"
    );

    ops.registry
        .upsert_provider("p2", "http://x", "sk-1")
        .expect("写供应商");
    let providers = ops.registry.providers().expect("读供应商");
    assert!(
        providers.iter().any(|p| p.id == "p2"),
        "写入要立刻可见：{:?}",
        providers.len()
    );

    assert_eq!(ops.workspace.roster().expect("清单").modules.len(), 1);
    assert!(ops
        .core
        .runtime_report(Tier::Host)
        .expect("能力报告")
        .diagnoses
        .is_empty());
    assert!(
        ops.history.list().expect("历史").is_empty(),
        "内存历史初始为空"
    );
}

#[test]
fn generation_pushes_facts_to_the_event_bus_with_sequence_numbers() {
    let handle = spawn(vec![module_of("a")], Vec::new());
    let ops = Ops::from_handle(&handle);
    let bus = handle.events();
    // 建工作：回包给（会话 + 名单 + 事件台头部），开场事实**只**进事件台。
    let (opened, base) = ops
        .sessions
        .create_work(single_work("w", &["a"]))
        .expect("建会话");
    assert!(base > 0, "开场事实也进事件台");
    assert!(
        !bus.snapshot(Some(&opened.sid), 0).0.is_empty(),
        "建工作的开场事实在事件台上"
    );

    let adv = ops
        .sessions
        .say(&opened.sid, "你好", Output::Final)
        .expect("说一句");
    // 命令回包只给**事件台头部序号**：事实只有一条来路，不再随回包返回。
    assert!(adv.head > base, "回包给的是事件台头部序号");
    let (lines, head, _oldest) = bus.snapshot(Some(&opened.sid), base);
    // 逐轮外送：事件按"一轮一批"进台，所以这里是多批（不攒到整回合结束）。
    assert!(!lines.is_empty(), "生成期间就该有事件进台");
    assert_eq!(head, adv.head, "回包头部 = 事件台头部");
    assert_eq!(
        lines[lines.len() - 1].seq,
        adv.head,
        "最后一批就是头部那一批"
    );
    assert!(
        bus.snapshot(Some(&opened.sid), adv.head).0.is_empty(),
        "游标之后不再重复"
    );
    assert!(
        bus.snapshot(Some("别的会话"), 0).0.is_empty(),
        "按会话取，不串台"
    );
}

/// 核心操作没有会话也**照推**：推荐落在系统会话上（只推不留）。
/// 推是底层收发消息的统一定律（与会话种类无关），留不留才是各会话自己的策略。
#[test]
fn suggest_models_pushes_on_a_system_session_and_leaves_no_trace() {
    let handle = spawn(
        vec![module_of("a")],
        vec![
            "{\"type\":\"tool\",\"name\":\"slate\",\"args\":{\"picks\":[{\"name\":\"甲\",\"modules\":[\"a\"],\"model\":\"m\",\"why\":\"对口\"}]}}",
        ],
    );
    let ops = Ops::from_handle(&handle);
    let agents = ops
        .core
        .suggest_models("做个东西", WorkMode::Collab)
        .expect("核心推荐");
    assert_eq!(agents.len(), 1, "回包只给名单（渲染那一步的契约不变）");
    let (batches, _head, _oldest) = handle.events().snapshot(
        Some(crate::capabilities::conductor::service::SYSTEM_SID_SUGGEST),
        0,
    );
    assert!(
        !batches.is_empty(),
        "核心这一趟的行必须推出来（系统会话也是推的落脚点）"
    );
    let _: Vec<&SessionEvent> = batches.iter().flat_map(|l| l.events.iter()).collect();
    assert!(
        ops.history.list().expect("历史").is_empty(),
        "系统会话只推不留：推荐没有工作区，不该落盘"
    );
    assert!(
        !ops.sessions
            .session_views(&[])
            .expect("会话视图")
            .iter()
            .any(|v| v.sid == crate::capabilities::conductor::service::SYSTEM_SID_SUGGEST),
        "系统会话不进会话列表（前端因此不会为它建标签页）"
    );
}

/// 历史与实时**合流**：盘上转录 + 事件台上"它之外"的尾巴，逐条互补（不重不漏）。
/// 前端因此只按序 append：两个来源的合流只在一处做，刷新后不会整段重复。
#[test]
fn history_merge_is_the_transcript_plus_the_bus_tail_without_repeats() {
    let handle = spawn(
        vec![module_of("a")],
        vec!["{\"type\":\"say\",\"text\":\"第一句\"}"],
    );
    let ops = Ops::from_handle(&handle);
    let (opened, _base) = ops
        .sessions
        .create_work(single_work("w", &["a"]))
        .expect("建会话");
    ops.sessions
        .say(&opened.sid, "你好", Output::Final)
        .expect("说一句");
    let (_meta, transcript) = ops.history.open(&opened.sid).expect("读转录");
    assert!(!transcript.is_empty(), "这一趟有转录");
    let (tail, head) = handle.events().tail_excluding(&opened.sid, &transcript);
    assert!(head > 0, "合流顺带给出头部序号（水位）");
    assert!(!tail.is_empty(), "事件台上还有它之外的实时行（短暂事件）");
    // **过时的短暂事件不许进尾巴**：开跑那条运行态（agent 有名字）已经被盘上的定稿行取代，
    // 它要是跟着尾巴下去，前端会先画定稿行、再画一个填不上的空"谁正在说"块。
    let opening = tail.iter().any(|v| {
        v.get("type").and_then(|t| t.as_str()) == Some("working")
            && v.get("agent").is_some_and(|a| !a.is_null())
    });
    assert!(!opening, "开跑那条运行态已过时，不许再进尾巴");
    // **逐条互补**：尾巴里的每条事实，都得是事件台上**转录没用掉**的那份（不重复、不凭空造）。
    let key = |v: &serde_json::Value| serde_json::to_string(v).expect("序列化");
    let mut bus: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let (lines, _h, _o) = handle.events().snapshot(Some(&opened.sid), 0);
    for l in &lines {
        for ev in &l.events {
            *bus.entry(key(&ev.to_json())).or_insert(0) += 1;
        }
    }
    let mut tail_cnt: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for ev in &tail {
        *tail_cnt.entry(key(ev)).or_insert(0) += 1;
    }
    for (k, n) in &tail_cnt {
        let on_bus = bus.get(k).copied().unwrap_or(0);
        let on_disk = transcript.iter().filter(|v| key(v) == *k).count();
        assert!(
            *n <= on_bus.saturating_sub(on_disk),
            "尾巴不许重复盘上已有的事实（也不许凭空造）：{}",
            k.chars().take(120).collect::<String>()
        );
    }
}

// ---------- 停止：生成期间也立刻生效（并发归核心） ----------

#[test]
fn stop_takes_effect_while_generation_is_still_running() {
    // 慢通道与装配在 contract_tests 里共用（intent 的「生成中不许改」测试用的是同一个）。
    let (_handle, ops, ticks) = slow_ops(vec![module_of("a")]);
    let sid = ops
        .sessions
        .create_work(single_work("w", &["a"]))
        .expect("建会话")
        .0
        .sid;

    let worker = {
        let sessions = Arc::clone(&ops.sessions);
        let sid = sid.clone();
        std::thread::spawn(move || sessions.say(&sid, "慢慢来", Output::Stream))
    };
    // 生成要真的开始跑（核心内部登记完成后 is_running 才为真）。
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ops.sessions.is_running(&sid) {
        assert!(Instant::now() < deadline, "生成没有启动");
        std::thread::sleep(Duration::from_millis(5));
    }

    let stopped = Instant::now();
    assert!(ops.sessions.stop(&sid), "在跑就该停得掉");
    assert!(ops.sessions.is_running(&sid), "停止是置位，不是同步等待");
    let out = worker.join().expect("生成线程");
    assert!(out.is_ok(), "停止是正常收尾，不是错误：{:?}", out.err());
    assert!(
        stopped.elapsed() < Duration::from_secs(3),
        "停止后要立刻收尾，不能等它跑完（{:?}）",
        stopped.elapsed()
    );
    assert!(!ops.sessions.is_running(&sid), "收尾后不再标记在跑");
    assert!(ticks.load(Ordering::Relaxed) > 0, "通道确实被调用过");
    assert!(
        !ops.sessions.stop("没这个会话"),
        "停一个没在跑的会话 = false"
    );
}

/// 生成期间，**只读命令不再排队**：生成不占用命令队列，读接口（历史列表 / 会话视图）
/// 会一直等到生成结束——界面因此"假死"。现在生成在工作线程上，队列只占"取/交"两步。
#[test]
fn reads_are_not_queued_behind_a_long_generation() {
    let (_handle, ops, ticks) = slow_ops(vec![module_of("a")]);
    let sid = ops
        .sessions
        .create_work(single_work("w", &["a"]))
        .expect("建会话")
        .0
        .sid;
    let worker = {
        let sessions = Arc::clone(&ops.sessions);
        let sid = sid.clone();
        std::thread::spawn(move || sessions.say(&sid, "慢慢来", Output::Stream))
    };
    // 等生成真的开始（通道已经在吐片）。
    let deadline = Instant::now() + Duration::from_secs(5);
    while ticks.load(Ordering::Relaxed) == 0 {
        assert!(Instant::now() < deadline, "生成没有启动");
        std::thread::sleep(Duration::from_millis(5));
    }
    // 生成进行中：两条只读命令都必须立刻返回——这是这次改动的全部意义。
    let t0 = Instant::now();
    let history = ops.history.list().expect("生成期间读历史");
    let read = t0.elapsed();
    let t1 = Instant::now();
    let views = ops
        .sessions
        .session_views(&history)
        .expect("生成期间读会话视图");
    let view = t1.elapsed();
    assert!(
        read < Duration::from_secs(1),
        "生成期间读历史不该排队（{:?}）",
        read
    );
    assert!(
        view < Duration::from_secs(1),
        "生成期间读会话视图不该排队（{:?}）",
        view
    );
    // 正在生成的会话仍要出现在视图里（对象在工作线程上，但它确实存在）。
    assert!(
        views.iter().any(|v| v.sid == sid),
        "生成中的会话不该从列表里消失：{:?}",
        views.iter().map(|v| v.sid.clone()).collect::<Vec<_>>()
    );
    // 关键判据：读完成时生成**必须还在跑**。若读被排在生成后面，它只会在生成结束后返回，
    // 那时这里就是 false——这条断言让"读没排队"这件事不必靠时间阈值单独成立。
    assert!(
        ops.sessions.is_running(&sid),
        "读完成时生成必须仍在进行（否则读是被排队到生成结束才返回的）"
    );

    ops.sessions.stop(&sid);
    let _ = worker.join().expect("生成线程");
}

/// 协作的长步骤（开始讨论）期间，只读命令同样不排队——B-1 只搬了单 agent，这条是协作。
#[test]
fn reads_are_not_queued_behind_a_collab_discussion() {
    let (_handle, ops, started, release) = gated_ops(vec![module_of("a"), module_of("b")]);
    let sid = ops
        .sessions
        .create_work(collab_work("c", &["a", "b"], false, "把资料整理成报告"))
        .expect("建协作会话")
        .0
        .sid;
    let worker = {
        let sessions = Arc::clone(&ops.sessions);
        let sid = sid.clone();
        std::thread::spawn(move || {
            sessions.collab_step(
                &sid,
                crate::capabilities::conductor::api::CollabStep::Begin,
                "yes",
            )
        })
    };
    // 等讨论真的开始（通道已被调用并卡在那里）。
    let deadline = Instant::now() + Duration::from_secs(5);
    while started.load(Ordering::Relaxed) == 0 {
        assert!(Instant::now() < deadline, "讨论没有启动");
        std::thread::sleep(Duration::from_millis(5));
    }
    // 讨论进行中：只读命令必须立刻返回。
    let t0 = Instant::now();
    let history = ops.history.list().expect("讨论期间读历史");
    let read = t0.elapsed();
    let t1 = Instant::now();
    let _ = ops
        .sessions
        .session_views(&history)
        .expect("讨论期间读会话视图");
    let view = t1.elapsed();
    assert!(
        read < Duration::from_secs(1),
        "讨论期间读历史不该排队（{:?}）",
        read
    );
    assert!(
        view < Duration::from_secs(1),
        "讨论期间读会话视图不该排队（{:?}）",
        view
    );
    assert!(
        ops.sessions.is_running(&sid),
        "读完成时讨论必须仍在进行（否则读是被排队到讨论结束才返回的）"
    );

    // 放行：协作的泵目前没有取消检查（缺口账另记），所以用放行结束而不是「停止」。
    release.store(true, Ordering::Relaxed);
    let _ = worker.join().expect("协作线程");
}

/// 协作的「停止」：在一个成员调用内收尾；**被中断的那条发言不吸收**；会话保持可继续。
#[test]
fn stopping_a_collab_discussion_is_prompt_and_keeps_the_session() {
    let (handle, ops, started, release) = gated_ops(vec![module_of("a"), module_of("b")]);
    let sid = ops
        .sessions
        .create_work(collab_work("c", &["a", "b"], false, "把资料整理成报告"))
        .expect("建协作会话")
        .0
        .sid;
    let worker = {
        let sessions = Arc::clone(&ops.sessions);
        let sid = sid.clone();
        std::thread::spawn(move || {
            sessions.collab_step(
                &sid,
                crate::capabilities::conductor::api::CollabStep::Begin,
                "yes",
            )
        })
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while started.load(Ordering::Relaxed) == 0 {
        assert!(Instant::now() < deadline, "讨论没有启动");
        std::thread::sleep(Duration::from_millis(5));
    }

    let stopped = Instant::now();
    assert!(ops.sessions.stop(&sid), "在跑就该停得掉");
    worker
        .join()
        .expect("协作线程")
        .expect("停止是正常收尾，不是错误");
    assert!(
        stopped.elapsed() < Duration::from_secs(3),
        "停止要在一个成员调用内收尾（{:?}）",
        stopped.elapsed()
    );
    // 回包只给头部序号；事实从**事件台**取（命令不携带事实）。
    let (batches, _head, _oldest) = handle.events().snapshot(Some(&sid), 0);
    let on_bus: Vec<&SessionEvent> = batches.iter().flat_map(|l| l.events.iter()).collect();
    // 如实告知：用户看得到"停在哪、没作废"。
    assert!(
        on_bus
            .iter()
            .any(|e| matches!(e, SessionEvent::Notice(n) if n.contains("[停止]"))),
        "要有「已停止」的如实说明"
    );
    // **子会话也要收尾**：它那一回合没有定稿行，流式层必须撤下并如实说一句——
    // 否则打开那个标签页，光标一直挂着、按钮一直停在「停止」。
    // 替身让第一个成员（a）正常说完、第二个（b）卡住被停——收尾要看**被停的那个**。
    let child = format!("{}--b", sid);
    let (child_batches, _ch, _co) = handle.events().snapshot(Some(&child), 0);
    let on_child: Vec<&SessionEvent> = child_batches.iter().flat_map(|l| l.events.iter()).collect();
    assert!(
        on_child
            .iter()
            .any(|e| matches!(e, SessionEvent::Working { agent: None })),
        "被停的子会话要收到运行态收尾：{:?}",
        on_child.iter().map(|e| e.to_json()).collect::<Vec<_>>()
    );
    assert!(
        on_child
            .iter()
            .any(|e| matches!(e, SessionEvent::Notice(n) if n.contains("[停止]"))),
        "子会话要如实收到「已停止」：{:?}",
        on_child.iter().map(|e| e.to_json()).collect::<Vec<_>>()
    );
    // 被中断的那条发言（半截 agree）**不该**进转录。
    let lines: Vec<String> = on_bus
        .iter()
        .filter_map(|e| match e {
            SessionEvent::Transcript(ls) => Some(ls.iter().map(|l| l.render())),
            _ => None,
        })
        .flatten()
        .collect();
    // 替身让第一个成员正常说完、第二个卡住：只有**被中断的那个**不该有发言。
    let spoken: Vec<&String> = lines
        .iter()
        .filter(|t| t.contains("[a:") || t.contains("[b:"))
        .collect();
    assert_eq!(
        spoken.len(),
        1,
        "只该有第一个成员那一条（被中断的那条不吸收）：{:?}",
        lines
    );
    assert!(
        lines.iter().all(|t| !t.contains("[b:")),
        "被中断的成员不该有发言：{:?}",
        lines
    );
    // 可继续：放行后再点「继续」，泵应接着推进（取消标志是每次派发新登记的，不会粘住）。
    release.store(true, Ordering::Relaxed);
    assert!(
        ops.sessions.continue_flow(&sid, Output::Final).is_ok(),
        "停止之后必须能「继续」"
    );
}

/// 协作生成**中途**就已经落盘：中途刷新页面能看到已产生的部分（按轮增量落盘）。
#[test]
fn collab_transcript_lands_on_disk_while_the_discussion_runs() {
    let (_handle, ops, started, release) = gated_ops(vec![module_of("a"), module_of("b")]);
    let sid = ops
        .sessions
        .create_work(collab_work("c", &["a", "b"], false, "把资料整理成报告"))
        .expect("建协作会话")
        .0
        .sid;
    let worker = {
        let sessions = Arc::clone(&ops.sessions);
        let sid = sid.clone();
        std::thread::spawn(move || {
            sessions.collab_step(
                &sid,
                crate::capabilities::conductor::api::CollabStep::Begin,
                "yes",
            )
        })
    };
    // 等第二个成员卡在调用里：此时第一个成员的发言已经定稿。
    let deadline = Instant::now() + Duration::from_secs(5);
    while started.load(Ordering::Relaxed) < 2 {
        assert!(Instant::now() < deadline, "讨论没有推进到第二个成员");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        ops.sessions.is_running(&sid),
        "讨论必须仍在进行，这条断言才有意义"
    );
    // 生成**还在跑**：盘上已经该有定稿的行（按轮增量落盘）。
    let (_, events) = ops.history.open(&sid).expect("中途读转录");
    assert!(
        !events.is_empty(),
        "生成中途就该有落盘内容（中途刷新页面靠它）"
    );

    release.store(true, Ordering::Relaxed);
    let _ = worker.join().expect("协作线程");
}

/// 协作逐成员外送：一个成员说完，它那一行**立刻**进事件台（不攒到整轮结束）。
#[test]
fn collab_discussion_emits_each_member_line_as_it_speaks() {
    let (handle, ops, started, release) = gated_ops(vec![module_of("a"), module_of("b")]);
    let sid = ops
        .sessions
        .create_work(collab_work("c", &["a", "b"], false, "把资料整理成报告"))
        .expect("建协作会话")
        .0
        .sid;
    let worker = {
        let sessions = Arc::clone(&ops.sessions);
        let sid = sid.clone();
        std::thread::spawn(move || {
            sessions.collab_step(
                &sid,
                crate::capabilities::conductor::api::CollabStep::Begin,
                "yes",
            )
        })
    };
    // 等第二个成员卡住：说明第一个成员已经说完，但**整轮还没结束**。
    let deadline = Instant::now() + Duration::from_secs(5);
    while started.load(Ordering::Relaxed) < 2 {
        assert!(Instant::now() < deadline, "讨论没有推进到第二个成员");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        ops.sessions.is_running(&sid),
        "整轮必须还没结束，这条断言才有意义"
    );
    let (lines, _, _oldest) = handle.events().snapshot(Some(&sid), 0);
    let spoken: Vec<String> = lines
        .iter()
        .flat_map(|l| l.events.iter())
        .filter_map(|e| match e {
            SessionEvent::Transcript(ls) => Some(ls.iter().map(|x| x.render())),
            _ => None,
        })
        .flatten()
        .collect();
    assert!(
        spoken
            .iter()
            .any(|t| t.contains("[a:") || t.contains("[b:")),
        "整轮还没结束时，第一个成员的发言就该已经外送：{:?}",
        spoken
    );

    release.store(true, Ordering::Relaxed);
    let _ = worker.join().expect("协作线程");
}

// ---------- 错误如实传播，不静默兜底 ----------

#[test]
fn missing_sessions_and_bad_inputs_come_back_as_errors() {
    let handle = spawn(vec![module_of("a")], Vec::new());
    let ops = Ops::from_handle(&handle);
    assert!(ops
        .sessions
        .say("没这个会话", "你好", Output::Final)
        .is_err());
    assert!(ops.sessions.config("没这个会话").is_err());
    assert!(ops.sessions.pending("没这个会话").is_err());
    assert!(ops.sessions.files("没这个会话").is_err());
    assert!(!ops.sessions.exists("没这个会话").expect("查存在"));
    assert!(ops.history.open("没这个会话").is_err());
    assert!(
        ops.registry.remove_provider("没这个供应商").is_ok(),
        "删不存在的供应商返回 false，不是错误"
    );
    assert!(ops.sessions.rewind("没这个会话", 0).is_err());
}

#[test]
fn a_panicking_command_does_not_take_the_core_down() {
    let handle = spawn(vec![module_of("a")], Vec::new());
    let ops = Ops::from_handle(&handle);
    let err = handle.panic_probe().unwrap_err();
    assert!(
        err.contains("无回应"),
        "panic 要以「无回应」如实回报：{}",
        err
    );
    // 核心仍然活着并且能继续服务（这才是接住 panic 的意义）。
    assert!(ops.registry.settings().is_ok(), "panic 之后必须还能服务");
    assert!(ops.workspace.roster().is_ok());
}

/// 压缩：发送视图变成「摘要 + 之后的内容」（转录完整）；压两次只有**一份**摘要（滚动）。
#[test]
fn compacting_replaces_the_send_view_with_one_rolling_summary() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut member = std::collections::BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "第一件事".to_string(),
            "{\"type\":\"tool\",\"name\":\"compact\",\"args\":{\"summary\":\"摘要一\"}}"
                .to_string(),
            "{\"type\":\"tool\",\"name\":\"compact\",\"args\":{\"summary\":\"摘要二\"}}"
                .to_string(),
            "收尾".to_string(),
        ],
    );
    let gateway = super::RecordingGateway {
        inner: super::doubles::ScriptGateway::new(member, vec![]),
        seen: Arc::clone(&seen),
    };
    let handle = crate::capabilities::conductor::api::ConductorHandle::spawn(
        super::doubles::core_with_gateway(vec![module_of("a")], gateway),
    )
    .expect("起核心线程");
    let ops = crate::capabilities::conductor::api::Ops::from_handle(&handle);
    let sid = ops
        .sessions
        .create_work(single_work("w", &["a"]))
        .expect("建会话")
        .0
        .sid;
    ops.sessions
        .say(
            &sid,
            "先做第一件事",
            crate::capabilities::conductor::api::Output::Final,
        )
        .expect("说一句");

    let first = ops.sessions.compact(&sid).expect("第一次压缩");
    let (b1, head1, _o1) = handle.events().snapshot(Some(&sid), 0);
    let s1: Vec<&SessionEvent> = b1.iter().flat_map(|l| l.events.iter()).collect();
    assert!(
        s1.iter()
            .any(|e| matches!(e, SessionEvent::Compacted { summary, .. } if summary == "摘要一")),
        "第一次压缩该如实落一条压缩事件（头部 {}）",
        first.head
    );
    let second = ops.sessions.compact(&sid).expect("第二次压缩");
    let (b2, _head2, _o2) = handle.events().snapshot(Some(&sid), head1);
    let s2: Vec<&SessionEvent> = b2.iter().flat_map(|l| l.events.iter()).collect();
    assert!(
        s2.iter()
            .any(|e| matches!(e, SessionEvent::Compacted { summary, .. } if summary == "摘要二")),
        "第二次压缩该如实落一条压缩事件（头部 {}）",
        second.head
    );

    let all = seen.lock().expect("锁").clone();
    let last = all.last().expect("至少问过一次").clone();
    assert!(
        last.iter().any(|c| c.contains("摘要一")),
        "第二次压缩的发送视图该带上上一份摘要（滚动摘要）：{last:?}"
    );
    assert!(
        !last.iter().any(|c| c.contains("第一件事")),
        "原始内容该已移出发送视图：{last:?}"
    );

    // 再走一轮：发送视图里**只剩一份摘要**（第二次那份），上一份已被取代。
    ops.sessions
        .say(
            &sid,
            "继续",
            crate::capabilities::conductor::api::Output::Final,
        )
        .expect("压完再走一轮");
    let after = seen.lock().expect("锁").clone();
    let last = after.last().expect("至少问过一次").clone();
    assert!(
        last.iter().any(|c| c.contains("摘要二")),
        "第二次压缩后的发送视图该是第二份摘要：{last:?}"
    );
    assert!(
        !last.iter().any(|c| c.contains("摘要一")),
        "上一份摘要该已被取代（同一时刻只有一份）：{last:?}"
    );
}

/// 到点自动压一次：历史超过预算时，**这一轮开始前**先压（发送视图里出现摘要）。
#[test]
fn auto_compaction_kicks_in_when_the_history_exceeds_the_budget() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let long = "长".repeat(2000);
    let mut member = std::collections::BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            long.clone(),
            "{\"type\":\"tool\",\"name\":\"compact\",\"args\":{\"summary\":\"自动摘要\"}}"
                .to_string(),
            "继续做".to_string(),
        ],
    );
    let gateway = super::RecordingGateway {
        inner: super::doubles::ScriptGateway::new(member, vec![]),
        seen: Arc::clone(&seen),
    };
    let handle = crate::capabilities::conductor::api::ConductorHandle::spawn(
        super::doubles::core_with_gateway(vec![module_of("a")], gateway),
    )
    .expect("起核心线程");
    let ops = crate::capabilities::conductor::api::Ops::from_handle(&handle);
    // 阈值调到 1%：预算 = 32000 × 1% × 4 ≈ 1280 字符，上面那条长回复会超。
    let mut st = ops.registry.settings().expect("读设置");
    st.compact_at_percent = 1;
    ops.registry.set_settings(st).expect("写设置");
    let sid = ops
        .sessions
        .create_work(single_work("w", &["a"]))
        .expect("建会话")
        .0
        .sid;
    ops.sessions
        .say(
            &sid,
            "先做第一件事",
            crate::capabilities::conductor::api::Output::Final,
        )
        .expect("第一轮");
    ops.sessions
        .say(
            &sid,
            "接着做",
            crate::capabilities::conductor::api::Output::Final,
        )
        .expect("第二轮（开头该自动压一次）");

    let all = seen.lock().expect("锁").clone();
    assert!(
        all.iter()
            .any(|msgs| msgs.iter().any(|c| c.contains("自动摘要"))),
        "超过预算时该自动压一次（发送视图里出现摘要）：{all:?}"
    );

    let last = all.last().expect("至少问过一次").clone();
    assert!(
        last.iter().any(|c| c.contains("自动摘要")),
        "压缩后的发送视图该带上摘要：{last:?}"
    );
    assert!(
        !last.iter().any(|c| c.contains(&long)),
        "被总结掉的内容该移出发送视图（不是只加摘要）：{last:?}"
    );
}
/// 压缩能扛住重启：按落盘转录重建时，发送视图仍是「一份摘要 + 之后的行」。
/// 回档跨越压缩点（`Conductor::rewind` 命中分流）→ 回到压缩前。
#[test]
fn compaction_survives_a_restart_and_rewinds_back_through_the_point() {
    let mut core = super::doubles::core_with(
        vec![module_of("a")],
        super::doubles::gw(std::collections::BTreeMap::new(), Vec::new()),
    );
    let sid = core
        .create_work(single_work("w", &["a"]))
        .expect("建会话")
        .sid;
    // 与**真实流水**对齐起号：建会话时已经种了一条系统提示词行，不能重号。
    let (_, existing) = core.history_open(&sid).expect("读流水");
    let next = existing
        .iter()
        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("transcript"))
        .filter_map(|e| e.get("lines").and_then(|l| l.as_array()))
        .flatten()
        .filter_map(|l| l.get("id").and_then(|i| i.as_u64()))
        .max()
        .map(|m| m + 1)
        .unwrap_or(0);
    let line = |id: u64, kind: &str, text: &str| serde_json::json!({"id": id, "reply": id, "line": text, "kind": kind, "speaker": "", "verb": "", "turn": 0});
    // 两份行 + 一次压缩（覆盖到 next+2）+ 之后的一行：模拟「压过之后再重启」的落盘历史。
    for (id, kind, text) in [
        (next, "user", "第一句话"),
        (next + 1, "assistant", "第一句的回复"),
    ] {
        core.history_append(
            &sid,
            &[serde_json::json!({"type": "transcript", "lines": [line(id, kind, text)]})],
        )
        .expect("落行");
    }
    core.history_append(
        &sid,
        &[serde_json::json!({"type": "compacted", "up_to": next + 2, "summary": "前两句的摘要"})],
    )
    .expect("落压缩事件");
    core.history_append(
        &sid,
        &[serde_json::json!({"type": "transcript", "lines": [line(next + 2, "user", "之后的新内容")]})],
    )
    .expect("落之后的新行");
    let shown = |msgs: &[crate::capabilities::llm::api::Msg]| {
        msgs.iter()
            .map(|m| format!("{}:{}", m.role, m.content))
            .collect::<Vec<_>>()
    };

    // ① 重建（重启：会话不在内存里 → 按落盘转录重建）：摘要替代被压的行，之后的行照常。
    // `prepare_single` 把会话交给工作线程，重建出来的对象就在它返回的这一轮里。
    let live = core.take_single(&sid).expect("会话在表里");
    drop(live);
    core.abort_running(&sid);
    let rebuilt = match core.prepare_single(&sid, None, false) {
        Ok(crate::capabilities::conductor::service::Prepared::Run { session, .. }) => session,
        Ok(_) => panic!("重建后该是可直接跑的一轮"),
        Err(e) => panic!("重建失败：{e}"),
    };
    assert_eq!(
        shown(rebuilt.dialogue()),
        vec![
            "user:[此前内容摘要]\n前两句的摘要".to_string(),
            "user:之后的新内容".to_string(),
        ],
        "重建后的发送视图该是「摘要 + 之后的行」，被压掉的内容不回来"
    );
    assert_eq!(
        rebuilt.compacted_upto(),
        next + 2,
        "重建也要恢复压缩点（回档分流靠它）"
    );
    drop(rebuilt);
    core.abort_running(&sid);

    // ② 回档跨越压缩点：会话在表里且压缩点 > 目标 → `rewind` 走重建，摘要不再生效。
    core.ensure_session(&sid).expect("把重建结果装回表里");
    core.rewind(&sid, next).expect("回档到压缩点之前");
    let back = core.take_single(&sid).expect("取回回档后的会话");
    let shown_back = shown(back.dialogue());
    assert!(
        !shown_back.iter().any(|c| c.contains("此前内容摘要")),
        "回档到压缩点之前：摘要该消失（内容回到压缩前）：{shown_back:?}"
    );
    assert_eq!(back.compacted_upto(), 0, "压缩点该一起回退掉");
}

// ---------- 从「共享意图层」搬来的规则测试（规则跟着归属走） ----------

#[test]
fn pick_agents_names_the_unknown_and_lists_the_rest() {
    let (_h, ops) = ops_with(vec![module_of("a")], Vec::new());
    ops.registry
        .upsert_agent("甲", &["a".to_string()], "", "")
        .expect("建 agent");
    let picked = ops.registry.pick_agents(&["甲".to_string()]).expect("点名");
    assert_eq!(picked.len(), 1);
    assert_eq!(picked[0].name, "甲");
    let err = ops.registry.pick_agents(&["乙".to_string()]).unwrap_err();
    assert!(
        err.contains("无此 agent：乙") && err.contains("甲"),
        "{err}"
    );
    assert!(ops.registry.pick_agents(&[]).is_err(), "没点名 = 报错");
    let views = ops.registry.agents().expect("读登记处");
    assert_eq!(
        views.iter().map(AgentInstance::from_view).count(),
        1,
        "视图 → 实例不丢项"
    );
}

#[test]
fn unique_work_name_falls_back_and_appends_a_suffix() {
    let (_h, ops) = ops_with(vec![module_of("a")], Vec::new());
    assert_eq!(
        ops.sessions.unique_work_name("", "single").expect("缺省名"),
        "single"
    );
    assert_eq!(
        ops.sessions
            .unique_work_name("  取名  ", "single")
            .expect("去空白"),
        "取名"
    );
    ops.sessions
        .create_work(single_work("w", &["a"]))
        .expect("建会话");
    assert_eq!(
        ops.sessions.unique_work_name("w", "x").expect("重名加尾号"),
        "w-2"
    );
    ops.sessions
        .create_work(single_work("w-2", &["a"]))
        .expect("建会话");
    assert_eq!(
        ops.sessions.unique_work_name("w", "x").expect("再重名"),
        "w-3"
    );
}

#[test]
fn single_mode_merges_multiple_agents_into_one_transient() {
    let (_h, ops) = ops_with(vec![module_of("a"), module_of("b")], Vec::new());
    ops.registry
        .upsert_agent("甲", &["a".to_string(), "b".to_string()], "", "")
        .expect("建 agent");
    ops.registry
        .upsert_agent("乙", &["b".to_string()], "", "")
        .expect("建 agent");
    let picked: Vec<AgentInstance> = ops
        .registry
        .agents()
        .expect("读登记处")
        .iter()
        .map(AgentInstance::from_view)
        .collect();
    let (opened, _) = ops
        .sessions
        .create_work(WorkSpec {
            name: "组合".to_string(),
            mode: WorkMode::Single,
            agents: picked,
            task: None,
            delegate: false,
            tier: crate::kernel::api::Tier::Host,
        })
        .expect("建工作");
    assert_eq!(
        opened.agents,
        vec!["组合".to_string()],
        "多个点名并成一个临时组合"
    );
    let (meta, _) = ops.history.open("组合").expect("读 meta");
    let mut modules = meta.agents[0].modules.clone();
    modules.sort();
    assert_eq!(
        modules,
        vec!["a".to_string(), "b".to_string()],
        "模块去重（顺序随登记处遍历序，不额外断言）"
    );
    assert!(meta.agents[0].transient, "并出来的「组合」是临时 agent");
}

#[test]
fn editing_is_refused_while_a_session_is_generating() {
    let (_h, ops, _ticks) = slow_ops(vec![module_of("a")]);
    let sid = ops
        .sessions
        .create_work(single_work("w", &["a"]))
        .expect("建会话")
        .0
        .sid;
    // 编辑体照抄当前配置（空名单会被"至少要有一个 agent"挡掉，那是另一条规则）。
    let cfg = ops.sessions.config(&sid).expect("读配置");
    let edit = || SessionEdit {
        agents: cfg.agents.clone(),
        tier: "host".to_string(),
        base: None,
        pins: std::collections::BTreeMap::new(),
        net: false,
    };
    ops.sessions.edit(&sid, edit()).expect("没在生成时可以改");

    let worker = {
        let sessions = Arc::clone(&ops.sessions);
        let sid = sid.clone();
        std::thread::spawn(move || sessions.say(&sid, "慢慢来", Output::Stream))
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ops.sessions.is_running(&sid) {
        assert!(Instant::now() < deadline, "生成没有启动");
        std::thread::sleep(Duration::from_millis(5));
    }
    let err = ops.sessions.edit(&sid, edit()).unwrap_err();
    assert!(err.contains("正在生成中"), "生成中必须拒绝改配置：{err}");

    assert!(ops.sessions.stop(&sid), "停掉它");
    let _ = worker.join();
    ops.sessions.edit(&sid, edit()).expect("收尾后可以改");
    assert!(
        ops.sessions.edit("没这个会话", edit()).is_err(),
        "无此会话要如实报错"
    );
}

#[test]
fn act_dispatches_to_the_two_result_shapes() {
    let (_h, ops) = ops_with(vec![module_of("a")], Vec::new());
    let sid = ops
        .sessions
        .create_work(single_work("w", &["a"]))
        .expect("建会话")
        .0
        .sid;

    match ops
        .sessions
        .act(&sid, Action::Say("你好"), Output::Final)
        .expect("说一句")
    {
        Acted::Advanced(adv) => assert!(adv.head > 0, "生成类只回事件台头部序号"),
        Acted::Replayed(_) => panic!("说一句不该给重放"),
    }
    match ops
        .sessions
        .act(&sid, Action::Rewind(0), Output::Final)
        .expect("回档")
    {
        Acted::Replayed(events) => {
            assert!(events.iter().all(|e| e.is_object()), "重放是线格式事件数组")
        }
        Acted::Advanced(_) => panic!("回档不该给事件批"),
    }
    assert!(
        ops.sessions
            .act("没这个会话", Action::Say("x"), Output::Final)
            .is_err(),
        "无此会话如实报错"
    );
}
