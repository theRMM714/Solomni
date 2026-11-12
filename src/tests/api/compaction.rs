//! 目的：压缩与恢复——发送视图换成一篇滚动摘要，删掉压缩行即回到全量。
//! 管：自动/手动触发、保留尾巴、预算与百分比、模型解析、重启与回档。
//! 不管：会话编辑（`session.rs`）、历史合并（`events.rs`）。
//! 联动：`src/capabilities/session/domain/session.rs` 与 `src/capabilities/conductor/api/handle.rs`。

use super::super::doubles::{model_draft, module_of};
use super::super::single_work;
use crate::capabilities::conductor::api::{ActionCall, Caller, SessionEvent};
use crate::capabilities::conductor::api::Output;
use std::sync::Arc;

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
    let gateway = super::super::RecordingGateway {
        inner: super::super::doubles::ScriptGateway::new(member, vec![]),
        seen: Arc::clone(&seen),
    };
    let handle = crate::capabilities::conductor::api::ConductorHandle::spawn(
        super::super::doubles::core_with_gateway(vec![module_of("a")], gateway),
    )
    .expect("起核心线程");
    let ops = crate::capabilities::conductor::api::Ops::from_handle(&handle);
    let mut st = ops.registry.settings().expect("读设置");
    st.compact_keep_percent = 0;
    st.sandbox.policy.windows.write_acl = true;
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
        .expect("说一句");

    let first = ops.sessions.compact(&sid).expect("第一次压缩");
    let (b1, head1, _o1) = handle.events().snapshot(Some(&sid), 0);
    let s1: Vec<&SessionEvent> = b1.iter().flat_map(|l| l.events.iter()).collect();
    assert!(
        s1.iter()
            .any(|e| matches!(e, SessionEvent::Transcript(lines)
            if lines.iter().any(|l| l.kind == "compacted" && l.line == "摘要一"))),
        "第一次压缩该如实落一条压缩记录行（头部 {}）",
        first.head
    );
    let second = ops.sessions.compact(&sid).expect("第二次压缩");
    let (b2, _head2, _o2) = handle.events().snapshot(Some(&sid), head1);
    let s2: Vec<&SessionEvent> = b2.iter().flat_map(|l| l.events.iter()).collect();
    assert!(
        s2.iter()
            .any(|e| matches!(e, SessionEvent::Transcript(lines)
            if lines.iter().any(|l| l.kind == "compacted" && l.line == "摘要二"))),
        "第二次压缩该如实落一条压缩记录行（头部 {}）",
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
    let gateway = super::super::RecordingGateway {
        inner: super::super::doubles::ScriptGateway::new(member, vec![]),
        seen: Arc::clone(&seen),
    };
    let handle = crate::capabilities::conductor::api::ConductorHandle::spawn(
        super::super::doubles::core_with_gateway(vec![module_of("a")], gateway),
    )
    .expect("起核心线程");
    let ops = crate::capabilities::conductor::api::Ops::from_handle(&handle);
    // 阈值调到 1%：预算 = 32000 × 1% × 4 ≈ 1280 字符，上面那条长回复会超。
    let mut st = ops.registry.settings().expect("读设置");
    st.compact_at_percent = 1;
    st.compact_keep_percent = 0;
    // Windows 上 require + write_acl=false 是非法组合；这条只验压缩阈值，先给授权。
    st.sandbox.policy.windows.write_acl = true;
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

#[test]
fn compaction_keeps_the_recent_tail_and_the_new_user_line() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let long = "长".repeat(1500);
    let mut member = std::collections::BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            long.clone(),
            "{\"type\":\"tool\",\"name\":\"compact\",\"args\":{\"summary\":\"保留摘要\"}}"
                .to_string(),
            "第二轮回复".to_string(),
        ],
    );
    let gateway = super::super::RecordingGateway {
        inner: super::super::doubles::ScriptGateway::new(member, vec![]),
        seen: Arc::clone(&seen),
    };
    let handle = crate::capabilities::conductor::api::ConductorHandle::spawn(
        super::super::doubles::core_with_gateway(vec![module_of("a")], gateway),
    )
    .expect("起核心线程");
    let ops = crate::capabilities::conductor::api::Ops::from_handle(&handle);
    let mut st = ops.registry.settings().expect("读设置");
    st.compact_at_percent = 1;
    st.compact_keep_percent = 1;
    st.sandbox.policy.windows.write_acl = true;
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
            "第一句话",
            crate::capabilities::conductor::api::Output::Final,
        )
        .expect("第一轮");
    ops.sessions
        .say(
            &sid,
            "第二句话",
            crate::capabilities::conductor::api::Output::Final,
        )
        .expect("第二轮（开头该压一次）");

    let all = seen.lock().expect("锁").clone();
    let last = all.last().expect("至少问过一次").clone();
    assert!(
        last.iter().any(|c| c.contains("保留摘要")),
        "压缩后的发送视图该带上摘要：{last:?}"
    );
    assert!(
        last.iter().any(|c| c.contains(&long)),
        "最近的大回复该原样保留：{last:?}"
    );
    assert!(
        last.iter().any(|c| c.contains("第二句话")),
        "本轮新用户消息必须原样保留：{last:?}"
    );
    assert!(
        !last.iter().any(|c| c.contains("第一句话")),
        "更早的内容该进摘要、不再原文发送：{last:?}"
    );
}

#[test]
fn manual_compaction_reports_when_there_is_nothing_to_compact() {
    let mut member = std::collections::BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec!["{\"type\":\"say\",\"text\":\"收到\"}".to_string()],
    );
    let handle = crate::capabilities::conductor::api::ConductorHandle::spawn(
        super::super::doubles::core_with_gateway(
            vec![module_of("a")],
            super::super::doubles::ScriptGateway::new(member, vec![]),
        ),
    )
    .expect("起核心线程");
    let ops = crate::capabilities::conductor::api::Ops::from_handle(&handle);
    let mut st = ops.registry.settings().expect("读设置");
    st.compact_at_percent = 0;
    st.compact_keep_percent = 15;
    st.sandbox.policy.windows.write_acl = true;
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
            "你好",
            crate::capabilities::conductor::api::Output::Final,
        )
        .expect("第一轮");
    ops.sessions.compact(&sid).expect("手动压缩");
    let (batches, _, _) = handle.events().snapshot(Some(&sid), 0);
    let events: Vec<&SessionEvent> = batches.iter().flat_map(|b| b.events.iter()).collect();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::Notice(t) if t.contains("没有可压缩的内容"))),
        "没有可压缩内容时该明确提示：{events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SessionEvent::Transcript(lines)
            if lines.iter().any(|l| l.kind == "compacted"))),
        "没有可压缩内容时不该落压缩记录行"
    );
}

#[test]
fn slash_compact_message_runs_the_command_instead_of_the_model() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut member = std::collections::BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "第一件事".to_string(),
            "{\"type\":\"tool\",\"name\":\"compact\",\"args\":{\"summary\":\"摘要\"}}".to_string(),
        ],
    );
    let gateway = super::super::RecordingGateway {
        inner: super::super::doubles::ScriptGateway::new(member, vec![]),
        seen: Arc::clone(&seen),
    };
    let handle = crate::capabilities::conductor::api::ConductorHandle::spawn(
        super::super::doubles::core_with_gateway(vec![module_of("a")], gateway),
    )
    .expect("起核心线程");
    let ops = crate::capabilities::conductor::api::Ops::from_handle(&handle);
    let mut st = ops.registry.settings().expect("读设置");
    st.compact_at_percent = 0;
    st.compact_keep_percent = 0;
    st.sandbox.policy.windows.write_acl = true;
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
            "先说一句",
            crate::capabilities::conductor::api::Output::Final,
        )
        .expect("第一轮");
    let acted = ops
        .actions
        .act(ActionCall {
            id: "send_message".to_string(),
            args: serde_json::json!({ "session_id": sid, "text": "/compact" }),
            caller: Caller::User,
            out: Output::Final,
        })
        .expect("斜杠命令该走动作");
    assert!(matches!(
        acted,
        crate::capabilities::conductor::api::Acted::Advanced(_)
    ));

    let (batches, _, _) = handle.events().snapshot(Some(&sid), 0);
    let events: Vec<&SessionEvent> = batches.iter().flat_map(|b| b.events.iter()).collect();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::Transcript(lines)
            if lines.iter().any(|l| l.kind == "compacted"))),
        "斜杠命令该落压缩记录行：{events:?}"
    );
    let all = seen.lock().expect("锁").clone();
    assert!(
        !all.iter()
            .any(|msgs| msgs.iter().any(|c| c.contains("/compact"))),
        "命令本身不该发给模型：{all:?}"
    );
}

#[test]
fn compact_action_takes_only_session_id() {
    let mut member = std::collections::BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "第一件事".to_string(),
            "{\"type\":\"tool\",\"name\":\"compact\",\"args\":{\"summary\":\"摘要\"}}".to_string(),
        ],
    );
    let handle = crate::capabilities::conductor::api::ConductorHandle::spawn(
        super::super::doubles::core_with_gateway(
            vec![module_of("a")],
            super::super::doubles::ScriptGateway::new(member, vec![]),
        ),
    )
    .expect("起核心线程");
    let ops = crate::capabilities::conductor::api::Ops::from_handle(&handle);
    let mut st = ops.registry.settings().expect("读设置");
    st.compact_at_percent = 0;
    st.compact_keep_percent = 0;
    st.sandbox.policy.windows.write_acl = true;
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
            "先说一句",
            crate::capabilities::conductor::api::Output::Final,
        )
        .expect("第一轮");
    ops.actions
        .act(ActionCall {
            id: "compact".to_string(),
            args: serde_json::json!({ "session_id": sid }),
            caller: Caller::User,
            out: Output::Final,
        })
        .expect("动作 /compact 只该要 session_id");
    let (batches, _, _) = handle.events().snapshot(Some(&sid), 0);
    let events: Vec<&SessionEvent> = batches.iter().flat_map(|b| b.events.iter()).collect();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::Transcript(lines)
            if lines.iter().any(|l| l.kind == "compacted"))),
        "动作 /compact 该落压缩记录行：{events:?}"
    );
}

#[test]
fn lowering_keep_percent_applies_to_live_manual_compaction() {
    let mut member = std::collections::BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "第一件事".to_string(),
            "{\"type\":\"tool\",\"name\":\"compact\",\"args\":{\"summary\":\"摘要\"}}".to_string(),
        ],
    );
    let handle = crate::capabilities::conductor::api::ConductorHandle::spawn(
        super::super::doubles::core_with_gateway(
            vec![module_of("a")],
            super::super::doubles::ScriptGateway::new(member, vec![]),
        ),
    )
    .expect("起核心线程");
    let ops = crate::capabilities::conductor::api::Ops::from_handle(&handle);
    let mut st = ops.registry.settings().expect("读设置");
    st.compact_at_percent = 0;
    st.compact_keep_percent = 15;
    st.sandbox.policy.windows.write_acl = true;
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
            "先说一句",
            crate::capabilities::conductor::api::Output::Final,
        )
        .expect("第一轮");
    // 活会话是在 keep=15 时建的；这里设为 0，手动压缩必须按新值重算。
    let mut st = ops.registry.settings().expect("读设置");
    st.compact_keep_percent = 0;
    ops.registry.set_settings(st).expect("改保留比例");
    ops.actions
        .act(ActionCall {
            id: "compact".to_string(),
            args: serde_json::json!({ "session_id": sid }),
            caller: Caller::User,
            out: Output::Final,
        })
        .expect("手动压缩");
    let (batches, _, _) = handle.events().snapshot(Some(&sid), 0);
    let events: Vec<&SessionEvent> = batches.iter().flat_map(|b| b.events.iter()).collect();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::Transcript(lines)
            if lines.iter().any(|l| l.kind == "compacted"))),
        "保留比例改成 0 后，活会话的手动压缩不该再说没有可压缩内容：{events:?}"
    );
}

#[test]
fn compaction_model_is_global_and_resolved_at_trigger() {
    let mut core = super::super::doubles::core_with_gateway(
        vec![module_of("a")],
        super::super::doubles::gw(std::collections::BTreeMap::new(), vec![]),
    );
    let sid = core
        .create_work(single_work("w", &["a"]))
        .expect("建会话")
        .sid;
    core.registry_mut()
        .model_upsert(model_draft("m2", "M2", "m2", "p", "", 32_000))
        .expect("登记第二个模型");
    let mut st = core.registry().app_settings();
    st.compact_model = "m2".to_string();
    st.sandbox.policy.windows.write_acl = true;
    core.registry_mut().set_app_settings(st).expect("写设置");
    core.sync_compact_choice();

    let session = core.take_single(&sid).expect("取会话");
    let spec = session
        .compact_chat()
        .expect("读压缩通道")
        .expect("独立总结模型");
    assert_eq!(spec.model, "m2");
    assert_eq!(spec.window, 32_000);
    core.put_single(&sid, session);

    // 与会话同模型：复用会话通道（None）。
    let mut st = core.registry().app_settings();
    st.compact_model = "m".to_string();
    core.registry_mut().set_app_settings(st).expect("写设置");
    core.sync_compact_choice();
    let session = core.take_single(&sid).expect("取会话");
    assert!(
        session.compact_chat().expect("读压缩通道").is_none(),
        "同模型不该另建通道"
    );
    core.put_single(&sid, session);

    // 未登记模型：保存即拒绝。
    let mut st = core.registry().app_settings();
    st.compact_model = "nope".to_string();
    assert!(
        core.registry_mut().set_app_settings(st).is_err(),
        "未登记的压缩模型该被拒绝"
    );

    // 仍被压缩设置引用的模型：删除即拒绝。
    let mut st = core.registry().app_settings();
    st.compact_model = "m2".to_string();
    core.registry_mut().set_app_settings(st).expect("写设置");
    let err = core.registry_mut().model_remove("m2").unwrap_err();
    assert!(err.contains("压缩用模型"), "{err}");
}

#[test]
fn deleting_the_compaction_line_restores_the_full_send_view() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut member = std::collections::BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "第一件事".to_string(),
            "{\"type\":\"tool\",\"name\":\"compact\",\"args\":{\"summary\":\"摘要\"}}".to_string(),
            "收尾".to_string(),
        ],
    );
    let gateway = super::super::RecordingGateway {
        inner: super::super::doubles::ScriptGateway::new(member, vec![]),
        seen: Arc::clone(&seen),
    };
    let handle = crate::capabilities::conductor::api::ConductorHandle::spawn(
        super::super::doubles::core_with_gateway(vec![module_of("a")], gateway),
    )
    .expect("起核心线程");
    let ops = crate::capabilities::conductor::api::Ops::from_handle(&handle);
    let mut st = ops.registry.settings().expect("读设置");
    st.compact_at_percent = 0; // 只测手动压缩与删除
    st.compact_keep_percent = 0;
    st.sandbox.policy.windows.write_acl = true;
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
            "先说一句",
            crate::capabilities::conductor::api::Output::Final,
        )
        .expect("第一轮");
    ops.sessions.compact(&sid).expect("手动压缩");
    let (batches, _, _) = handle.events().snapshot(Some(&sid), 0);
    let marker = batches
        .iter()
        .flat_map(|b| b.events.iter())
        .find_map(|e| match e {
            SessionEvent::Transcript(lines) => {
                lines.iter().find(|l| l.kind == "compacted").map(|l| l.id)
            }
            _ => None,
        })
        .expect("压缩记录行");
    ops.sessions
        .rewind(
            &sid,
            crate::capabilities::conductor::api::RewindTarget::Delete(marker),
        )
        .expect("删除压缩记录行");
    ops.sessions
        .say(
            &sid,
            "继续",
            crate::capabilities::conductor::api::Output::Final,
        )
        .expect("删除后再问一轮");

    let all = seen.lock().expect("锁").clone();
    let last = all.last().expect("至少问过一次").clone();
    assert!(
        last.iter().any(|c| c.contains("第一件事")),
        "删除压缩记录行后旧内容该回到发送视图：{last:?}"
    );
    assert!(
        !last.iter().any(|c| c.contains("此前内容摘要")),
        "删除压缩记录行后摘要该消失：{last:?}"
    );
}

#[test]
fn auto_compaction_uses_provider_usage_when_it_is_available() {
    use crate::capabilities::llm::api::TokenUsage;
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut member = std::collections::BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "先做".to_string(),
            "{\"type\":\"tool\",\"name\":\"compact\",\"args\":{\"summary\":\"用量摘要\"}}"
                .to_string(),
            "继续".to_string(),
        ],
    );
    let gateway = super::super::RecordingGateway {
        inner: super::super::doubles::ScriptGateway::new(member, vec![]).with_usage(TokenUsage::new(
            18_000,
            2_000,
            Some(20_000),
        )),
        seen: Arc::clone(&seen),
    };
    let handle = crate::capabilities::conductor::api::ConductorHandle::spawn(
        super::super::doubles::core_with_gateway(vec![module_of("a")], gateway),
    )
    .expect("起核心线程");
    let ops = crate::capabilities::conductor::api::Ops::from_handle(&handle);
    let mut st = ops.registry.settings().expect("读设置");
    st.compact_at_percent = 10; // 128K × 10% = 12.8K < 20K
    st.compact_keep_percent = 0;
    st.sandbox.policy.windows.write_acl = true;
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
            "第一句",
            crate::capabilities::conductor::api::Output::Final,
        )
        .expect("第一轮");
    ops.sessions
        .say(
            &sid,
            "第二句",
            crate::capabilities::conductor::api::Output::Final,
        )
        .expect("第二轮该先压一次");

    let all = seen.lock().expect("锁").clone();
    assert!(
        all.iter()
            .any(|msgs| msgs.iter().any(|c| c.contains("用量摘要"))),
        "精确 usage 超阈值时该自动压：{all:?}"
    );
    let last = all.last().expect("至少问过一次").clone();
    assert!(
        last.iter().any(|c| c.contains("用量摘要")),
        "压缩后的发送视图该带摘要：{last:?}"
    );
}

#[test]
fn compaction_survives_a_restart_and_rewinds_back_through_the_point() {
    let mut core = super::super::doubles::core_with(
        vec![module_of("a")],
        super::super::doubles::gw(std::collections::BTreeMap::new(), Vec::new()),
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
    // 两份行 + 一条压缩记录行（覆盖到 next+2）+ 之后的一行：模拟「压过之后再重启」的落盘历史。
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
        &[serde_json::json!({
            "type": "transcript",
            "lines": [{
                "id": next + 2,
                "reply": next + 2,
                "line": "前两句的摘要",
                "kind": "compacted",
                "system": true,
                "payload": {"up_to": next + 2},
                "speaker": "",
                "verb": "",
                "turn": 0
            }]
        })],
    )
    .expect("落压缩记录行");
    core.history_append(
        &sid,
        &[serde_json::json!({"type": "transcript", "lines": [line(next + 3, "user", "之后的新内容")]})],
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
        rebuilt.compacted_upto,
        next + 2,
        "重建也要恢复压缩点（发送视图靠它）"
    );
    assert_eq!(
        rebuilt.compacted_line,
        next + 2,
        "重建也要恢复压缩记录行（回档分流靠它）"
    );
    drop(rebuilt);
    core.abort_running(&sid);

    // ② 回档跨越压缩点：会话在表里且压缩点 > 目标 → `rewind` 走重建，摘要不再生效。
    core.ensure_session(&sid).expect("把重建结果装回表里");
    core.rewind(
        &sid,
        crate::capabilities::conductor::api::RewindTarget::Delete(next),
    )
    .expect("回档到压缩点之前");
    let back = core.take_single(&sid).expect("取回回档后的会话");
    let shown_back = shown(back.dialogue());
    assert!(
        !shown_back.iter().any(|c| c.contains("此前内容摘要")),
        "回档到压缩点之前：摘要该消失（内容回到压缩前）：{shown_back:?}"
    );
    assert_eq!(back.compacted_upto, 0, "压缩点该一起回退掉");
}
