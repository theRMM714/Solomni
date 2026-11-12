//! 目的：HTTP 适配器只对本机环回假供应商说话的真实边界。
//! 管：网关/目录/探针、流式与非流式、超时与取消、工具调用与推理字段、错误分类与密钥不外泄。
//! 不管：真实外网与真实密钥。
//! 联动：假供应商定义在文件内；契约见 docs/testing/doubles.md。

use super::*;

/// 本机假供应商：按脚本逐条应答，跑完/丢弃时停线程。只绑 127.0.0.1。
struct Mock {
    base: String,
    stop: Arc<AtomicBool>,
    hits: Arc<Mutex<Vec<String>>>,
    /// 收到的请求体（与 hits 同序）：探针类用例要断言"到底发出去了什么形状"。
    bodies: Arc<Mutex<Vec<String>>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Mock {
    fn start(replies: Vec<(u16, &'static str, String)>) -> Mock {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("起本机假供应商");
        let addr = server.server_addr().to_ip().expect("TCP 地址");
        let base = format!("http://127.0.0.1:{}", addr.port());
        let stop = Arc::new(AtomicBool::new(false));
        let hits = Arc::new(Mutex::new(Vec::new()));
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let (st, hi, bo) = (Arc::clone(&stop), Arc::clone(&hits), Arc::clone(&bodies));
        let handle = std::thread::spawn(move || {
            let mut served = 0usize;
            while !st.load(Ordering::Relaxed) {
                let mut req = match server.recv_timeout(Duration::from_millis(50)) {
                    Ok(Some(r)) => r,
                    _ => continue,
                };
                hi.lock()
                    .expect("锁")
                    .push(format!("{} {}", req.method(), req.url()));
                let mut body_text = String::new();
                let _ = req.as_reader().read_to_string(&mut body_text);
                bo.lock().expect("锁").push(body_text);
                let (code, ctype, body) = match replies.get(served) {
                    Some(r) => (r.0, r.1, r.2.clone()),
                    None => (200, "application/json", "{}".to_string()),
                };
                served += 1;
                let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], ctype.as_bytes())
                    .expect("响应头");
                let _ = req.respond(
                    tiny_http::Response::from_string(body)
                        .with_status_code(code)
                        .with_header(header),
                );
            }
        });
        Mock {
            base,
            stop,
            hits,
            bodies,
            handle: Some(handle),
        }
    }

    fn channel(&self, key: &str) -> Channel {
        Channel {
            base_url: self.base.clone(),
            api_key: key.to_string(),
            model: "m".to_string(),
        }
    }

    fn requests(&self) -> Vec<String> {
        self.hits.lock().expect("锁").clone()
    }

    fn bodies(&self) -> Vec<String> {
        self.bodies.lock().expect("锁").clone()
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// 按时间片吐 SSE 的本机假供应商：先写响应头，再按步骤写 chunk，用于验首块/空闲超时与取消。
struct StreamMock {
    base: String,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl StreamMock {
    fn start(steps: Vec<(Duration, Vec<u8>)>) -> StreamMock {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("起本机流式假供应商");
        listener.set_nonblocking(true).expect("非阻塞 accept");
        let addr = listener.local_addr().expect("TCP 地址");
        let base = format!("http://127.0.0.1:{}", addr.port());
        let stop = Arc::new(AtomicBool::new(false));
        let st = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            while !st.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(x) => x,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(_) => break,
                };
                // macOS / BSD 的 accept 会让连接继承 listener 的 O_NONBLOCK（Linux 不继承）：
                // 不恢复阻塞，write_all 可能返回 WouldBlock，连接就会在 chunked 终止块之前被关掉。
                stream
                    .set_nonblocking(false)
                    .expect("accepted stream 必须按阻塞读写");
                // 请求头读一次就够：后续 body 不参与假供应商的判断。
                let mut buf = [0u8; 8192];
                let _ = stream.read(&mut buf);
                // 先发响应头：客户端的 send() 立刻返回；首块预算从这里开始算。
                if stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n",
                    )
                    .is_err()
                {
                    break;
                }
                let _ = stream.flush();
                let mut broken = false;
                for (delay, bytes) in steps.iter() {
                    std::thread::sleep(*delay);
                    let head = format!("{:X}\r\n", bytes.len());
                    if stream.write_all(head.as_bytes()).is_err()
                        || stream.write_all(bytes).is_err()
                        || stream.write_all(b"\r\n").is_err()
                    {
                        broken = true;
                        break;
                    }
                    let _ = stream.flush();
                }
                if !broken {
                    let _ = stream.write_all(b"0\r\n\r\n");
                    let _ = stream.flush();
                }
            }
        });
        StreamMock {
            base,
            stop,
            handle: Some(handle),
        }
    }

    /// 以 /v1 结尾：候选只有一条，超时用例不会被端点回落放大成两倍等待。
    fn channel_v1(&self, key: &str) -> Channel {
        Channel {
            base_url: format!("{}/v1", self.base),
            api_key: key.to_string(),
            model: "m".to_string(),
        }
    }
}

impl Drop for StreamMock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn completion_body(content: &str) -> String {
    serde_json::json!({"choices": [{"message": {"content": content}}]}).to_string()
}

fn sse_delta(content: &str) -> String {
    format!(
        "data: {}\n\n",
        serde_json::json!({"choices": [{"delta": {"content": content}}]})
    )
}

/// 收尾分片：只带 finish_reason（供应商通常这样收尾）。
fn sse_finish(reason: &str) -> String {
    format!(
        "data: {}\n\n",
        serde_json::json!({"choices": [{"delta": {}, "finish_reason": reason}]})
    )
}

fn gateway() -> HttpGateway {
    let log: Arc<dyn Log + Send + Sync> = Arc::new(NoopLog);
    HttpGateway::with_log(log, memo_new())
}

#[test]
fn http_gateway_talks_to_a_real_endpoint_on_loopback() {
    let mock = Mock::start(vec![(
        200,
        "application/json",
        completion_body("来自本机假供应商"),
    )]);
    let mut chat = gateway().member_channel(&mock.channel("k"), "a");
    let out = chat
        .complete(
            &[Msg::user("你好")],
            CompleteOpts::plain(false),
            &mut |_| true,
        )
        .raw;
    assert_eq!(out, "来自本机假供应商");
    let hits = mock.requests();
    assert_eq!(hits.len(), 1, "端点一次命中就不再探测候选：{:?}", hits);
    assert!(hits[0].contains("/chat/completions"), "{:?}", hits);
}

#[test]
fn http_chat_streams_chunks_and_stops_when_the_caller_aborts() {
    let body = format!("{}{}data: [DONE]\n\n", sse_delta("你"), sse_delta("好"));
    let mock = Mock::start(vec![(200, "text/event-stream", body)]);
    let mut chat = gateway().member_channel(&mock.channel("k"), "a");
    let mut seen: Vec<String> = Vec::new();
    let out = chat
        .complete(
            &[Msg::user("讲两句")],
            CompleteOpts::plain(true),
            &mut |c| {
                seen.push(match c {
                    Chunk::Start => "start".to_string(),
                    Chunk::Text(t) => format!("text:{}", t),
                    Chunk::Reasoning(r) => format!("reasoning:{}", r),
                });
                true
            },
        )
        .raw;
    assert_eq!(out, "你好", "流式返回完整正文");
    assert_eq!(
        seen,
        vec!["start", "text:你", "text:好"],
        "先 Start，再逐片 Text"
    );

    // 中止：Start 放行、第一片 Text 后要求停止 → 立即停下并保留已产出的正文。
    let body2 = format!("{}{}data: [DONE]\n\n", sse_delta("你"), sse_delta("好"));
    let mock2 = Mock::start(vec![(200, "text/event-stream", body2)]);
    let mut chat2 = gateway().member_channel(&mock2.channel("k"), "a");
    let mut seen2: Vec<String> = Vec::new();
    let out2 = chat2
        .complete(&[Msg::user("停")], CompleteOpts::plain(true), &mut |c| {
            let is_text = matches!(c, Chunk::Text(_));
            seen2.push(format!("{:?}", c));
            !is_text
        })
        .raw;
    assert_eq!(out2, "你", "中止后返回已产出的正文，且不重开候选");
    assert_eq!(seen2.len(), 2, "中止后不再回调：{:?}", seen2);
    assert_eq!(mock2.requests().len(), 1, "中止不得把流重新拉起来");
}

/// usage 是两个方向的真实协议事实：非流式在响应体，流式在最后一个只有 usage 的分片。
#[test]
fn http_chat_reads_usage_from_stream_and_non_stream() {
    let non_stream = serde_json::json!({
        "choices": [{"message": {"content": "答"}}],
        "usage": {"prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120}
    })
    .to_string();
    let mock = Mock::start(vec![(200, "application/json", non_stream)]);
    let mut chat = gateway().member_channel(&mock.channel("k"), "a");
    let done = chat.complete(&[Msg::user("问")], CompleteOpts::plain(false), &mut |_| {
        true
    });
    assert_eq!(done.raw, "答");
    let usage = done.usage.expect("非流式 usage");
    assert_eq!(
        (
            usage.prompt_tokens,
            usage.completion_tokens,
            usage.total_tokens
        ),
        (100, 20, 120)
    );
    assert_eq!(chat.last_usage().expect("通道记住 usage").total_tokens, 120);

    // 流式：正文分片之后，供应商发一个 choices 为空、只带 usage 的分片。
    let sse = format!(
        "{}{}data: {}\n\ndata: [DONE]\n\n",
        sse_delta("你"),
        sse_delta("好"),
        serde_json::json!({"choices": [], "usage": {"prompt_tokens": 200, "completion_tokens": 30, "total_tokens": 230}})
    );
    let mock2 = Mock::start(vec![(200, "text/event-stream", sse)]);
    let mut chat2 = gateway().member_channel(&mock2.channel("k"), "a");
    let mut pieces = Vec::new();
    let done2 = chat2.complete(&[Msg::user("问")], CompleteOpts::plain(true), &mut |c| {
        if let Chunk::Text(t) = c {
            pieces.push(t);
        }
        true
    });
    assert_eq!(done2.raw, "你好");
    assert_eq!(pieces, vec!["你".to_string(), "好".to_string()]);
    let usage2 = done2.usage.expect("流式 usage");
    assert_eq!(usage2.total_tokens, 230);
    assert_eq!(
        chat2
            .last_usage()
            .expect("通道记住流式 usage")
            .prompt_tokens,
        200
    );
}

/// 供应商不接受 stream_options：去掉参数重试一次（不再重复正文），拿不到 usage 就由会话估算。
#[test]
fn http_chat_retries_without_stream_options_when_provider_rejects_it() {
    let reject =
        serde_json::json!({"error": {"message": "unknown field: stream_options"}}).to_string();
    let body = format!("{}data: [DONE]\n\n", sse_delta("好"));
    let mock = Mock::start(vec![
        (400, "application/json", reject),
        (200, "text/event-stream", body),
    ]);
    let mut chat = gateway().member_channel(&mock.channel("k"), "a");
    let done = chat.complete(&[Msg::user("问")], CompleteOpts::plain(true), &mut |_| {
        true
    });
    assert_eq!(done.raw, "好", "回退后正常收正文");
    assert!(done.usage.is_none(), "回退后的响应没 usage，不能伪造精确");
    let bodies = mock.bodies();
    assert_eq!(bodies.len(), 2, "400 之后只重试一次：{:?}", bodies);
    assert!(bodies[0].contains("stream_options"), "{}", bodies[0]);
    assert!(!bodies[1].contains("stream_options"), "{}", bodies[1]);
}

#[test]
fn http_stream_idle_keepalives_do_not_trip_a_total_timeout() {
    // 总时长 1.5s > timeout_secs=1，但每 300ms 都有字节（含 SSE keepalive）：
    // 正常流式不因总时长中断，只有真正空闲才中断。
    let steps = vec![
        (Duration::from_millis(300), b": ping\n\n".to_vec()),
        (Duration::from_millis(300), b": ping\n\n".to_vec()),
        (Duration::from_millis(300), sse_delta("你").into_bytes()),
        (Duration::from_millis(300), sse_delta("好").into_bytes()),
        (Duration::from_millis(300), b"data: [DONE]\n\n".to_vec()),
    ];
    let mock = StreamMock::start(steps);
    let mut chat = gateway().member_channel(&mock.channel_v1("k"), "a");
    let out = chat.complete(
        &[Msg::user("慢慢说")],
        CompleteOpts::plain(true).with_timeout(1),
        &mut |_| true,
    );
    assert_eq!(out.error, None, "正常流式不该超时：{:?}", out.error);
    assert_eq!(out.raw, "你好");
}

#[test]
fn http_stream_reports_idle_timeout_only_after_the_stream_goes_quiet() {
    let steps = vec![
        (Duration::from_millis(100), sse_delta("你").into_bytes()),
        (Duration::from_millis(2000), b"data: [DONE]\n\n".to_vec()),
    ];
    let mock = StreamMock::start(steps);
    let mut chat = gateway().member_channel(&mock.channel_v1("k"), "a");
    let out = chat.complete(
        &[Msg::user("停一下")],
        CompleteOpts::plain(true).with_timeout(1),
        &mut |_| true,
    );
    let err = out.error.unwrap_or_default();
    assert!(err.contains("流空闲超时"), "空闲超时要如实报：{}", err);
}

#[test]
fn http_stream_times_out_waiting_for_the_first_data_block() {
    let steps = vec![(Duration::from_millis(1500), sse_delta("迟到").into_bytes())];
    let mock = StreamMock::start(steps);
    let mut chat = gateway().member_channel(&mock.channel_v1("k"), "a");
    let out = chat.complete(
        &[Msg::user("在吗")],
        CompleteOpts::plain(true).with_timeout(1),
        &mut |_| true,
    );
    let err = out.error.unwrap_or_default();
    assert!(
        err.contains("等待首个数据块超时"),
        "首块超时要如实报：{}",
        err
    );
}

#[test]
fn http_stream_stop_cancels_without_waiting_for_the_next_chunk() {
    let mut steps = Vec::new();
    for _ in 0..20 {
        steps.push((Duration::from_millis(200), b": ping\n\n".to_vec()));
    }
    steps.push((
        Duration::from_millis(200),
        sse_delta("不应到达").into_bytes(),
    ));
    let mock = StreamMock::start(steps);
    let chat = gateway().member_channel(&mock.channel_v1("k"), "a");
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_for_call = Arc::clone(&cancel);
    let worker = std::thread::spawn(move || {
        let mut chat = chat;
        chat.complete(
            &[Msg::user("想很久")],
            CompleteOpts::plain(true)
                .with_timeout(30)
                .with_cancel(cancel_for_call),
            &mut |_| true,
        )
    });
    std::thread::sleep(Duration::from_millis(300));
    cancel.store(true, Ordering::Relaxed);
    let started = std::time::Instant::now();
    let out = worker.join().expect("生成线程");
    assert!(out.cancelled, "取消要显式标记：{:?}", out.error);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "停止不能等下一个 chunk（{:?}）",
        started.elapsed()
    );
}

#[test]
fn finish_reason_is_carried_back_from_both_paths() {
    // 非流式：结束原因在 choices[0].finish_reason
    let body = serde_json::json!({
        "choices": [{"message": {"content": "写完"}, "finish_reason": "stop"}]
    })
    .to_string();
    let mock = Mock::start(vec![(200, "application/json", body)]);
    let mut chat = gateway().member_channel(&mock.channel("k"), "a");
    let out = chat.complete(&[Msg::user("hi")], CompleteOpts::plain(false), &mut |_| {
        true
    });
    assert_eq!(out.raw, "写完");
    assert_eq!(out.finish, "stop", "结束原因要如实带回");
    assert_eq!(out.reasoning, "", "没有思维链字段时保持为空");
    assert!(!out.truncated(), "stop 不是截断");

    // 流式：收尾分片带 finish_reason = length（核心据此判定"被截断"，而不是"模型写错"）
    let sse = format!(
        "{}{}data: [DONE]\n\n",
        sse_delta("半句"),
        sse_finish("length")
    );
    let mock2 = Mock::start(vec![(200, "text/event-stream", sse)]);
    let mut chat2 = gateway().member_channel(&mock2.channel("k"), "a");
    let out2 = chat2.complete(&[Msg::user("hi")], CompleteOpts::plain(true), &mut |_| true);
    assert_eq!(out2.raw, "半句", "已产出的正文照常返回");
    assert_eq!(out2.finish, "length");
    assert!(out2.truncated(), "各家的截断取值都归到一处判定");
}

/// 一次原生工具调用的非流式响应（content 为 null：纯工具调用轮的常见形状）。
fn tool_call_body(name: &str, args: &str) -> String {
    serde_json::json!({
        "choices": [{
            "message": {
                "content": null,
                "tool_calls": [{ "id": "call_1", "type": "function", "function": { "name": name, "arguments": args } }]
            },
            "finish_reason": "tool_calls"
        }]
    })
    .to_string()
}

/// 流式里的一段工具调用分片（按 index 归位；id/name 一般只在第一片，arguments 逐片拼）。
fn sse_tool(index: u64, id: Option<&str>, name: Option<&str>, args: Option<&str>) -> String {
    let mut f = serde_json::json!({ "index": index });
    if let Some(id) = id {
        f["id"] = serde_json::json!(id);
    }
    let mut func = serde_json::Map::new();
    if let Some(n) = name {
        func.insert("name".to_string(), serde_json::json!(n));
    }
    if let Some(a) = args {
        func.insert("arguments".to_string(), serde_json::json!(a));
    }
    f["function"] = serde_json::Value::Object(func);
    format!(
        "data: {}\n\n",
        serde_json::json!({ "choices": [{ "delta": { "tool_calls": [f] } }] })
    )
}

#[test]
fn http_chat_reads_non_stream_reasoning() {
    let body = serde_json::json!({
        "choices": [{
            "message": { "content": "写完", "reasoning_content": "先思考" },
            "finish_reason": "stop"
        }]
    })
    .to_string();
    let mock = Mock::start(vec![(200, "application/json", body)]);
    let mut chat = gateway().member_channel(&mock.channel("k"), "a");
    let out = chat.complete(&[Msg::user("hi")], CompleteOpts::plain(false), &mut |_| {
        true
    });
    assert_eq!(out.reasoning, "先思考");
}

#[test]
fn http_chat_reads_native_tool_calls() {
    // content 是 null（不是缺字段）：按空串处理，并把 tool_calls 原样带回来
    let mock = Mock::start(vec![(
        200,
        "application/json",
        tool_call_body("read", "{\"path\":\"/x\"}"),
    )]);
    let mut chat = gateway().member_channel(&mock.channel("k"), "a");
    let out = chat.complete(
        &[Msg::user("读一下")],
        CompleteOpts::plain(false),
        &mut |_| true,
    );
    assert_eq!(out.raw, "", "纯工具调用轮没有正文");
    assert_eq!(out.finish, "tool_calls");
    assert_eq!(
        out.calls,
        vec![crate::capabilities::llm::api::ToolCall {
            id: "call_1".to_string(),
            name: "read".to_string(),
            args_json: "{\"path\":\"/x\"}".to_string(),
        }]
    );
}

#[test]
fn http_chat_assembles_streaming_tool_calls_by_index() {
    let sse = format!(
        "{}{}{}{}{}data: [DONE]\n\n",
        sse_tool(0, Some("call_a"), Some("read"), Some("{\"pa")),
        sse_tool(0, None, None, Some("th\":\"/x\"}")),
        sse_tool(1, Some("call_b"), Some("search"), Some("{}")),
        sse_finish("tool_calls"),
        "",
    );
    let mock = Mock::start(vec![(200, "text/event-stream", sse)]);
    let mut chat = gateway().member_channel(&mock.channel("k"), "a");
    let out = chat.complete(
        &[Msg::user("两件事")],
        CompleteOpts::plain(true),
        &mut |_| true,
    );
    assert_eq!(out.finish, "tool_calls");
    assert_eq!(
        out.raw, "",
        "纯工具调用轮没有正文，也不能因此判成「没内容」去换候选"
    );
    assert_eq!(
        out.calls.len(),
        2,
        "两次调用按 index 各就各位：{:?}",
        out.calls
    );
    assert_eq!(out.calls[0].id, "call_a");
    assert_eq!(out.calls[0].name, "read");
    assert_eq!(
        out.calls[0].args_json, "{\"path\":\"/x\"}",
        "分片的 arguments 要按序拼起来"
    );
    assert_eq!(out.calls[1].name, "search");
    assert_eq!(out.calls[1].args_json, "{}");
}

/// 历史里带原生调用时，请求体必须发成协议形状：assistant(正文 + tool_calls) + 每条结果一条 role=tool。
/// 这就是回放（重启/回档重建）要走的那套形状，所以它必须真的发得出去，不能只在内存里对。
#[test]
fn native_history_goes_out_in_protocol_shape() {
    let mock = Mock::start(vec![(200, "application/json", completion_body("ok"))]);
    let mut chat = gateway().member_channel(&mock.channel("k"), "a");
    let history = [
        Msg::user("读一下"),
        Msg::assistant_calls(
            "我看看这个文件。",
            vec![crate::capabilities::llm::api::ToolCall {
                id: "call_1".to_string(),
                name: "read".to_string(),
                args_json: "{\"path\":\"/x\"}".to_string(),
            }],
        ),
        Msg::tool("call_1", "[工具结果] read\n第一行"),
    ];
    let _ = chat.complete(&history, CompleteOpts::plain(false), &mut |_| true);
    let body = &mock.bodies()[0];
    assert!(
        body.contains("\"tool_calls\""),
        "助手消息要带 tool_calls：{}",
        body
    );
    assert!(
        body.contains("\"id\":\"call_1\"") && body.contains("\"name\":\"read\""),
        "调用的 id 与名字原样带上：{}",
        body
    );
    assert!(
        body.contains("\"arguments\""),
        "参数走结构化槽位（arguments）而不是正文：{}",
        body
    );
    assert!(
        body.contains("\"content\":\"我看看这个文件。\""),
        "助手消息的正文要真的发出去：{}",
        body
    );
    assert!(
        body.contains("\"role\":\"tool\"") && body.contains("\"tool_call_id\":\"call_1\""),
        "结果消息用 role=tool + tool_call_id 回应它：{}",
        body
    );
}

#[test]
fn tool_probe_tells_supported_unsupported_and_unknown_apart() {
    let log: Arc<dyn Log + Send + Sync> = Arc::new(NoopLog);
    let gateway = || HttpGateway::with_log(Arc::clone(&log), memo_new());

    // ① 支持：不带 tools 能通，带 tools 返回工具调用
    let mock = Mock::start(vec![
        (200, "application/json", completion_body("好")),
        (
            200,
            "application/json",
            tool_call_body("solomni_ping", "{}"),
        ),
    ]);
    match gateway().probe_tools(&mock.channel("k")) {
        Ok(ProbeOutcome::Supported { detail }) => {
            assert!(detail.contains("solomni_ping"), "{}", detail)
        }
        other => panic!("应判为支持：{:?}", other),
    }

    // ② 明确不支持：同样的请求，带上 tools 被供应商拒（附供应商原话）
    let mock = Mock::start(vec![
        (200, "application/json", completion_body("好")),
        (
            400,
            "application/json",
            serde_json::json!({ "error": { "message": "Unknown parameter: tools" } }).to_string(),
        ),
    ]);
    match gateway().probe_tools(&mock.channel("k")) {
        Ok(ProbeOutcome::Unsupported { detail }) => assert!(detail.contains("tools"), "{}", detail),
        other => panic!("应判为明确不支持：{:?}", other),
    }

    // ③ 无法判定：带 tools 也通，但这次没发起调用——如实说"没法定论"，不替用户拍板
    let mock = Mock::start(vec![
        (200, "application/json", completion_body("好")),
        (200, "application/json", completion_body("我不调用工具")),
    ]);
    match gateway().probe_tools(&mock.channel("k")) {
        Ok(ProbeOutcome::Unknown { detail }) => {
            assert!(detail.contains("没有发起调用"), "{}", detail)
        }
        other => panic!("应判为无法判定：{:?}", other),
    }

    // ④ 通道本身就不通：如实报"测不了"，绝不把 401 之类的失败当成"不支持工具调用"
    let mock = Mock::start(vec![(
        401,
        "application/json",
        "{\"error\":{\"message\":\"bad key\"}}".to_string(),
    )]);
    let err = gateway()
        .probe_tools(&mock.channel("bad"))
        .expect_err("通道不通就是 Err");
    assert!(err.contains("通道本身就没打通"), "{}", err);
}

/// 回放形状探测：逐项如实回报哪种写法被接受（不合并、不替用户拍板），
/// 且发出去的确实是那个形状；基线不通时如实报"测不了"，绝不把 401 说成"这个形状被拒"。
#[test]
fn replay_shape_probe_reports_which_writings_the_supplier_accepts() {
    let log: Arc<dyn Log + Send + Sync> = Arc::new(NoopLog);
    let rejected = serde_json::json!({
        "error": { "message": "content is required when tool_calls is present" }
    })
    .to_string();
    // 编号只放在工具结果里：回答里带回它才算"真的读到了"（收了 != 看懂了）。
    // 而且模型常常把前缀省掉（真机实测就只回后半段）——那不算没读懂，判据要容得下。
    let nonce = "solomni-7f3a91c2";
    let mock = Mock::start(vec![
        (200, "application/json", completion_body("7f3a91c2")),
        (
            200,
            "application/json",
            completion_body("那个编号是 7f3a91c2"),
        ),
        (400, "application/json", rejected),
        (200, "application/json", completion_body("我看不到任何编号")),
        (200, "application/json", completion_body("solomni-7f3a91c2")),
    ]);
    let report = crate::capabilities::llm::detail::http_probe::probe_replay_with(
        &mock.channel("k"),
        &log,
        nonce,
    )
    .expect("基线通过就该拿到报告");
    let got: Vec<(&str, bool, bool)> = report
        .shapes
        .iter()
        .map(|s| (s.name.as_str(), s.accepted, s.understood))
        .collect();
    assert_eq!(
        got,
        vec![
            ("baseline-text", true, true),
            ("content-empty", true, true),
            ("content-null", false, false),
            ("content-prose", true, false),
            ("tool-with-name", true, true),
        ],
        "每个形状的接受 / 被拒 / 有没有真被读懂都要逐项报出来（省掉前缀也算读懂）"
    );
    let no = report
        .shapes
        .iter()
        .find(|s| !s.accepted)
        .expect("有一条被拒");
    assert!(
        no.detail.contains("content is required"),
        "被拒要带供应商原话：{}",
        no.detail
    );
    // 发出去的确实是那个形状：助手回合带 tool_calls，结果消息用同一个 tool_call_id 对应。
    let bodies = mock.bodies();
    assert_eq!(bodies.len(), 5, "每个形状各发一次");
    assert!(
        !bodies[0].contains("tool_calls"),
        "基线是文本回放，不带协议字段：{}",
        bodies[0]
    );
    assert!(
        bodies[0].contains("\"tools\""),
        "每个形状都要带工具声明：{}",
        bodies[0]
    );
    assert!(
        bodies[1].contains("\"tool_calls\"")
            && bodies[1].contains("\"tool_call_id\":\"call_solomni_probe\""),
        "协议形状要真的发出去：{}",
        bodies[1]
    );
    assert!(
        bodies[1].contains(nonce),
        "本次编号要真的在工具结果里：{}",
        bodies[1]
    );

    let bad = Mock::start(vec![(
        401,
        "application/json",
        "{\"error\":{\"message\":\"bad key\"}}".to_string(),
    )]);
    let err = crate::capabilities::llm::detail::http_probe::probe_replay(&bad.channel("bad"), &log)
        .expect_err("通道不通就是 Err");
    assert!(err.contains("通道本身就没打通"), "{}", err);
}

#[test]
fn http_errors_come_back_honestly_and_never_carry_the_api_key() {
    let secret = "sk-super-secret-xyz";
    let body =
        serde_json::json!({"error": {"message": format!("invalid key {}", secret)}}).to_string();
    let mock = Mock::start(vec![
        (401, "application/json", body),
        (401, "application/json", "{}".to_string()),
    ]);
    let mut chat = gateway().member_channel(&mock.channel(secret), "a");
    let done = chat.complete(&[Msg::user("hi")], CompleteOpts::plain(false), &mut |_| {
        true
    });
    // 失败**不是**模型的回复：正文为空，原因在 error 里（上层据此如实告知并中断本轮）。
    assert!(done.raw.is_empty(), "失败不该有正文：{}", done.raw);
    let err = done.error.expect("失败必须如实带原因");
    assert!(err.contains("模型调用失败"), "失败必须如实回执：{}", err);
    assert!(!err.contains(secret), "出站错误里的密钥必须先脱敏：{}", err);
    assert!(
        err.contains("***") || err.contains("401"),
        "要留下可定位的事实：{}",
        err
    );
}

#[test]
fn http_model_catalog_lists_models_and_rejects_broken_shapes() {
    let ok = Mock::start(vec![(
        200,
        "application/json",
        serde_json::json!({"data": [{"id": "m-a"}, {"id": "m-b"}]}).to_string(),
    )]);
    let c = ok.channel("k");
    let got = HttpModelCatalog::with_log(Arc::new(NoopLog), memo_new())
        .list_models(&c.base_url, &c.api_key)
        .expect("发现成功");
    assert_eq!(got, vec!["m-a", "m-b"]);
    assert!(ok.requests()[0].contains("/models"), "{:?}", ok.requests());

    // 2xx 但不是模型清单形状：不猜测兜底，如实报错（并试过回落候选）。
    let bad = Mock::start(vec![
        (200, "application/json", "{\"foo\": 1}".to_string()),
        (200, "application/json", "{\"foo\": 1}".to_string()),
    ]);
    let c2 = bad.channel("k");
    let err = HttpModelCatalog::with_log(Arc::new(NoopLog), memo_new())
        .list_models(&c2.base_url, &c2.api_key)
        .unwrap_err();
    assert!(!err.is_empty(), "形状不符必须报错：{}", err);

    // 4xx：如实报错，不静默给空清单。
    let boom = Mock::start(vec![
        (400, "application/json", "no".to_string()),
        (400, "application/json", "no".to_string()),
    ]);
    let c3 = boom.channel("k");
    let err2 = HttpModelCatalog::with_log(Arc::new(NoopLog), memo_new())
        .list_models(&c3.base_url, &c3.api_key)
        .unwrap_err();
    assert!(err2.contains("400"), "{}", err2);
}
/// 出站错误分类的判据：**网络类**算环境（env-skip），**其余 TLS 错误**算我们的问题（必须失败），
/// 只有「本进程取不到系统 TLS 凭证」那一码才算环境结论（env-tls）。
/// 现场：DSH 沙箱挡住工作区外的用户凭证存储时，schannel 报 SEC_E_NO_CREDENTIALS——
/// 连 Windows 自带的 curl.exe 都握不了手，放开沙箱后同一条链路立刻通。
/// 这条测试钉的就是「绝不把 TLS 坏了说成环境不允许」这条红线，同时不把环境结论误报成失败。
#[test]
fn outbound_error_classification_keeps_environment_and_our_bug_apart() {
    use crate::capabilities::llm::detail::http_agent::classify;
    // 网络类：环境结论。
    assert_eq!(classify(&ureq::Error::HostNotFound), "no-net");
    assert_eq!(classify(&ureq::Error::ConnectionFailed), "no-net");
    // 取不到系统凭证（十进制与十六进制两种原文都要认，且大小写无关）。
    let dec = ureq::Error::Tls("安全包中没有可用的凭证 (os error -2146893042)");
    assert_eq!(
        classify(&dec),
        "env-tls",
        "判据只认稳定错误码，不认本地化文案"
    );
    let hex = ureq::Error::Tls(
        "schannel: AcquireCredentialsHandle failed: SEC_E_NO_CREDENTIALS (0x8009030e)",
    );
    assert_eq!(classify(&hex), "env-tls", "十六进制写法也要认");
    // 其余 TLS 错误：我们链路的问题，必须失败——不许借「环境」二字溜过去。
    let other = ureq::Error::Tls("certificate verify failed");
    assert_eq!(classify(&other), "tls-fail");
    let bad_host = ureq::Error::Tls("hostname mismatch");
    assert_eq!(classify(&bad_host), "tls-fail");

    // 判据本身：只认那一码的两种写法，别的原文一律不算（"环境"不能是个筐）。Windows 上 native-tls 的错误走
    // NativeTls 变体（不是通用 Tls 壳），构造不出来，所以由真机探针（--https-check → env-tls）验接线。
    use crate::capabilities::llm::detail::http_agent::lacks_system_credentials;
    assert!(
        lacks_system_credentials("(os error -2146893042)"),
        "有符号十进制要认"
    );
    assert!(lacks_system_credentials("(0x8009030e)"), "十六进制要认");
    assert!(lacks_system_credentials("(0x8009030E)"), "大小写无关");
    assert!(!lacks_system_credentials("certificate verify failed"));
    assert!(!lacks_system_credentials("hostname mismatch"));
    assert!(!lacks_system_credentials(""), "空原文不能算环境结论");
}

/// 目的：思考片段在**唯一的请求体构造处**合并：字符串/对象都进 wire，空对象不产生字段；
/// 保留键即使绕过登记处校验也挡在运行时之外。
#[test]
fn http_chat_merges_the_thinking_fragment_into_the_request_body() {
    // 对象片段（GLM 的关闭写法）
    let mock = Mock::start(vec![(200, "application/json", completion_body("好"))]);
    let mut chat = gateway().member_channel(&mock.channel("k"), "a");
    let thinking = Arc::new(serde_json::json!({ "thinking": { "type": "disabled" } }));
    let out = chat.complete(
        &[Msg::user("hi")],
        CompleteOpts::plain(false).with_thinking(Some(thinking)),
        &mut |_| true,
    );
    assert!(out.error.is_none());
    let body = &mock.bodies()[0];
    assert!(
        body.contains("\"thinking\":{\"type\":\"disabled\"}"),
        "对象片段要原样合并：{}",
        body
    );
    assert!(
        body.contains("\"model\":\"m\""),
        "模型字段仍由通道决定：{}",
        body
    );

    // 保留键：运行时不得被片段覆盖
    let mock2 = Mock::start(vec![(200, "application/json", completion_body("好"))]);
    let mut chat2 = gateway().member_channel(&mock2.channel("k"), "a");
    let hijack = Arc::new(serde_json::json!({ "model": "hijack", "reasoning_effort": "high" }));
    let _ = chat2.complete(
        &[Msg::user("hi")],
        CompleteOpts::plain(false).with_thinking(Some(hijack)),
        &mut |_| true,
    );
    let body2 = &mock2.bodies()[0];
    assert!(
        body2.contains("\"model\":\"m\""),
        "model 不得被片段覆盖：{}",
        body2
    );
    assert!(
        !body2.contains("hijack"),
        "保留键的值不得进请求体：{}",
        body2
    );
    assert!(body2.contains("\"reasoning_effort\":\"high\""));
}

