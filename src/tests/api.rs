//! 入站契约（`core::api`）的契约测试：命令/事件模型、能力分面、停止语义、panic 隔离。
//! 这一层不碰 HTTP；HTTP 侧（路由目录与逐路由契约）另见本目录的 routes。

use super::doubles::{collab_work, module_of};
use super::{gated_ops, ops_with, single_work, slow_ops};
use crate::core::api::{CoreHandle, Ops, Output};
use crate::core::exec::Tier;
use crate::core::module::Module;
use crate::core::SessionEvent;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 用内存装配起一个核心手柄（核心从此有自己的线程、自己的状态）。
fn spawn(modules: Vec<Module>, core_script: Vec<&str>) -> CoreHandle {
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

    assert_eq!(ops.discovery.roster().expect("清单").modules.len(), 1);
    assert!(ops
        .discovery
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
    // 逐轮外送：事件按"一轮一批"进台，所以这里是多批（以前是整回合一批）。
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

/// 生成期间，**只读命令不再排队**：以前生成占着唯一的命令队列，读接口（历史列表 / 会话视图）
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
        .history
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
            sessions.collab_step(&sid, crate::core::CollabStep::Begin, "yes")
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
        .history
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
            sessions.collab_step(&sid, crate::core::CollabStep::Begin, "yes")
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
            SessionEvent::Transcript(ls) => Some(ls.iter().map(|l| l.line.clone())),
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

/// 协作生成**中途**就已经落盘：中途刷新页面能看到已产生的部分（以前整段跑完才落一次）。
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
            sessions.collab_step(&sid, crate::core::CollabStep::Begin, "yes")
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
    // 生成**还在跑**：盘上已经该有定稿的行（以前是整段跑完才落一次）。
    let (_, events) = ops.history.open(&sid).expect("中途读转录");
    assert!(
        !events.is_empty(),
        "生成中途就该有落盘内容（中途刷新页面靠它）"
    );

    release.store(true, Ordering::Relaxed);
    let _ = worker.join().expect("协作线程");
}

/// 协作逐成员外送：一个成员说完，它那一行**立刻**进事件台（以前整轮问完才一次性出）。
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
            sessions.collab_step(&sid, crate::core::CollabStep::Begin, "yes")
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
            SessionEvent::Transcript(ls) => Some(ls.iter().map(|x| x.line.clone())),
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
    assert!(ops.discovery.roster().is_ok());
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
        ],
    );
    let gateway = super::RecordingGateway {
        inner: super::doubles::ScriptGateway::new(member, vec![]),
        seen: Arc::clone(&seen),
    };
    let handle = crate::core::api::CoreHandle::spawn(super::doubles::core_with_gateway(
        vec![module_of("a")],
        gateway,
    ))
    .expect("起核心线程");
    let ops = crate::core::api::Ops::from_handle(&handle);
    let sid = ops
        .sessions
        .create_work(single_work("w", &["a"]))
        .expect("建会话")
        .0
        .sid;
    ops.sessions
        .say(&sid, "先做第一件事", crate::core::api::Output::Final)
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
    let handle = crate::core::api::CoreHandle::spawn(super::doubles::core_with_gateway(
        vec![module_of("a")],
        gateway,
    ))
    .expect("起核心线程");
    let ops = crate::core::api::Ops::from_handle(&handle);
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
        .say(&sid, "先做第一件事", crate::core::api::Output::Final)
        .expect("第一轮");
    ops.sessions
        .say(&sid, "接着做", crate::core::api::Output::Final)
        .expect("第二轮（开头该自动压一次）");

    let all = seen.lock().expect("锁").clone();
    assert!(
        all.iter()
            .any(|msgs| msgs.iter().any(|c| c.contains("自动摘要"))),
        "超过预算时该自动压一次（发送视图里出现摘要）：{all:?}"
    );
}
