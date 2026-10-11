//! 目的：MCP（Model Context Protocol）stdio 适配器——把 JSON-RPC 按行协议翻成常驻服务的操作与调用。
//! 管：initialize 握手（采用服务回选的协议版本，记进运行日志）、notifications/initialized、tools/list
//!   （含游标翻页与 `inputSchema` 原样收下）、tools/call（`content` 各项如实、非文本项标注、`isError`）；
//!   应答按 id 配对，通知与别的消息如实跳过。
//! 不管：进程与围栏机制（走注入的 `SessionHost`）；生命周期与租约（在 service）；参数校验（服务是权威）。
//! 联动：实现 ports::ServiceAdapter；经 `detail/mod.rs` 暴露给组合根。

use crate::capabilities::residents::api::Operation;
use crate::capabilities::residents::ports::{LaunchSpec, ServiceAdapter, ServiceInstance};
use crate::kernel::ports::{Session, SessionHost, SessionSpec};
use serde_json::{json, Value};
use std::sync::Arc;

/// 目的：本适配器在 module.yaml 的 `services.<名>.adapter` 里用的 id。
pub const ADAPTER_ID: &str = "mcp";

/// 目的：MCP 协议版本（握手时声明；服务不认也得如实把它的版本回给上层）。
const PROTOCOL_VERSION: &str = "2024-11-05";

/// 目的：MCP stdio 适配器——按声明拉起服务、握手、发现工具、转发调用。
pub struct McpAdapter {
    host: Arc<dyn SessionHost + Send + Sync>,
    /// 运行日志：协议版本这类"服务回选的事实"记在这里——能力面保持协议中立，不带协议字段。
    log: Arc<dyn crate::kernel::ports::Log + Send + Sync>,
}

impl McpAdapter {
    /// 目的：组合根注入长驻进程端口与运行日志（机制在 kernel 的 `SessionHost` 实现里）。
    pub fn new(
        host: Arc<dyn SessionHost + Send + Sync>,
        log: Arc<dyn crate::kernel::ports::Log + Send + Sync>,
    ) -> McpAdapter {
        McpAdapter { host, log }
    }
}

impl ServiceAdapter for McpAdapter {
    fn id(&self) -> &str {
        ADAPTER_ID
    }

    fn start(
        &self,
        spec: &LaunchSpec,
    ) -> Result<(Box<dyn ServiceInstance>, Vec<Operation>), String> {
        let mut session = self.host.open(&SessionSpec {
            fence: spec.fence.clone(),
            command: spec.command.clone(),
            env: spec.env.clone(),
        })?;
        let mut next_id = 1u64;
        // 握手期间先记下有没有 list_changed（真出现就在第一次调用前刷新一次）。
        let mut list_changed = false;
        // 握手：声明客户端支持的协议版本与能力，并**采用服务回选的版本**（记进运行日志）。
        // 后续消息不携带版本，所以"采用"的行为差异只在能力开关（见 docs/residents/README.md）。
        let hello = request(
            &mut *session,
            &mut next_id,
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "solomni", "version": env!("CARGO_PKG_VERSION") },
            }),
            &mut list_changed,
        )?;
        let picked = hello
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or(PROTOCOL_VERSION);
        if picked == PROTOCOL_VERSION {
            self.log.info(
                "residents::mcp",
                &format!("协议版本 {}（客户端声明）", picked),
            );
        } else {
            self.log.warn(
                "residents::mcp",
                &format!(
                    "服务回选的协议版本 {} 与客户端声明 {} 不同：按服务的版本继续（按行 JSON-RPC 语义不变）",
                    picked, PROTOCOL_VERSION
                ),
            );
        }
        session.send(
            &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string(),
        )?;
        let operations = list_tools(&mut *session, &mut next_id, &mut list_changed)?;
        Ok((
            Box::new(McpInstance {
                session,
                next_id,
                list_changed,
            }),
            operations,
        ))
    }
}

/// 一个跑着的 MCP 会话。
struct McpInstance {
    session: Box<dyn Session>,
    next_id: u64,
    /// 收到过 `notifications/tools/list_changed`：操作清单可能在下次调用前刷新一次。
    list_changed: bool,
}

impl ServiceInstance for McpInstance {
    fn call(&mut self, op: &str, args: &Value) -> Result<String, String> {
        let result = request(
            &mut *self.session,
            &mut self.next_id,
            "tools/call",
            json!({ "name": op, "arguments": args }),
            &mut self.list_changed,
        )?;
        let text = content_text(&result);
        if result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            Err(text)
        } else {
            Ok(text)
        }
    }

    fn stop(&mut self) {
        self.session.kill();
    }

    fn is_alive(&self) -> bool {
        self.session.alive()
    }

    fn take_refreshed_operations(&mut self) -> Option<Vec<Operation>> {
        if !self.list_changed {
            return None;
        }
        match list_tools(
            &mut *self.session,
            &mut self.next_id,
            &mut self.list_changed,
        ) {
            Ok(ops) => {
                // list_tools 期间可能又收到一次 list_changed：保留标志，交给下一次。
                Some(ops)
            }
            Err(_) => {
                // 刷新没成功：保留标志，下一次调用前再试；不静默丢掉这条事实。
                self.list_changed = true;
                None
            }
        }
    }
}

/// 目的：发一条请求并按 id 等它的应答；等服务期间的通知 / 别的消息如实跳过。
/// 返回：应答的 `result`；服务回 `error` 时如实报错。
fn request(
    session: &mut dyn Session,
    next_id: &mut u64,
    method: &str,
    params: Value,
    list_changed: &mut bool,
) -> Result<Value, String> {
    let id = *next_id;
    *next_id += 1;
    let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    session.send(&msg.to_string())?;
    loop {
        let line = session.recv()?;
        let value: Value = serde_json::from_str(&line)
            .map_err(|e| format!("服务返回的不是 JSON：{}（原文：{}）", e, line))?;
        if value.get("id").and_then(Value::as_u64) != Some(id) {
            // 服务改了自己的操作清单：记下这条事实，下次调用前刷新（不在这里打断当前请求）。
            if value.get("method").and_then(Value::as_str)
                == Some("notifications/tools/list_changed")
            {
                *list_changed = true;
            }
            continue;
        }
        if let Some(err) = value.get("error") {
            let message = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("（无说明）");
            return Err(format!("服务在 {} 上报错：{}", method, message));
        }
        return Ok(value.get("result").cloned().unwrap_or(Value::Null));
    }
}

/// 目的：`tools/list`（按游标翻页）→ 常驻服务的操作清单。
/// 说明：`inputSchema` **原样**收进 `Operation.parameters`——它就是 JSON Schema：原生通道直接发，信封通道渲染摘要；核心不拿它做校验。
fn list_tools(
    session: &mut dyn Session,
    next_id: &mut u64,
    list_changed: &mut bool,
) -> Result<Vec<Operation>, String> {
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let params = match &cursor {
            Some(c) => json!({ "cursor": c }),
            None => json!({}),
        };
        let result = request(session, next_id, "tools/list", params, list_changed)?;
        for tool in result
            .get("tools")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let name = tool
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if name.is_empty() {
                continue;
            }
            out.push(Operation {
                name,
                description: tool
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                // 参数契约：`inputSchema` **原样**收下（核心不解释、不校验；服务是参数的权威）。
                parameters: tool.get("inputSchema").filter(|v| v.is_object()).cloned(),
            });
        }
        match result.get("nextCursor").and_then(Value::as_str) {
            Some(c) if !c.is_empty() => cursor = Some(c.to_string()),
            _ => break,
        }
    }
    Ok(out)
}

/// 目的：把 `tools/call` 的回执拼成给上层的文本——`content` 各项**如实**（非文本项标注，不静默丢），
///   并带上 `structuredContent`（较新协议的结构化结果）；一项都没有才回整个结果的 JSON。
fn content_text(result: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();
    for item in result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match item.get("text").and_then(Value::as_str) {
            Some(text) => parts.push(text.to_string()),
            None => parts.push(content_label(item)),
        }
    }
    if let Some(structured) = result.get("structuredContent") {
        parts.push(format!(
            "[structuredContent] {}",
            serde_json::to_string(structured).unwrap_or_else(|_| "{}".to_string())
        ));
    }
    if parts.is_empty() {
        serde_json::to_string(result).unwrap_or_else(|_| "{}".to_string())
    } else {
        parts.join("\n")
    }
}

/// 目的：一个非文本 `content` 项的如实标注（图片 / 音频 / 资源 / 未知类型），不猜内容。
fn content_label(item: &Value) -> String {
    let ty = item
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    match ty {
        "image" | "audio" => {
            let mime = item.get("mimeType").and_then(Value::as_str).unwrap_or("");
            let bytes = item
                .get("data")
                .and_then(Value::as_str)
                .map(str::len)
                .unwrap_or(0);
            format!("[{} {} {} 字节 base64]", ty, mime, bytes)
        }
        "resource" => {
            let uri = item
                .get("resource")
                .and_then(|r| r.get("uri"))
                .and_then(Value::as_str)
                .or_else(|| item.get("uri").and_then(Value::as_str))
                .unwrap_or("");
            format!("[resource {}]", uri)
        }
        _ => format!(
            "[{}] {}",
            ty,
            serde_json::to_string(item).unwrap_or_else(|_| "{}".to_string())
        ),
    }
}
