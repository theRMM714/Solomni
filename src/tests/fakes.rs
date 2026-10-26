//! FakeChat / DemoGateway 的独立契约测试（docs/testing/doubles.md 的 Chat 与 ChatGateway 行）。
//! 两者都是**测试专用**替身：生产不装配，也不存在"没有模型就回落演示"这条路。

use crate::capabilities::llm::api::Channel;
use crate::capabilities::llm::api::{parse, Verb};
use crate::capabilities::llm::api::{Chat, Chunk, CompleteOpts, Msg};
use crate::capabilities::llm::detail::fake_chat::{DemoGateway, FakeChat};
use crate::capabilities::llm::ports::ChatGateway;

/// 故意指向不可路由地址（TEST-NET-1）：任何真实拨号都会失败或超时——「没有网络依赖」因此可观察。
fn unreachable_channel() -> Channel {
    Channel {
        base_url: "http://192.0.2.1:9".to_string(),
        api_key: "k".to_string(),
        model: "m".to_string(),
    }
}

/// 跑一次调用，收集 on 收到的事件（顺序与内容都要可断言）。
fn call(chat: &mut dyn Chat, stream: bool) -> (String, Vec<String>) {
    let mut seen: Vec<String> = Vec::new();
    let out = chat.complete(
        &[Msg::user("你好")],
        CompleteOpts::plain(stream),
        &mut |c| {
            seen.push(match c {
                Chunk::Start => "start".to_string(),
                Chunk::Text(t) => format!("text:{}", t),
                Chunk::Reasoning(r) => format!("reasoning:{}", r),
            });
            true
        },
    );
    (out.raw, seen)
}

// ---------- FakeChat：成功 / 空结果 / 记录 ----------

#[test]
fn fake_chat_replays_in_order_then_repeats_the_last() {
    let mut chat = FakeChat::new(vec!["一".to_string(), "二".to_string()]);
    assert_eq!(call(&mut chat, false).0, "一");
    assert_eq!(call(&mut chat, false).0, "二");
    assert_eq!(
        call(&mut chat, false).0,
        "二",
        "末条重复兜底：调用次数超过脚本长度也不能空手"
    );
}

#[test]
fn fake_chat_empty_script_is_an_empty_answer() {
    let mut chat = FakeChat::new(Vec::new());
    let (out, seen) = call(&mut chat, false);
    assert_eq!(out, "", "空脚本 = 空串，不是 panic 也不是编造内容");
    assert!(seen.is_empty());
}

#[test]
fn fake_chat_records_every_call_verbatim() {
    let mut chat = FakeChat::new(vec!["x".to_string()]);
    let _ = call(&mut chat, false);
    let _ = call(&mut chat, false);
    assert_eq!(chat.calls.len(), 2, "每次调用都要留现场");
    assert_eq!(chat.calls[0].len(), 1);
    assert_eq!(chat.calls[0][0].role, "user");
    assert_eq!(
        chat.calls[0][0].content, "你好",
        "记录的是原文，不是渲染后的提示词"
    );
}

// ---------- FakeChat：流式与中止 ----------

#[test]
fn fake_chat_without_stream_never_calls_back() {
    let mut chat = FakeChat::new(vec!["安静".to_string()]);
    let (out, seen) = call(&mut chat, false);
    assert_eq!(out, "安静");
    assert!(seen.is_empty(), "非流式实现不回调（Chat 端口契约）");
}

#[test]
fn fake_chat_streaming_emits_start_then_one_text_chunk() {
    let mut chat = FakeChat::new(vec!["流".to_string()]);
    let (out, seen) = call(&mut chat, true);
    assert_eq!(
        seen,
        vec!["start", "text:流"],
        "先 Start 后 Text，且只回调一次"
    );
    assert_eq!(out, "流", "返回值与流出的正文一致");
}

#[test]
fn fake_chat_abort_at_start_returns_empty_and_emits_nothing_more() {
    let mut chat = FakeChat::new(vec!["不该出现".to_string()]);
    let mut seen: Vec<String> = Vec::new();
    let out = chat
        .complete(&[Msg::user("停")], CompleteOpts::plain(true), &mut |c| {
            seen.push(format!("{:?}", c));
            false
        })
        .raw;
    assert_eq!(out, "", "Start 阶段中止 = 还没产出正文");
    assert_eq!(seen.len(), 1, "中止后不得再回调：{:?}", seen);
}

#[test]
fn fake_chat_abort_at_text_keeps_the_produced_text() {
    let mut chat = FakeChat::new(vec!["半句".to_string()]);
    let mut seen: Vec<String> = Vec::new();
    let out = chat
        .complete(&[Msg::user("停")], CompleteOpts::plain(true), &mut |c| {
            let is_text = matches!(c, Chunk::Text(_));
            seen.push(format!("{:?}", c));
            !is_text
        })
        .raw;
    assert_eq!(
        out, "半句",
        "已经吐出的正文照常返回（与 http_chat 同一套中止语义）"
    );
    assert_eq!(seen.len(), 2);
}

// ---------- DemoGateway：测试专用 / 两类通道 / 不需要真实供应商 ----------

#[test]
fn demo_gateway_member_channel_names_the_module_and_replies() {
    let mut chat = DemoGateway.member_channel(&unreachable_channel(), "reviewer");
    let (out, _) = call(chat.as_mut(), false);
    let reply = parse(&out);
    assert!(
        matches!(reply.verb, Verb::Say),
        "测试成员通道回一条发言：{}",
        out
    );
    assert!(reply.text.contains("reviewer"), "{}", out);
}

#[test]
fn demo_gateway_core_channel_replies_with_a_checklist_tool_call() {
    let mut chat = DemoGateway.core_channel(&unreachable_channel());
    let (first, _) = call(chat.as_mut(), false);
    assert!(matches!(parse(&first).verb, Verb::Say), "{}", first);
    // 核心操作走**工具调用**：测试通道发手写信封，清单在 checklist 的参数里。
    let (second, _) = call(chat.as_mut(), false);
    let r = parse(&second);
    let tool = r
        .tools
        .iter()
        .find(|t| t.name == "checklist")
        .unwrap_or_else(|| panic!("验收清单必须走 checklist 工具调用：{}", second));
    let payload: serde_json::Value =
        serde_json::from_str(&tool.args_json).expect("参数必须是合法 JSON");
    assert!(
        payload
            .get("items")
            .and_then(|v| v.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false),
        "{}",
        second
    );
}

#[test]
fn demo_gateway_scripts_are_valid_envelopes() {
    let mut member = DemoGateway.member_channel(&unreachable_channel(), "a");
    let (out, _) = call(member.as_mut(), false);
    assert!(
        !parse(&out).degraded,
        "测试脚本必须是干净信封（模型下一轮会照抄格式）：{}",
        out
    );
    let mut core_chat = DemoGateway.core_channel(&unreachable_channel());
    let (say, _) = call(core_chat.as_mut(), false);
    assert!(!parse(&say).degraded, "{}", say);
    // 核心操作走工具调用：测试通道发的是**手写信封**，载荷与真实通道同形。
    let (list, _) = call(core_chat.as_mut(), false);
    let r = parse(&list);
    assert!(
        !r.degraded && r.tools.iter().any(|t| t.name == "checklist"),
        "验收清单走 checklist 工具调用：{}",
        list
    );
}

#[test]
fn demo_gateway_needs_no_provider_and_does_not_dial_out() {
    // 通道不可路由也立刻给答复：出现演示文案即证明它根本没碰网络。
    let mut with_bogus = DemoGateway.member_channel(&unreachable_channel(), "m");
    let (out, _) = call(with_bogus.as_mut(), false);
    assert!(
        out.contains("（演示）"),
        "测试通道不依赖任何供应商：{}",
        out
    );
    let other = Channel {
        base_url: "http://198.51.100.7:9".to_string(),
        api_key: "另一把".to_string(),
        model: "另一条".to_string(),
    };
    let mut without = DemoGateway.member_channel(&other, "m");
    let (out2, _) = call(without.as_mut(), false);
    assert_eq!(out, out2, "测试答复不随通道变化（说明它根本没用到通道）");
}
