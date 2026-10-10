//! 目的：MCP stdio 适配器的契约用例——握手、翻页发现、调用、错误与通知处理。
//! 管：initialize → notifications/initialized → tools/list（翻页）→ tools/call；应答按 id 配对；isError 与 error 如实报错。
//! 不管：进程与围栏机制（在 kernel 的 SessionHost 实现里）；真实服务的端到端另有用例。
//! 联动：适配器见 src/capabilities/residents/detail/mcp.rs；替身见 src/tests/doubles.rs。

use crate::capabilities::residents::detail::McpAdapter;
use crate::capabilities::residents::ports::{LaunchSpec, ServiceAdapter};
use crate::kernel::api::FenceSpec;
use crate::kernel::ports::SessionHost;
use crate::tests::doubles::ScriptedHost;
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
        r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"echo","description":"回显"}],"nextCursor":"c1"}}"#,
        r#"{"jsonrpc":"2.0","id":3,"result":{"tools":[{"name":"add","description":"相加"}]}}"#,
        r#"{"jsonrpc":"2.0","id":4,"result":{"content":[{"type":"text","text":"echo hi"}],"isError":false}}"#,
        r#"{"jsonrpc":"2.0","id":5,"result":{"content":[{"type":"text","text":"坏了"}],"isError":true}}"#,
    ]));
    let adapter = McpAdapter::new(host.clone() as Arc<dyn SessionHost + Send + Sync>);
    let (mut instance, ops) = adapter.start(&launch()).expect("握手并发现工具");
    let names: Vec<&str> = ops.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, vec!["echo", "add"], "游标翻页要收全工具");
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
/// 约束：需要已构建的产品可执行文件与 python；缺一即如实跳过（Windows 的容器预授权尚未接入长驻会话）。
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
    let adapter =
        McpAdapter::new(Arc::new(ProcessSessions::new(exe)) as Arc<dyn SessionHost + Send + Sync>);
    let mut spec = launch();
    spec.command = format!("{} tests/fixtures/mcp_echo.py", py);
    spec.fence = FenceSpec::standalone(&root, None, false);

    let (mut instance, ops) = adapter.start(&spec).expect("真实服务要握手成功");
    assert!(
        ops.iter().any(|o| o.name == "echo"),
        "要发现 echo：{:?}",
        ops.iter().map(|o| o.name.clone()).collect::<Vec<_>>()
    );
    let out = instance.call("echo", &json!({ "ping": 1 })).expect("调用");
    assert!(out.contains("ping"), "回显应含参数：{}", out);
    instance.stop();
}

/// 服务回 JSON-RPC error：如实报错并带上服务给的消息。
#[test]
fn mcp_error_response_is_reported() {
    let host = Arc::new(ScriptedHost::new(vec![
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"没有这个方法"}}"#,
    ]));
    let adapter = McpAdapter::new(host.clone() as Arc<dyn SessionHost + Send + Sync>);
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
    use crate::capabilities::residents::api::ResidentOps;
    use crate::capabilities::residents::service::ResidentsService;
    use crate::capabilities::workspace::api::ServiceDecl;
    use crate::kernel::detail::ProcessSessions;
    use crate::tests::doubles::{
        built_exe, module_of, python, test_workspace, InMemoryPackages, InMemoryWorkspace,
        VecSource,
    };
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
    let host: Arc<dyn SessionHost + Send + Sync> = Arc::new(ProcessSessions::new(exe));
    let residents: Arc<dyn ResidentOps + Send + Sync> = Arc::new(ResidentsService::new(
        vec![Arc::new(McpAdapter::new(host)) as Arc<dyn ServiceAdapter + Send + Sync>],
        test_workspace(
            Arc::new(VecSource(vec![m])),
            Arc::new(InMemoryPackages::empty()),
            Arc::new(InMemoryWorkspace::new()),
        ),
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    ));

    let ops = residents.start("m1", "echo", "s1").expect("启动 MCP 服务");
    assert!(
        ops.iter().any(|o| o.name == "echo"),
        "要发现服务提供的 echo 操作"
    );
    let receipt = residents
        .call("m1", "echo", "echo", &json!({ "hi": 1 }))
        .expect("调用服务操作");
    assert!(
        receipt.ok && receipt.output.contains("hi"),
        "{}",
        receipt.output
    );
    residents.stop("m1", "echo").expect("停止");
}
