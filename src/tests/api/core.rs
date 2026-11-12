//! 目的：命令线程边界——命令在核心自己的线程上跑，读不排在长生成后面。
//! 管：命令可见性、长生成期间的读、缺会话与坏输入、panic 不带走核心。
//! 不管：事件事实（`events.rs`）、动作目录（`tools.rs`）。
//! 联动：`src/capabilities/conductor/api/handle.rs`。

use super::super::doubles::module_of;
use super::super::{single_work, slow_ops};
use super::support::spawn;
use crate::capabilities::conductor::api::{Ops, Output};
use crate::kernel::api::Tier;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

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

    assert_eq!(ops.modules.roster().expect("清单").modules.len(), 1);
    assert!(
        ops.core
            .runtime_report(Tier::Host)
            .expect("能力报告")
            .diagnoses
            .is_empty()
    );
    assert!(
        ops.history.list().expect("历史").is_empty(),
        "内存历史初始为空"
    );
}

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
    let deadline = Instant::now() + Duration::from_secs(10);
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
        read < Duration::from_secs(10),
        "生成期间读历史不该排队（{:?}）",
        read
    );
    assert!(
        view < Duration::from_secs(10),
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

#[test]
fn missing_sessions_and_bad_inputs_come_back_as_errors() {
    let handle = spawn(vec![module_of("a")], Vec::new());
    let ops = Ops::from_handle(&handle);
    assert!(
        ops.sessions
            .say("没这个会话", "你好", Output::Final)
            .is_err()
    );
    assert!(ops.sessions.config("没这个会话").is_err());
    assert!(ops.sessions.open_queue("没这个会话").is_err());
    assert!(ops.sessions.files("没这个会话").is_err());
    assert!(!ops.sessions.exists("没这个会话").expect("查存在"));
    assert!(ops.history.open("没这个会话").is_err());
    assert!(
        ops.registry.remove_provider("没这个供应商").is_ok(),
        "删不存在的供应商返回 false，不是错误"
    );
    assert!(
        ops.sessions
            .rewind(
                "没这个会话",
                crate::capabilities::conductor::api::RewindTarget::Archive(0)
            )
            .is_err()
    );
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
    assert!(ops.modules.roster().is_ok());
}
