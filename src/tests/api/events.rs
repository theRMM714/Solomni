//! 目的：事实总线与历史——生成把事实推上总线，历史合并是转录加总线尾巴。
//! 管：序号、去重、回档后的重置事实、停止在生成中生效。
//! 不管：压缩（`compaction.rs`）、协作讨论（`collab.rs`）。
//! 联动：`src/capabilities/conductor/api/handle.rs` 与 `src/capabilities/session/api`。

use super::super::doubles::module_of;
use super::super::{single_work, slow_ops};
use super::support::spawn;
use crate::capabilities::conductor::api::SessionEvent;
use crate::capabilities::conductor::api::{Ops, Output};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

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

#[test]
fn rewind_pushes_a_reset_fact_and_history_merge_ignores_superseded_batches() {
    let handle = spawn(vec![module_of("a")], Vec::new());
    let ops = Ops::from_handle(&handle);
    let sid = ops
        .sessions
        .create_work(single_work("w", &["a"]))
        .expect("建会话")
        .0
        .sid;
    ops.sessions
        .say(&sid, "问一", Output::Final)
        .expect("说一句");
    ops.sessions
        .say(&sid, "问二", Output::Final)
        .expect("再说一句");

    // ① 删除到第 0 行：盘上转录清空；事件台必须给一条 rewound（活动视图为空）。
    let replayed = ops
        .sessions
        .rewind(
            &sid,
            crate::capabilities::conductor::api::RewindTarget::Delete(0),
        )
        .expect("回档到第 0 行");
    assert!(replayed.is_empty(), "删除到 0 的活动视图应为空");

    let (lines, _head, _oldest) = handle.events().snapshot(Some(&sid), 0);
    let mut saw_reset = false;
    for line in &lines {
        for ev in &line.events {
            if let SessionEvent::Rewound { events } = ev {
                saw_reset = true;
                assert!(events.is_empty(), "删除到 0 的重置快照应为空：{:?}", events);
            }
        }
    }
    assert!(saw_reset, "回档成功后必须推一条 rewound");

    // ② 刷新/重新打开：盘上转录为空，事件台里的旧批次不能再被当成 live 拼回。
    let (_meta, transcript) = ops.history.open(&sid).expect("读转录");
    assert!(transcript.is_empty(), "删除到 0 后盘上转录应为空");
    let (tail, head) = handle.events().tail_excluding(&sid, &transcript);
    assert!(head > 0, "合流要给出头部序号");
    assert!(
        tail.iter()
            .all(|v| v.get("type").and_then(|t| t.as_str()) != Some("transcript")),
        "旧转录不许作为 live 回来：{:?}",
        tail
    );

    // ③ 回档后还能继续；新事件按水印之后的窗口接上，旧内容仍不回来。
    ops.sessions
        .say(&sid, "新问", Output::Final)
        .expect("回档后还能说");
    let (_meta, transcript2) = ops.history.open(&sid).expect("读新转录");
    let (tail2, _head2) = handle.events().tail_excluding(&sid, &transcript2);
    let text = serde_json::to_string(&tail2).expect("序列化尾巴");
    assert!(
        !text.contains("问一") && !text.contains("问二"),
        "回档前的旧内容不许从 live 回来：{}",
        text
    );

    // ④ 留档同样不能被旧批次污染：折叠后的活动视图只有标记，没有正文行。
    let sid2 = ops
        .sessions
        .create_work(single_work("w2", &["a"]))
        .expect("建第二个会话")
        .0
        .sid;
    ops.sessions
        .say(&sid2, "留档问", Output::Final)
        .expect("说一句");
    let archived = ops
        .sessions
        .rewind(
            &sid2,
            crate::capabilities::conductor::api::RewindTarget::Archive(0),
        )
        .expect("留档");
    assert!(
        archived
            .iter()
            .any(|v| v.get("type").and_then(|t| t.as_str()) == Some("rewind")),
        "留档活动视图应带标记：{:?}",
        archived
    );
    let (_meta, transcript3) = ops.history.open(&sid2).expect("读留档转录");
    let (tail3, _head3) = handle.events().tail_excluding(&sid2, &transcript3);
    assert!(
        tail3
            .iter()
            .all(|v| v.get("type").and_then(|t| t.as_str()) != Some("transcript")),
        "留档后旧转录不许作为 live 回来：{:?}",
        tail3
    );
}

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
    // 生成要真的**进入模型调用**才算"在跑"：is_running 在派发时就为真，此刻 worker 还没走到通道；
    // 若这一窗口里就 stop，on(Start) 会被取消挡下、ticks 恒为 0——等 ticks 才是"可停止"的确定状态。
    let deadline = Instant::now() + Duration::from_secs(10);
    while ticks.load(Ordering::Relaxed) == 0 {
        assert!(Instant::now() < deadline, "生成没有进入模型调用");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(ops.sessions.is_running(&sid), "进入模型调用时必须标记在跑");

    let stopped = Instant::now();
    assert!(!ops.sessions.stop(&sid).is_empty(), "在跑就该停得掉");
    let out = worker.join().expect("生成线程");
    assert!(out.is_ok(), "停止是正常收尾，不是错误：{:?}", out.err());
    assert!(
        stopped.elapsed() < Duration::from_secs(10),
        "停止后要立刻收尾，不能等它跑完（{:?}）",
        stopped.elapsed()
    );
    assert!(!ops.sessions.is_running(&sid), "收尾后不再标记在跑");
    assert!(ticks.load(Ordering::Relaxed) > 0, "通道确实被调用过");
    // 停止要留下可回档的内容行；原始瞬时 Notice 不得落盘（否则刷新后不可删）。
    let (_meta, events) = ops.history.open(&sid).expect("读停止后的转录");
    assert!(
        !events
            .iter()
            .any(|e| e.get("type").and_then(|t| t.as_str()) == Some("notice")),
        "原始 Notice 不落盘：{:?}",
        events
    );
    let stopped_line = events
        .iter()
        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("transcript"))
        .filter_map(|e| e.get("lines").and_then(|l| l.as_array()))
        .flatten()
        .any(|l| {
            l.get("kind").and_then(|k| k.as_str()) == Some("notice")
                && l.get("line")
                    .and_then(|s| s.as_str())
                    .is_some_and(|s| s.contains("[停止]"))
        });
    assert!(
        stopped_line,
        "停止要写一条可回档的 [停止] 内容行：{:?}",
        events
    );
    assert!(
        ops.sessions.stop("没这个会话").is_empty(),
        "停一个没在跑的会话 = 空"
    );
}

#[test]
fn archive_stops_a_running_turn_before_rewinding() {
    use crate::capabilities::conductor::api::RewindTarget;
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
    let deadline = Instant::now() + Duration::from_secs(10);
    while ticks.load(Ordering::Relaxed) == 0 {
        assert!(Instant::now() < deadline, "生成没有启动");
        std::thread::sleep(Duration::from_millis(5));
    }
    // 在跑的时候留档：先停、再动文件，不能回"正在生成"的拒绝。
    let view = ops
        .sessions
        .rewind(&sid, RewindTarget::Archive(0))
        .expect("留档要把在跑的回合停下来");
    let out = worker.join().expect("被停的回合也要正常收尾");
    assert!(out.is_ok(), "停止是正常收尾：{:?}", out.err());
    assert!(!ops.sessions.is_running(&sid), "收尾后不再标记在跑");
    assert!(
        view.iter()
            .any(|v| v.get("type").and_then(|t| t.as_str()) == Some("rewind")),
        "留档视图要带标记：{:?}",
        view
    );
    let (_meta, events) = ops.history.open(&sid).expect("读留档后的转录");
    assert!(
        events.iter().any(|e| {
            e.get("type").and_then(|t| t.as_str()) == Some("rewind")
                && e.get("mode").and_then(|m| m.as_str()) == Some("archive")
        }),
        "标记要落盘且带 archive：{:?}",
        events
    );
}
