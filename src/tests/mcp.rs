//! 目的：MCP stdio 适配器的契约用例——握手、翻页发现、调用、错误与通知处理。
//! 管：initialize → notifications/initialized → tools/list（翻页）→ tools/call；应答按 id 配对；isError 与 error 如实报错。
//! 不管：进程与围栏机制（在 kernel 的 SessionHost 实现里）；真实服务的端到端另有用例。
//! 联动：适配器见 src/capabilities/residents/detail/mcp.rs；替身见 src/tests/doubles.rs。

use crate::capabilities::residents::detail::McpAdapter;
use crate::capabilities::residents::ports::{LaunchSpec, ServiceAdapter};
use crate::kernel::api::FenceSpec;
use crate::kernel::ports::SessionHost;
use crate::tests::doubles::{RecordingLog, ScriptedHost};
use serde_json::json;
use std::sync::Arc;

/// 一次拉起服务的声明事实（命令与围栏在真实运行时由 service 派生）。
fn launch() -> LaunchSpec {
    LaunchSpec {
        module: "m1".to_string(),
        name: "svc".to_string(),
        command: "python server.py".to_string(),
        cwd: std::path::PathBuf::from("."),
        options: Default::default(),
        env: vec![("TOKEN".to_string(), "s3cr3t".to_string())],
        fence: FenceSpec::standalone(std::path::Path::new("."), None, false),
    }
}

/// 握手 → 初始化通知 → 翻页发现工具 → 调用（含通知跳过与 isError）。
#[test]
fn mcp_handshake_lists_tools_calls_and_reports_errors() {
    let host = Arc::new(ScriptedHost::new(vec![
        r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","serverInfo":{"name":"demo"}}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/message","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"echo","description":"回显","inputSchema":{"type":"object","properties":{"x":{"type":"integer"}},"required":["x"]}}],"nextCursor":"c1"}}"#,
        r#"{"jsonrpc":"2.0","id":3,"result":{"tools":[{"name":"add","description":"相加","inputSchema":{"type":"object"}}]}}"#,
        r#"{"jsonrpc":"2.0","id":4,"result":{"content":[{"type":"text","text":"echo hi"}],"isError":false}}"#,
        r#"{"jsonrpc":"2.0","id":5,"result":{"content":[{"type":"text","text":"坏了"}],"isError":true}}"#,
    ]));
    let adapter = McpAdapter::new(
        host.clone() as Arc<dyn SessionHost + Send + Sync>,
        Arc::new(crate::kernel::ports::NoopLog),
    );
    let (mut instance, ops) = adapter.start(&launch()).expect("握手并发现工具");
    let names: Vec<&str> = ops.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, vec!["echo", "add"], "游标翻页要收全工具");
    assert_eq!(
        ops[0]
            .parameters
            .as_ref()
            .and_then(|s| s.get("required"))
            .and_then(|v| v.as_array())
            .map(|a| a.len()),
        Some(1),
        "inputSchema 要原样收进参数契约：{:?}",
        ops[0].parameters
    );
    let sent = host.sent.lock().expect("锁").clone();
    assert!(sent[0].contains("\"initialize\""), "先握手：{}", sent[0]);
    assert!(
        sent.iter().any(|s| s.contains("notifications/initialized")),
        "要发 initialized 通知"
    );
    assert!(
        sent.iter().any(|s| s.contains("\"cursor\":\"c1\"")),
        "翻页要带上游标"
    );

    let out = instance.call("echo", &json!({ "x": 1 })).expect("调用");
    assert_eq!(out, "echo hi");
    let call = host
        .sent
        .lock()
        .expect("锁")
        .iter()
        .find(|s| s.contains("tools/call"))
        .cloned()
        .expect("要有 tools/call");
    assert!(call.contains("\"name\":\"echo\""), "{}", call);

    let err = instance.call("add", &json!({})).unwrap_err();
    assert!(err.contains("坏了"), "isError 要如实报错：{}", err);

    instance.stop();
    assert!(
        host.sent.lock().expect("锁").iter().any(|s| s == "<kill>"),
        "停止要关会话"
    );
}

/// 真实端到端：`ProcessSessions`（守门进程 + 长驻 stdio）拉起一个真的 MCP 服务，握手、发现、调用。
/// 约束：需要已构建的产品可执行文件与 python；缺一即如实跳过。Windows 的容器预授权已接入长驻会话，
///   但 Windows 的长驻围栏由 `tests/windows/` 的真机用例验证，本用例只在 unix 上跑。
#[cfg(unix)]
#[test]
fn mcp_adapter_talks_to_a_real_stdio_server() {
    use crate::kernel::detail::ProcessSessions;
    use crate::tests::doubles::{built_exe, python};

    let (Some(exe), Some(py)) = (built_exe(), python()) else {
        eprintln!("[mcp] 缺少已构建的可执行文件或 python，跳过真实 stdio 端到端");
        return;
    };
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let book = crate::kernel::detail::procs::ProcessBook::open(&crate::tests::scratch("mcp-book"));
    let adapter = McpAdapter::new(
        Arc::new(ProcessSessions::new(
            exe,
            crate::tests::doubles::test_process_texts(),
            std::path::PathBuf::new(),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            Arc::clone(&book),
        )) as Arc<dyn SessionHost + Send + Sync>,
        Arc::new(crate::kernel::ports::NoopLog),
    );
    let mut spec = launch();
    spec.command = format!("{} tests/fixtures/mcp_echo.py", py);
    spec.fence = FenceSpec::standalone(&root, None, false);

    let (mut instance, ops) = adapter.start(&spec).expect("真实服务要握手成功");
    assert!(
        ops.iter().any(|o| o.name == "echo"),
        "要发现 echo：{:?}",
        ops.iter().map(|o| o.name.clone()).collect::<Vec<_>>()
    );
    assert!(
        ops.iter().all(|o| o.parameters.is_some()),
        "真实服务的 inputSchema 要原样收下：{:?}",
        ops.iter().map(|o| o.parameters.clone()).collect::<Vec<_>>()
    );
    let out = instance.call("echo", &json!({ "ping": 1 })).expect("调用");
    assert!(out.contains("ping"), "回显应含参数：{}", out);
    let listed = book.list();
    assert_eq!(listed.len(), 1, "守门进程起时进账：{:?}", listed);
    assert_eq!(listed[0].purpose, "resident", "用途要标成常驻服务");
    assert_eq!(
        listed[0].guard_note, None,
        "unix 由内核信号兜，不该报没挂上"
    );
    instance.stop();
    assert!(book.list().is_empty(), "收掉时要注销账上那条");
}

/// 模型侧工具面：已就绪服务的操作进原生声明与可用清单，调用路由到常驻服务能力。
#[test]
fn model_tool_face_exposes_started_service_ops() {
    use crate::capabilities::collab::service::tool_loop::{
        available_tools, dispatch_external, tool_decls,
    };
    use crate::capabilities::llm::api::ToolInvoke;
    use crate::capabilities::modules::api::ServiceDecl;
    use crate::capabilities::residents::api::ResidentOps;
    use crate::capabilities::residents::service::ResidentsService;
    use crate::capabilities::session::api::ServiceOp;
    use crate::tests::doubles::{
        member_with_service_tools, module_of, test_modules, FakeServiceAdapter, VecSource,
    };
    use std::collections::BTreeMap;

    let fake = Arc::new(FakeServiceAdapter::new());
    let mut m = module_of("m0");
    m.manifest.services.insert(
        "svc".to_string(),
        ServiceDecl {
            adapter: "fake".to_string(),
            command: "run".to_string(),
            desc: String::new(),
            enabled: true,
            options: BTreeMap::new(),
        },
    );
    let residents: Arc<dyn ResidentOps + Send + Sync> = Arc::new(ResidentsService::new(
        vec![Arc::clone(&fake) as Arc<dyn ServiceAdapter + Send + Sync>],
        test_modules(Arc::new(VecSource(vec![m]))),
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    ));
    let ops = residents.start("m0", "svc", "").expect("启动服务");
    let op_name = ops[0].name.clone();
    let ctx = member_with_service_tools(
        "m0",
        Arc::clone(&residents),
        ServiceOp {
            tool: format!("svc.{}", op_name),
            service: "svc".to_string(),
            op: op_name.clone(),
            description: "回显".to_string(),
            parameters: ops[0].parameters.clone(),
        },
    );

    let decls = tool_decls(&ctx);
    let echo_decl = decls
        .list
        .iter()
        .find(|d| d.name == "m0_svc_echo")
        .expect("原生声明要含服务操作");
    assert_eq!(
        echo_decl.parameters["properties"]["x"]["type"].as_str(),
        Some("integer"),
        "服务操作的参数契约要原样进原生声明：{}",
        echo_decl.parameters
    );
    let param_lines = crate::capabilities::tools::api::json_schema_lines(&echo_decl.parameters);
    assert!(
        param_lines
            .iter()
            .any(|l| l.contains("x") && l.contains("必填")),
        "信封说明要列出参数名与必填：{:?}",
        param_lines
    );
    assert_eq!(
        decls.wire.get("m0_svc_echo"),
        Some(&(Some("m0".to_string()), "svc.echo".to_string())),
        "线上名要能译回（模块, 服务.操作）"
    );
    let listed = available_tools(&ctx);
    assert!(
        listed.contains("m0.svc.echo"),
        "可用清单要含服务操作：{}",
        listed
    );

    let inv = ToolInvoke {
        malformed: None,
        module: Some("m0".to_string()),
        name: "svc.echo".to_string(),
        args_json: "{\"x\":1}".to_string(),
        body: String::new(),
        lead: String::new(),
    };
    let (module, outcome) = dispatch_external(&ctx, &inv, None);
    assert_eq!(module, "m0");
    assert!(
        outcome.ok && outcome.output.contains("echo"),
        "{}",
        outcome.output
    );
    assert!(
        fake.calls
            .lock()
            .expect("锁")
            .iter()
            .any(|c| c == "call:echo"),
        "要路由到常驻服务能力"
    );
}

/// 服务报告 tools/list_changed：下一次调用前刷新操作清单，并交回新清单（没有变化 = None）。
#[test]
fn mcp_refreshes_operations_on_list_changed() {
    let host = Arc::new(ScriptedHost::new(vec![
        r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05"}}"#,
        r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"echo"}]}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/tools/list_changed"}"#,
        r#"{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"ok"}]}}"#,
        r#"{"jsonrpc":"2.0","id":4,"result":{"tools":[{"name":"echo"},{"name":"add"}]}}"#,
    ]));
    let adapter = McpAdapter::new(
        host.clone() as Arc<dyn SessionHost + Send + Sync>,
        Arc::new(crate::kernel::ports::NoopLog),
    );
    let (mut instance, ops) = adapter.start(&launch()).expect("握手");
    assert_eq!(ops.len(), 1);

    let out = instance.call("echo", &json!({})).expect("调用");
    assert_eq!(out, "ok");
    let refreshed = instance
        .take_refreshed_operations()
        .expect("list_changed 后要交回新清单");
    let names: Vec<&str> = refreshed.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, vec!["echo", "add"]);
    assert!(
        instance.take_refreshed_operations().is_none(),
        "没有新变化就不该再交一次"
    );
}

/// 服务回选的协议版本：采用它（记进运行日志），握手照常成功。
#[test]
fn mcp_adopts_the_server_protocol_version() {
    let host = Arc::new(ScriptedHost::new(vec![
        r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","serverInfo":{"name":"demo"}}}"#,
        r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[]}}"#,
    ]));
    let log = Arc::new(RecordingLog::new());
    let adapter = McpAdapter::new(
        host.clone() as Arc<dyn SessionHost + Send + Sync>,
        log.clone(),
    );
    let (mut instance, ops) = adapter.start(&launch()).expect("版本不同也要继续");
    assert!(ops.is_empty());
    assert!(log.has("2025-06-18"), "服务回选的版本要如实记进日志");
    instance.stop();
}

/// tools/call 的非文本 content 如实标注（不静默丢），structuredContent 也带上。
#[test]
fn mcp_reports_non_text_content_honestly() {
    let host = Arc::new(ScriptedHost::new(vec![
        r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05"}}"#,
        r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"pic","inputSchema":{"type":"object"}}]}}"#,
        r#"{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"image","mimeType":"image/png","data":"AAAA"},{"type":"text","text":"说明"}],"structuredContent":{"ok":true}}}"#,
    ]));
    let adapter = McpAdapter::new(
        host.clone() as Arc<dyn SessionHost + Send + Sync>,
        Arc::new(crate::kernel::ports::NoopLog),
    );
    let (mut instance, _) = adapter.start(&launch()).expect("握手");
    let out = instance.call("pic", &json!({})).expect("调用");
    assert!(out.contains("[image image/png 4 字节 base64]"), "{}", out);
    assert!(out.contains("说明"), "{}", out);
    assert!(out.contains("[structuredContent] {\"ok\":true}"), "{}", out);
    instance.stop();
}

/// 服务回 JSON-RPC error：如实报错并带上服务给的消息。
#[test]
fn mcp_error_response_is_reported() {
    let host = Arc::new(ScriptedHost::new(vec![
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"没有这个方法"}}"#,
    ]));
    let adapter = McpAdapter::new(
        host.clone() as Arc<dyn SessionHost + Send + Sync>,
        Arc::new(crate::kernel::ports::NoopLog),
    );
    let err = match adapter.start(&launch()) {
        Ok(_) => panic!("服务回 error 时不该当成成功"),
        Err(e) => e,
    };
    assert!(
        err.contains("initialize") && err.contains("没有这个方法"),
        "{}",
        err
    );
}

/// 全链路：清单声明 services（adapter = mcp）→ residents 拉起 → MCP 适配器 → 真实服务进程 → 调用。
#[cfg(unix)]
#[test]
fn residents_mcp_service_end_to_end() {
    use crate::capabilities::modules::api::ServiceDecl;
    use crate::capabilities::residents::api::ResidentOps;
    use crate::capabilities::residents::service::ResidentsService;
    use crate::kernel::detail::ProcessSessions;
    use crate::tests::doubles::{built_exe, module_of, python, test_modules, VecSource};
    use std::collections::BTreeMap;

    let (Some(exe), Some(py)) = (built_exe(), python()) else {
        eprintln!("[mcp] 缺少已构建的可执行文件或 python，跳过全链路端到端");
        return;
    };
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut m = module_of("m1");
    m.root = root.clone();
    m.manifest.services.insert(
        "echo".to_string(),
        ServiceDecl {
            adapter: "mcp".to_string(),
            command: format!("{} tests/fixtures/mcp_echo.py", py),
            desc: "测试用 MCP 服务".to_string(),
            enabled: true,
            options: BTreeMap::new(),
        },
    );
    let book =
        crate::kernel::detail::procs::ProcessBook::open(&crate::tests::scratch("mcp-chain-book"));
    let host: Arc<dyn SessionHost + Send + Sync> = Arc::new(ProcessSessions::new(
        exe,
        crate::tests::doubles::test_process_texts(),
        std::path::PathBuf::new(),
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
        Arc::clone(&book),
    ));
    let residents: Arc<dyn ResidentOps + Send + Sync> = Arc::new(ResidentsService::new(
        vec![Arc::new(McpAdapter::new(
            host,
            Arc::new(crate::kernel::ports::NoopLog),
        )) as Arc<dyn ServiceAdapter + Send + Sync>],
        test_modules(Arc::new(VecSource(vec![m]))),
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    ));

    let ops = residents.start("m1", "echo", "s1").expect("启动 MCP 服务");
    assert!(
        ops.iter().any(|o| o.name == "echo"),
        "要发现服务提供的 echo 操作"
    );
    assert!(
        ops.iter().all(|o| o.parameters.is_some()),
        "服务给的 inputSchema 要原样进操作契约"
    );
    let receipt = residents
        .call("m1", "echo", "echo", &json!({ "hi": 1 }))
        .expect("调用服务操作");
    assert!(
        receipt.ok && receipt.output.contains("hi"),
        "{}",
        receipt.output
    );
    let listed = book.list();
    assert_eq!(listed.len(), 1, "守门进程起时进账：{:?}", listed);
    assert_eq!(
        listed[0].owner, "resident/m1/echo",
        "归属要能指出哪个模块的哪个服务"
    );
    residents.stop("m1", "echo").expect("停止");
    assert!(book.list().is_empty(), "停止后要注销账上那条");
}
