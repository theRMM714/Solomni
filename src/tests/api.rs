//! 入站契约（`core::api`）的契约测试：命令/事件模型、能力分面、停止语义、panic 隔离。
//! 这一层不碰 HTTP；HTTP 侧（路由目录与逐路由契约）另见本目录的 routes。

use super::{ops_with, single_work, slow_ops};
use crate::core::api::{CoreHandle, Ops, Output};
use crate::core::exec::Tier;
use crate::core::module::Module;
use super::doubles::module_of;
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
    let opened = ops
        .sessions
        .create_work(single_work("w", &["a"]))
        .expect("建会话");
    assert!(bus.snapshot(None, 0).0.is_empty(), "建会话本身不入事件台");

    let adv = ops
        .sessions
        .say(&opened.sid, "你好", Output::Final)
        .expect("说一句");
    assert!(adv.seq > 0, "本批事件必须带序号");
    let (lines, head) = bus.snapshot(Some(&opened.sid), 0);
    assert_eq!(lines.len(), 1, "一次推进一批：{:?}", lines.len());
    assert_eq!(
        lines[0].seq, adv.seq,
        "回复里的序号就是事件台上的序号（客户端据此去重）"
    );
    assert_eq!(
        lines[0].events.len(),
        adv.events.len(),
        "事件台与回复是同一批事实"
    );
    assert_eq!(head, adv.seq);
    assert!(
        bus.snapshot(Some(&opened.sid), adv.seq).0.is_empty(),
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
