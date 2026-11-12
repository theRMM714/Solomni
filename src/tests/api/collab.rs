//! 目的：协作讨论的可见性与停止——每句成员发言立刻可见，停下不丢会话。
//! 管：讨论期间的读、停止语义、落盘时序。
//! 不管：单人生成（`events.rs`）、协作回合内部（`src/tests/collab/`）。
//! 联动：`src/capabilities/conductor/api/handle.rs`。

use super::super::builders::shown_contains;
use super::super::doubles::{collab_work, module_of};
use super::super::gated_ops;
use crate::capabilities::conductor::api::Output;
use crate::capabilities::conductor::api::SessionEvent;
use crate::capabilities::session::api::OPT_BEGIN;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

#[test]
fn reads_are_not_queued_behind_a_collab_discussion() {
    let (_handle, ops, started, release) = gated_ops(vec![module_of("a"), module_of("b")]);
    let sid = ops
        .sessions
        .create_work(collab_work("c", &["a", "b"], false, "把资料整理成报告"))
        .expect("建协作会话")
        .0
        .sid;
    // 回答当前那张卡（选项 id 是契约）：协作从这一答开始跑泵。
    let card = ops
        .sessions
        .open_queue(&sid)
        .expect("取卡")
        .expect("挂着一张卡")
        .card
        .id;
    let worker = {
        let sessions = Arc::clone(&ops.sessions);
        let sid = sid.clone();
        let card = card.clone();
        std::thread::spawn(move || sessions.answer_card(&sid, &card, OPT_BEGIN, ""))
    };
    // 等讨论真的开始（通道已被调用并卡在那里）。
    let deadline = Instant::now() + Duration::from_secs(10);
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
        read < Duration::from_secs(10),
        "讨论期间读历史不该排队（{:?}）",
        read
    );
    assert!(
        view < Duration::from_secs(10),
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

#[test]
fn stopping_a_collab_discussion_is_prompt_and_keeps_the_session() {
    let (handle, ops, started, release) = gated_ops(vec![module_of("a"), module_of("b")]);
    let sid = ops
        .sessions
        .create_work(collab_work("c", &["a", "b"], false, "把资料整理成报告"))
        .expect("建协作会话")
        .0
        .sid;
    // 回答当前那张卡（选项 id 是契约）：协作从这一答开始跑泵。
    let card = ops
        .sessions
        .open_queue(&sid)
        .expect("取卡")
        .expect("挂着一张卡")
        .card
        .id;
    let worker = {
        let sessions = Arc::clone(&ops.sessions);
        let sid = sid.clone();
        let card = card.clone();
        std::thread::spawn(move || sessions.answer_card(&sid, &card, OPT_BEGIN, ""))
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    // 等到讨论真的进入**阻塞中的那次调用**（gated_ops 的第二次调用一直阻塞到放行/停止）；
    // 只等第一次会落在「第一次已返回、第二次还没开始」的窗口里，那时 stop 合理地看不到在跑的会话。
    while started.load(Ordering::Relaxed) < 2 || !ops.sessions.is_running(&sid) {
        assert!(Instant::now() < deadline, "讨论没有进入可停止的状态");
        std::thread::sleep(Duration::from_millis(5));
    }

    let stopped = Instant::now();
    assert!(!ops.sessions.stop(&sid).is_empty(), "在跑就该停得掉");
    // 回答与「停止」同一条直路：这一答只负责**落定**，处置脱离调用方跑，所以这里等它收尾。
    worker
        .join()
        .expect("协作线程")
        .expect("停止是正常收尾，不是错误");
    // **子会话也要收尾**：它那一回合没有定稿行，流式层必须撤下并如实说一句，否则打开那个标签页，光标一直挂着、按钮一直停在「停止」；替身让第一个成员（a）正常说完、第二个（b）卡住被停——收尾要看**被停的那个**。
    let child = format!("{}--b", sid);
    let stopped_line = |evs: &[SessionEvent]| shown_contains(evs, "[停止]");
    let bus_of = |sid: &str| -> Vec<SessionEvent> {
        let (batches, _head, _oldest) = handle.events().snapshot(Some(sid), 0);
        batches
            .iter()
            .flat_map(|l| l.events.iter().cloned())
            .collect()
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    let (mut on_bus, mut on_child) = (bus_of(&sid), bus_of(&child));
    while Instant::now() < deadline && !(stopped_line(&on_bus) && stopped_line(&on_child)) {
        std::thread::sleep(Duration::from_millis(10));
        on_bus = bus_of(&sid);
        on_child = bus_of(&child);
    }
    assert!(
        stopped.elapsed() < Duration::from_secs(10),
        "停止要在一个成员调用内收尾（{:?}）",
        stopped.elapsed()
    );
    // 如实告知：用户看得到"停在哪、没作废"。
    assert!(stopped_line(&on_bus), "要有「已停止」的如实说明");
    assert!(
        on_child
            .iter()
            .any(|e| matches!(e, SessionEvent::Working { agent: None })),
        "被停的子会话要收到运行态收尾：{:?}",
        on_child.iter().map(|e| e.to_json()).collect::<Vec<_>>()
    );
    assert!(
        shown_contains(&on_child, "[停止]"),
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
    // 停止的处置**脱离调用方**跑：事件台上的"已停止"在会话交回核心之前就外送了，不能凭它判断
    // "可以继续了"。等会话视图里不再标记在跑（SessionView.running 由 put_collab 清掉）再「继续」。
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let history = ops.history.list().expect("列历史");
        let views = ops.sessions.session_views(&history).expect("会话视图");
        if views.iter().any(|v| v.sid == sid && !v.running) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "停止后会话没有交回核心，无法继续"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    // 可继续：放行后再点「继续」，泵应接着推进（取消标志是每次派发新登记的，不会粘住）。
    release.store(true, Ordering::Relaxed);
    let resumed = ops.sessions.continue_flow(&sid, Output::Final);
    assert!(
        resumed.is_ok(),
        "停止之后必须能「继续」：{:?}",
        resumed.err()
    );
}

#[test]
fn collab_transcript_lands_on_disk_while_the_discussion_runs() {
    let (_handle, ops, started, release) = gated_ops(vec![module_of("a"), module_of("b")]);
    let sid = ops
        .sessions
        .create_work(collab_work("c", &["a", "b"], false, "把资料整理成报告"))
        .expect("建协作会话")
        .0
        .sid;
    // 回答当前那张卡（选项 id 是契约）：协作从这一答开始跑泵。
    let card = ops
        .sessions
        .open_queue(&sid)
        .expect("取卡")
        .expect("挂着一张卡")
        .card
        .id;
    let worker = {
        let sessions = Arc::clone(&ops.sessions);
        let sid = sid.clone();
        let card = card.clone();
        std::thread::spawn(move || sessions.answer_card(&sid, &card, OPT_BEGIN, ""))
    };
    // 等第二个成员卡在调用里：此时第一个成员的发言已经定稿。
    let deadline = Instant::now() + Duration::from_secs(10);
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

#[test]
fn collab_discussion_emits_each_member_line_as_it_speaks() {
    let (handle, ops, started, release) = gated_ops(vec![module_of("a"), module_of("b")]);
    let sid = ops
        .sessions
        .create_work(collab_work("c", &["a", "b"], false, "把资料整理成报告"))
        .expect("建协作会话")
        .0
        .sid;
    // 回答当前那张卡（选项 id 是契约）：协作从这一答开始跑泵。
    let card = ops
        .sessions
        .open_queue(&sid)
        .expect("取卡")
        .expect("挂着一张卡")
        .card
        .id;
    let worker = {
        let sessions = Arc::clone(&ops.sessions);
        let sid = sid.clone();
        let card = card.clone();
        std::thread::spawn(move || sessions.answer_card(&sid, &card, OPT_BEGIN, ""))
    };
    // 等第二个成员卡住：说明第一个成员已经说完，但**整轮还没结束**。
    let deadline = Instant::now() + Duration::from_secs(10);
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
