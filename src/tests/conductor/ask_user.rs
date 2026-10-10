//! 目的：工具层提问端口的**会话侧实现**（`SessionAsk`）：推卡等回答、构不出可用选项时停会话 + 落警告。
//! 管：卡的登记与外送、阻塞等回答、空选项集的收场（停会话）、停过之后不再推卡。
//! 不管：队列本体的排队与校验（session 的用例）、围栏自己怎么问（tools 的用例）。
//! 联动：端口形状见 src/kernel/ports.rs；契约见 docs/session/session-model.md 的「请用户裁决」。

use crate::capabilities::conductor::api::{ConductorHandle, Ops, Output};
use crate::capabilities::conductor::service::ask_user::SessionAsk;
use crate::capabilities::session::api::SessionEvent;
use crate::kernel::api::{Ask, AskOutcome};
use crate::kernel::domain::fence::{OPT_FENCE_ABORT, OPT_FENCE_UNFENCED};
use crate::kernel::ports::AskUser;
use crate::tests::builders::SilentRunner;
use crate::tests::doubles::{abs, core_with_runner, gw, module_of};
use crate::tests::prelude::*;
use std::sync::{Arc, Mutex};

/// 一个单 agent 会话 + 它的核心手柄与能力面（提问端口要现造，卡要走核心那条道）。
fn session_case() -> (ConductorHandle, Ops, String) {
    let mut a = module_of("a");
    a.root = abs(&["mods", "a"]);
    let handle = ConductorHandle::spawn(core_with_runner(
        vec![a],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
    ))
    .expect("起核心线程");
    handle.allow_tool_cards();
    let ops = Ops::from_handle(&handle);
    let sid = ops
        .sessions
        .create_work(work("w", WorkMode::Single, &["a"]))
        .expect("建会话")
        .0
        .sid;
    (handle, ops, sid)
}

/// 会话自己的裁决队（与核心各关卡、工具级确认**同一份**）。
fn door_of(
    handle: &ConductorHandle,
    sid: &str,
) -> Arc<crate::capabilities::session::api::DecisionDoor> {
    let s = sid.to_string();
    handle
        .call(move |core| Ok(core.desk_of(&s)))
        .expect("取这个会话的裁决队")
}

/// 一条围栏式的提问（选项 id 就是围栏那两条契约）。
fn request() -> Ask {
    Ask {
        role: "tools".to_string(),
        name: "a".to_string(),
        title: "围栏的这一环装不上，这次调用怎么跑？".to_string(),
        body: "按规则不许悄悄按无围栏执行。".to_string(),
        detail: "模块目录（C:/mods/a）授不上：写 DACL 失败（错误码 5）".to_string(),
        options: vec![
            (
                OPT_FENCE_UNFENCED.to_string(),
                "本轮无围栏跑一次".to_string(),
            ),
            (OPT_FENCE_ABORT.to_string(), "放弃这次调用".to_string()),
        ],
        on_unanswered: None,
    }
}

/// 一条问题推进这条会话的裁决队、**阻塞**等回答：用户按**同一条回答命令**作答，
/// 端口把选中的**选项 id** 还给发起方（落盘由外送出口一起做，生产那条路由 permission 的用例钉）。
#[test]
fn ask_pushes_a_card_and_blocks_until_the_answer_command_answers_it() {
    let (handle, ops, sid) = session_case();
    let door = door_of(&handle, &sid);
    let seen: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let ask = Arc::new(SessionAsk::new(
        Arc::clone(&door),
        &sid,
        Box::new({
            let seen = Arc::clone(&seen);
            move |ev: SessionEvent| seen.lock().expect("锁").push(ev)
        }),
        handle.clone(),
    ));
    // 发起方阻塞且不设超时：放工作线程上；主线程**等卡真的进队**再作答。
    // 反过来（用有上限的轮询线程去答）会在这格永久挂住——全量串行里偶发的死等就是这么来的。
    let outcome: Arc<Mutex<Option<AskOutcome>>> = Arc::new(Mutex::new(None));
    let _asker = {
        let ask = Arc::clone(&ask);
        let outcome = Arc::clone(&outcome);
        std::thread::spawn(move || {
            let got = ask.ask(&request());
            *outcome.lock().expect("锁") = Some(got);
        })
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let queued = loop {
        if let Ok(Some(q)) = ops.sessions.open_queue(&sid) {
            break q;
        }
        assert!(std::time::Instant::now() < deadline, "卡要进这条会话的队");
        std::thread::sleep(std::time::Duration::from_millis(2));
    };
    assert_eq!(queued.card.envelope.role, "tools", "谁在问：工具层");
    assert_eq!(queued.card.envelope.name, "a");
    assert!(
        queued.card.message.detail.contains("错误码 5"),
        "详情要写清缺什么前提：{}",
        queued.card.message.detail
    );
    ops.sessions
        .answer_card(&sid, &queued.card.id, OPT_FENCE_UNFENCED, "")
        .expect("按选项作答");
    while outcome.lock().expect("锁").is_none() {
        assert!(std::time::Instant::now() < deadline, "ask 没有被作答解开");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(
        outcome.lock().expect("锁").take(),
        Some(AskOutcome::Chosen(OPT_FENCE_UNFENCED.to_string())),
        "选中的选项 id 要还给发起方"
    );
    assert!(door.is_empty(), "答完不再挂着");
    let pushed = seen.lock().expect("锁");
    assert!(
        pushed
            .iter()
            .any(|e| matches!(e, SessionEvent::DecisionCard { gate, .. } if gate == "tool_ask")),
        "卡要外送（与快照同源）"
    );
}

/// **构不出可用选项**（空选项集）时：**不发起裁决**，改为**停掉这个会话 + 落一条警告**
/// （契约禁止置灰：没有一条真能执行的选项就不该推一张卡出来）。
#[test]
fn no_options_means_halt_instead_of_asking() {
    let (handle, ops, sid) = session_case();
    let door = door_of(&handle, &sid);
    let seen: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let ask = SessionAsk::new(
        Arc::clone(&door),
        &sid,
        Box::new({
            let seen = Arc::clone(&seen);
            move |ev: SessionEvent| seen.lock().expect("锁").push(ev)
        }),
        handle.clone(),
    );
    let mut empty = request();
    empty.options.clear();
    assert_eq!(
        ask.ask(&empty),
        AskOutcome::NoOptions,
        "构不出可用选项 = 没有回答（端口已停会话 + 落警告）"
    );
    assert!(door.is_empty(), "空选项集不该推卡");
    let pushed = seen.lock().expect("锁");
    assert!(
        pushed.iter().any(|e| matches!(e, SessionEvent::Notice(n)
            if n.contains("做不下去") && n.contains("错误码 5"))),
        "要落一条警告说清哪一环做不下去：{:?}",
        *pushed
    );
    drop(pushed);
    // 会话被停：之后的派发被拒（用户「继续」才解冻——那之前不再往下走）。
    assert!(
        ops.sessions.say(&sid, "再来", Output::Final).is_err(),
        "停下的会话不再派发"
    );
    // 停过一次之后不再推卡：会话已停，再推一张没人能做主的卡只会让人困惑。
    assert_eq!(
        ask.ask(&request()),
        AskOutcome::NoAnswer,
        "停过之后不再发起裁决"
    );
    assert!(door.is_empty(), "停过之后不许再推卡");
}

/// 没有可回答的前端（没有端口）时也走同一条：按**发起方自己声明的**默认项收场；声明不算数则如实按"没人答"。
#[test]
fn no_answerer_falls_back_to_the_declared_option_or_refuses() {
    use crate::kernel::ports::ask_user;
    let mut with_default = request();
    with_default.on_unanswered = Some(OPT_FENCE_ABORT.to_string());
    assert_eq!(
        ask_user(None, &with_default),
        AskOutcome::Defaulted(OPT_FENCE_ABORT.to_string()),
        "没人答 + 声明了默认项 = 按它收场（不是用户答的）"
    );
    assert_eq!(
        ask_user(None, &request()),
        AskOutcome::NoAnswer,
        "没人答 + 没声明默认项 = 不办（fail-closed）"
    );
    let mut bogus = request();
    bogus.on_unanswered = Some("不在卡上的选项".to_string());
    assert_eq!(
        ask_user(None, &bogus),
        AskOutcome::NoAnswer,
        "声明不属于本卡选项集就不算数，不静默改写"
    );
}

/// 端口上**用户停止 ≠ 没人答**：停止按拒绝收场，**绝不能被声明的默认项吞成放行**。
#[test]
fn stopped_is_not_the_same_as_nobody_answered() {
    let (handle, _ops, sid) = session_case();
    let door = door_of(&handle, &sid);
    let ask = Arc::new(SessionAsk::new(
        Arc::clone(&door),
        &sid,
        Box::new(|_ev: SessionEvent| {}),
        handle.clone(),
    ));
    let mut request = request();
    request.on_unanswered = Some(OPT_FENCE_UNFENCED.to_string());
    // ask 阻塞且**不设超时**，所以放到工作线程上；主线程先确认卡已进队，再整队作废。
    // 反过来（作废线程可能先于入队就放弃）会把这格永久挂住——那正是全量串行里偶发的死等。
    let outcome: Arc<Mutex<Option<AskOutcome>>> = Arc::new(Mutex::new(None));
    let _asker = {
        let ask = Arc::clone(&ask);
        let outcome = Arc::clone(&outcome);
        std::thread::spawn(move || {
            let got = ask.ask(&request);
            *outcome.lock().expect("锁") = Some(got);
        })
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while door.is_empty() {
        assert!(std::time::Instant::now() < deadline, "卡没有进队");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(door.void().len(), 1, "作废的就是刚进队那张卡");
    while outcome.lock().expect("锁").is_none() {
        assert!(std::time::Instant::now() < deadline, "ask 没有被停止解开");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(
        outcome.lock().expect("锁").take(),
        Some(AskOutcome::Stopped),
        "用户停止 = 拒绝：声明的默认项不能把它吞成放行"
    );
}

/// 没人答（这一趟没有回答者）时端口按声明的默认项收场，并在会话里**如实记一句"不是用户答的"**。
#[test]
fn port_applies_the_declared_default_and_says_it_was_not_the_user() {
    let (handle, _ops, sid) = session_case();
    let door = door_of(&handle, &sid);
    let seen: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let ask = Arc::new(SessionAsk::new(
        Arc::clone(&door),
        &sid,
        Box::new({
            let seen = Arc::clone(&seen);
            move |ev: SessionEvent| seen.lock().expect("锁").push(ev)
        }),
        handle.clone(),
    ));
    let mut request = request();
    request.on_unanswered = Some(OPT_FENCE_UNFENCED.to_string());
    // 没人答：主线程等卡进队后按"这一趟没有回答者"重建整队（不用有上限的轮询线程）。
    let outcome: Arc<Mutex<Option<AskOutcome>>> = Arc::new(Mutex::new(None));
    let _asker = {
        let ask = Arc::clone(&ask);
        let outcome = Arc::clone(&outcome);
        std::thread::spawn(move || {
            let got = ask.ask(&request);
            *outcome.lock().expect("锁") = Some(got);
        })
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while door.is_empty() {
        assert!(std::time::Instant::now() < deadline, "卡没有进队");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    door.rebuild(0);
    while outcome.lock().expect("锁").is_none() {
        assert!(std::time::Instant::now() < deadline, "ask 没有被解开");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(
        outcome.lock().expect("锁").take(),
        Some(AskOutcome::Defaulted(OPT_FENCE_UNFENCED.to_string())),
        "没人答 + 声明了默认项 = 按它收场"
    );
    let pushed = seen.lock().expect("锁");
    assert!(
        pushed.iter().any(|e| matches!(e, SessionEvent::Notice(n)
            if n.contains("没人能答") && n.contains("不是用户答的"))),
        "要如实记一句“不是用户答的”：{:?}",
        *pushed
    );
}
