//! Web 转录中心：tiny_http 服务器 + 长轮询增量事件推送。
//! 只做协议适配（HTTP/长轮询与**入站能力面**之间的转译）；渲染在浏览器（app.js）。
//! 路由由 `presentation/routes.rs` 的目录驱动：匹配不到就 404，目录里有而这里没分支 = 程序缺陷。
//! 核心状态在核心自己的线程上：这里拿不到它、也拿不到任何核心锁，「停止」直接说给核心听。
//! 安全底线：只绑 127.0.0.1；密钥永不进任何响应（能力面只给 id）。

use crate::core::api::{Ops, Output};
use crate::core::providers::AppSettings;
use crate::core::{CollabStep, Pending, SessionEdit, SessionEvent, WorkMode, WorkSpec};
use crate::presentation::{intent, routes};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tiny_http::{Header, Response, Server};

/// 默认监听端口（CLI 的 webui 命令与启动参数共用这一个来源）。
pub const DEFAULT_PORT: u16 = 3081;

/// 本机工具围栏能力（由组合根注入；呈现层如实显示，不假装）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct FenceInfo {
    /// 本机能力：这台机器**能不能**强制文件系统围栏。
    pub fs: bool,
    /// 本机能力：这台机器**能不能**断网。
    pub net: bool,
    /// 本机能力：进程树围栏（未授权时段同样生效）。
    pub tree: bool,
    /// 如实说明（机制名 + 限制）。
    pub note: String,
    /// **本次实际**生效的文件系统围栏（未授权本机写权限时为假：路径级围栏要写目录 ACL 才装得上）。
    pub effective_fs: bool,
    /// **本次实际**生效的断网。
    pub effective_net: bool,
    /// 用户有没有授权在本机写权限项。
    pub write_allowed: bool,
    /// 用户显式授权的只读根个数（`settings.yaml` 的 `fence_read`）；0 = 一个都没授。
    pub read_only_roots: usize,
}

/// 启动转录中心服务器（阻塞直至出错）。端口可指定，默认 3081，只绑本机回环。
pub fn serve(
    ops: Ops,
    port: u16,
    log: Arc<dyn crate::core::ports::Log + Send + Sync>,
    fence: FenceInfo,
) -> Result<(), String> {
    log.info("web::serve", &format!("转录中心启动，端口 {}", port));
    let addr = format!("127.0.0.1:{}", port);
    let server = Server::http(addr.as_str()).map_err(|e| e.to_string())?;
    println!("Solomni 转录中心：http://{}（只监听本机）", addr);
    let fence = Arc::new(fence);
    for request in server.incoming_requests() {
        let ops = ops.clone();
        let fence = Arc::clone(&fence);
        let log = Arc::clone(&log);
        // 每请求一线程：长连接绝不阻塞其它请求（转录中心是多端并用的）。
        std::thread::spawn(move || {
            let mut request = request;
            let url = request.url().to_string();
            let method = request.method().to_string();
            let mut body = String::new();
            let _ = request.as_reader().read_to_string(&mut body);
            log.info("web::request", &format!("{} {}", method, url));

            let (code, headers, text) = route(&ops, &fence, &log, &method, &url, &body);
            let mut response = Response::from_string(text).with_status_code(code);
            for (k, v) in headers {
                if let Ok(h) = Header::from_bytes(k.as_bytes(), v.as_bytes()) {
                    response = response.with_header(h);
                }
            }
            let _ = request.respond(response);
        });
    }
    Ok(())
}

type Reply = (u16, Vec<(&'static str, String)>, String);

fn json_head() -> Vec<(&'static str, String)> {
    vec![(
        "Content-Type",
        "application/json; charset=utf-8".to_string(),
    )]
}

fn static_head(kind: &str) -> Vec<(&'static str, String)> {
    vec![
        ("Content-Type", format!("{}; charset=utf-8", kind)),
        // 静态资源一律 no-store：避免浏览器拿到旧的 app.js/md.js（改了却不生效的经典陷阱）
        ("Cache-Control", "no-store".to_string()),
    ]
}

/// 统一错误响应：形状与既有前端一致（`{error}`）。
fn complaint(code: u16, msg: impl Into<String>) -> Reply {
    (
        code,
        json_head(),
        json!({ "error": msg.into() }).to_string(),
    )
}

/// 统一成功响应（JSON 体）。
fn ok_json(v: serde_json::Value) -> Reply {
    (200, json_head(), v.to_string())
}

/// 解析请求体；失败即给出统一错误文案。
fn parse_body(body: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str::<serde_json::Value>(body).map_err(|e| format!("请求不是 JSON：{}", e))
}

/// 极简 base64 解码（上传用；标准字母表，容忍换行与缺失填充）。
fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' | b' ' | b'\t' => continue,
            _ => return Err("base64 含非法字符".to_string()),
        } as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

fn str_field(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string()
}

fn str_list(v: &serde_json::Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|t| t.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// 长轮询最长等待：有新事件立刻回，否则到点回空（客户端随即再问一次）。
const POLL_WAIT: Duration = Duration::from_secs(20);
const POLL_TICK: Duration = Duration::from_millis(300);

/// 分发一次请求。**契约测试直接调它**（不经过 socket），逐条路由验成功/错误/空/边界。
pub(crate) fn route(
    ops: &Ops,
    fence: &FenceInfo,
    log: &Arc<dyn crate::core::ports::Log + Send + Sync>,
    method: &str,
    url: &str,
    body: &str,
) -> Reply {
    // 匹配交给路由目录；`id` 是分发键，所以代码与目录不会各写一份路径规则。
    let Some(m) = routes::matches(method, url) else {
        return complaint(404, "无此路由");
    };
    let sid = m.param("sid");
    let action = m.param("action");
    let id = m.param("id");
    let name = m.param("name");

    match m.route.id {
        "index" => (
            200,
            static_head("text/html"),
            include_str!("web/index.html").to_string(),
        ),
        "style" => (
            200,
            static_head("text/css"),
            include_str!("web/style.css").to_string(),
        ),
        "app" => (
            200,
            static_head("application/javascript"),
            include_str!("web/app.js").to_string(),
        ),
        "md" => (
            200,
            static_head("application/javascript"),
            include_str!("web/md.js").to_string(),
        ),

        "events" => {
            // 长轮询：?sid= 会话，&since= 客户端已见序号。最多等 20s；有新事件立即回。
            let q = url.split('?').nth(1).unwrap_or("");
            let mut sid: Option<String> = None;
            let mut since = 0u64;
            for pair in q.split('&') {
                let mut kv = pair.splitn(2, '=');
                match (kv.next(), kv.next()) {
                    (Some("sid"), Some(v)) if !v.is_empty() => sid = Some(v.to_string()),
                    (Some("since"), v) => {
                        since = v.and_then(|x| x.parse::<u64>().ok()).unwrap_or(0)
                    }
                    _ => {}
                }
            }
            let deadline = Instant::now() + POLL_WAIT;
            loop {
                // 同一把锁里取「批 + 头部」：客户端据头部推进游标不会漏事件。
                let (lines, head) = ops.events.snapshot(sid.as_deref(), since);
                if !lines.is_empty() || Instant::now() >= deadline {
                    let snap: Vec<serde_json::Value> = lines
                        .iter()
                        .map(
                            |l| json!({ "seq": l.seq, "sid": l.sid, "events": ev_json(&l.events) }),
                        )
                        .collect();
                    return ok_json(json!({ "lines": snap, "head": head }));
                }
                std::thread::sleep(POLL_TICK);
            }
        }

        "state" => match state_json(ops, fence) {
            Ok(v) => ok_json(v),
            Err(e) => complaint(400, e),
        },

        // ---- 创建/操作会话（工作） ----
        "session.new" => {
            let req = match parse_body(body) {
                Ok(v) => v,
                Err(e) => return complaint(400, e),
            };
            let mode = match parse_mode(&str_field(&req, "mode")) {
                Ok(m) => m,
                Err(e) => return complaint(400, e),
            };
            let agents: Vec<crate::core::AgentInstance> = req
                .get("agents")
                .and_then(|t| t.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|x| crate::core::AgentInstance {
                            name: x
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            transient: x
                                .get("transient")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false),
                            modules: x
                                .get("modules")
                                .and_then(|v| v.as_array())
                                .map(|m| {
                                    m.iter()
                                        .filter_map(|s| s.as_str().map(|s| s.to_string()))
                                        .collect()
                                })
                                .unwrap_or_default(),
                            model: x
                                .get("model")
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.is_empty())
                                .map(|s| s.to_string()),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let spec = WorkSpec {
                name: str_field(&req, "name"),
                mode,
                agents,
                task: req
                    .get("task")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string()),
                delegate: req
                    .get("delegate")
                    .and_then(|t| t.as_bool())
                    .unwrap_or(false),
            };
            match ops.sessions.create_work(spec) {
                Ok(o) => ok_json(
                    json!({ "sid": o.sid, "agents": o.agents, "events": ev_json(&o.events) }),
                ),
                Err(e) => {
                    log.error("web::create_work", &format!("创建工作失败：{}", e));
                    complaint(400, e)
                }
            }
        }

        "session.act" => {
            // 「停止」不进命令队列：直接置位核心的取消标志，所以生成期间照样立刻生效。
            if action == "stop" {
                let found = ops.sessions.stop(&sid);
                log.info(
                    "web::stop",
                    &format!("sid={} 找到在跑的生成={}", sid, found),
                );
                return ok_json(json!({ "ok": true }));
            }
            let req = match parse_body(body) {
                Ok(v) => v,
                Err(e) => return complaint(400, e),
            };
            let text = str_field(&req, "text");
            let agent = str_field(&req, "agent");
            // 配置界面：提交编辑（「生成中不许改」的规则在共享意图层收口一次）。
            if action == "edit" {
                let edit = match serde_json::from_value::<SessionEdit>(req.clone()) {
                    Ok(e) => e,
                    Err(e) => return complaint(400, format!("编辑内容非法：{}", e)),
                };
                return match intent::edit_session(ops, &sid, edit) {
                    Ok(()) => ok_json(json!({ "ok": true })),
                    Err(e) => {
                        log.warn("web::edit_session", &format!("sid={} 编辑被拒：{}", sid, e));
                        complaint(400, e)
                    }
                };
            }
            // 上传：把文件写进本次工作的 work/；同名冲突返回 409，由用户决定覆盖/改名。
            if action == "upload" {
                let name = str_field(&req, "name");
                let overwrite = req
                    .get("overwrite")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let bytes = match base64_decode(&str_field(&req, "data_base64")) {
                    Ok(b) => b,
                    Err(e) => return complaint(400, e),
                };
                return match ops.sessions.upload(&sid, &name, &bytes, overwrite) {
                    Ok(true) => ok_json(json!({ "ok": true })),
                    Ok(false) => complaint(409, "同名文件已存在"),
                    Err(e) => complaint(400, e),
                };
            }
            // 名单状态：前端据此决定下一个动作（这一问不产出事件）。
            if action == "pending" {
                return match ops.sessions.pending(&sid) {
                    Ok(p) => ok_json(json!({ "sid": sid, "pending": pending_json(&p) })),
                    Err(e) => complaint(400, e),
                };
            }
            // 生成类动作：流式与否是**显示**的选择，归呈现层（取自设置）。
            let out = match ops.registry.settings() {
                Ok(s) if s.streaming => Output::Stream,
                _ => Output::Final,
            };
            // 其余动作走共享意图层：分发只写一份（新增动作只改 intent::Action）。
            let what = match action.as_str() {
                "say" => intent::Action::Say(&text),
                "continue" => intent::Action::Continue,
                "task" => intent::Action::Step(CollabStep::SetTask, &text),
                "slate" => intent::Action::Step(CollabStep::ConfirmSlate, &text),
                "begin" => intent::Action::Step(CollabStep::Begin, &text),
                "answer" => intent::Action::Step(CollabStep::Answer, &text),
                // 审查关卡：用户点「同意」才开工（方案待审时前端给的就是这个动作）。
                "approve-plan" => intent::Action::Step(CollabStep::ApprovePlan, &text),
                "withdraw" => intent::Action::Withdraw(&agent),
                // 压缩上下文：AI 自己压成摘要（此后此前内容不再发给模型，用户仍可查看）。
                "compact" => intent::Action::Compact,
                "rewind" => intent::Action::Rewind(
                    req.get("id").and_then(|v| v.as_u64()).unwrap_or(u64::MAX),
                ),
                "update-task" => intent::Action::UpdateTask(&text),
                _ => return complaint(400, format!("未知动作：{}", action)),
            };
            match intent::act(ops, &sid, what, out) {
                // 动作回包即时返回本批事件与事件台序号；同批也早已入台供其它端增量取。
                // 客户端按 seq 去重，避免「动作回包 + 长轮询」把同一批事件派发两次。
                Ok(intent::Acted::Advanced(adv)) => {
                    ok_json(json!({ "sid": sid, "events": ev_json(&adv.events), "seq": adv.seq }))
                }
                // 回档 / 改需求返回完整重放（前端整体重建）。
                Ok(intent::Acted::Replayed(events)) => {
                    ok_json(json!({ "sid": sid, "events": events }))
                }
                Err(e) => {
                    log.error(
                        "web::session_action",
                        &format!("会话动作 {} 失败：{}", action, e),
                    );
                    complaint(400, e)
                }
            }
        }

        // 配置视图（会话界面之外）：能改什么、现在是什么、缺什么，一次性如实给出。
        "session.config" => match ops.sessions.config(&sid) {
            Ok(config) => ok_json(json!({ "config": config })),
            Err(e) => complaint(400, e),
        },
        // 会话文件清单 + 真实根（前端 @ 菜单与长路径缩写）。
        "session.files" => match ops.sessions.files(&sid) {
            Ok(v) => ok_json(json!({ "work": v.work, "agents": v.agents, "roots": v.roots })),
            Err(e) => complaint(404, e),
        },

        // ---- 供应商 ----
        "provider.new" => {
            let req = match parse_body(body) {
                Ok(v) => v,
                Err(e) => return complaint(400, e),
            };
            match ops.registry.upsert_provider(
                &str_field(&req, "id"),
                &str_field(&req, "base_url"),
                &str_field(&req, "api_key"),
            ) {
                Ok(()) => ok_json(json!({ "ok": true })),
                Err(e) => complaint(400, e),
            }
        }
        "provider.act" => {
            let outcome = match action.as_str() {
                "remove" => ops
                    .registry
                    .remove_provider(&id)
                    .map(|ok| json!({ "ok": ok })),
                "discover" => ops
                    .registry
                    .discover_models(&id)
                    .map(|models| json!({ "ok": true, "models": models })),
                _ => return complaint(404, format!("未知动作：{}", action)),
            };
            match outcome {
                Ok(v) => ok_json(v),
                Err(e) => complaint(400, e),
            }
        }

        // ---- 模型 ----
        "model.new" => {
            let req = match parse_body(body) {
                Ok(v) => v,
                Err(e) => return complaint(400, e),
            };
            match ops.registry.upsert_model(
                &str_field(&req, "id"),
                &str_field(&req, "name"),
                &str_field(&req, "api_model"),
                &str_field(&req, "provider"),
                &str_field(&req, "note"),
                req.get("context").and_then(|v| v.as_u64()).unwrap_or(0),
            ) {
                Ok(()) => ok_json(json!({ "ok": true })),
                Err(e) => complaint(400, e),
            }
        }
        "model.act" => {
            let outcome = match action.as_str() {
                "remove" => ops.registry.remove_model(&id).map(|ok| json!({ "ok": ok })),
                "core" => ops
                    .registry
                    .set_core_model(&id)
                    .map(|ok| json!({ "ok": ok })),
                // 探测要真实网络（两条最小请求），结论由 core 按三种如实回报并只写确定的结论。
                "probe" => ops
                    .registry
                    .probe_model_tools(&id)
                    .map(|outcome| probe_json(ops, &id, &outcome)),
                // 回放形状探测：只报事实、不改登记处（采不采用由人定）。
                "probe-replay" => ops
                    .registry
                    .probe_replay_shape(&id)
                    .map(|report| json!({ "ok": true, "shapes": report.shapes })),
                _ => return complaint(404, format!("未知动作：{}", action)),
            };
            match outcome {
                Ok(v) => ok_json(v),
                Err(e) => complaint(400, e),
            }
        }

        // ---- agent ----
        "agent.new" => {
            let req = match parse_body(body) {
                Ok(v) => v,
                Err(e) => return complaint(400, e),
            };
            match ops.registry.upsert_agent(
                &str_field(&req, "name"),
                &str_list(&req, "modules"),
                &str_field(&req, "model"),
                &str_field(&req, "note"),
            ) {
                Ok(()) => ok_json(json!({ "ok": true })),
                Err(e) => complaint(400, e),
            }
        }
        "agent.act" => {
            let outcome = match action.as_str() {
                "remove" => ops
                    .registry
                    .remove_agent(&name)
                    .map(|ok| json!({ "ok": ok })),
                _ => return complaint(404, format!("未知动作：{}", action)),
            };
            match outcome {
                Ok(v) => ok_json(v),
                Err(e) => complaint(400, e),
            }
        }

        // ---- 基本设置 ----
        "settings.get" => match ops.registry.settings() {
            Ok(s) => ok_json(json!({ "settings": s })),
            Err(e) => complaint(400, e),
        },
        "settings.set" => {
            let req = match parse_body(body) {
                Ok(v) => v,
                Err(e) => return complaint(400, e),
            };
            let current = match ops.registry.settings() {
                Ok(s) => s,
                Err(e) => return complaint(400, e),
            };
            let settings = AppSettings {
                streaming: req
                    .get("streaming")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(current.streaming),
                show_reasoning: req
                    .get("show_reasoning")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(current.show_reasoning),
                // 执行档位与围栏写权限：界面暂未暴露（后续阶段），改设置只保留现有值。
                tier: current.tier,
                fence_write: current.fence_write,
                fence_read: current.fence_read.clone(),
                qemu_path: current.qemu_path.clone(),
                llm_timeout_secs: req
                    .get("llm_timeout_secs")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(current.llm_timeout_secs),
                // 压缩阈值可由界面调；缺省沿用现值。
                compact_at_percent: req
                    .get("compact_at_percent")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u8)
                    .unwrap_or(current.compact_at_percent),
                discuss_remind_cap: req
                    .get("discuss_remind_cap")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32)
                    .unwrap_or(current.discuss_remind_cap),
            };
            match ops.registry.set_settings(settings) {
                Ok(()) => ok_json(json!({ "ok": true })),
                Err(e) => complaint(400, e),
            }
        }

        // ---- 会话历史 ----
        "history.list" => match ops.history.list() {
            Ok(sessions) => ok_json(json!({ "sessions": sessions })),
            Err(e) => complaint(400, e),
        },
        "history.open" => match ops.history.open(&name) {
            Ok((meta, events)) => ok_json(json!({ "meta": meta, "events": events })),
            Err(e) => complaint(404, e),
        },
        "history.delete" => match ops.history.delete(&name) {
            Ok(ok) => ok_json(json!({ "ok": ok })),
            Err(e) => complaint(400, e),
        },

        // ---- 核心推荐模型 ----
        "suggest" => {
            let req = match parse_body(body) {
                Ok(v) => v,
                Err(e) => return complaint(400, e),
            };
            let task = str_field(&req, "task");
            let mode = match parse_mode(&str_field(&req, "mode")) {
                Ok(m) => m,
                Err(e) => return complaint(400, e),
            };
            match ops.discovery.suggest_models(&task, mode) {
                Ok(agents) => ok_json(json!({ "ok": true, "agents": agents })),
                Err(e) => complaint(400, e),
            }
        }

        // 目录里有、这里却没有分支 = 程序缺陷（契约测试会逐条点名）。
        _ => complaint(500, "路由目录与处理器不一致（程序缺陷）"),
    }
}

/// 探测结论 → 响应 JSON。三种结论如实给出（不猜）；`mode` 是探测后登记处里的**实际**形态，
/// 也就是下一次生成会走的那套协议（无法判定时登记处不变，它就是原样）。
fn probe_json(
    ops: &Ops,
    id: &str,
    outcome: &crate::core::ports::ProbeOutcome,
) -> serde_json::Value {
    use crate::core::ports::ProbeOutcome;
    let (kind, detail) = match outcome {
        ProbeOutcome::Supported { detail } => ("supported", detail.clone()),
        ProbeOutcome::Unsupported { detail } => ("unsupported", detail.clone()),
        ProbeOutcome::Unknown { detail } => ("unknown", detail.clone()),
    };
    let mode = ops
        .registry
        .models()
        .ok()
        .and_then(|ms| ms.into_iter().find(|m| m.id == id))
        .map(|m| m.tools);
    json!({ "ok": true, "outcome": kind, "detail": detail, "mode": mode })
}

/// 请求里的形态标识 → WorkMode：唯一解析处；未知值如实报错（路由据此回 400）。
pub fn parse_mode(s: &str) -> Result<WorkMode, String> {
    match s {
        "single" => Ok(WorkMode::Single),
        "collab" => Ok(WorkMode::Collab),
        other => Err(format!("未知模式：{}（只接受 single / collab）", other)),
    }
}

/// 概览状态：模块清单 + 供应商/模型视图 + 核心默认 + 进行中会话（均无密钥）。
fn state_json(ops: &Ops, fence: &FenceInfo) -> Result<serde_json::Value, String> {
    let roster = ops.discovery.roster()?;
    // 会话形态取落盘 meta（单一真相）：只读一次盘，sessions 与 history 共用。
    let history = ops.history.list()?;
    Ok(json!({
        "modules": roster.modules.iter().map(|m| json!({
            "id": m.manifest.id,
            "brief": m.manifest.brief,
        })).collect::<Vec<_>>(),
        "rejected": roster.rejected,
        // 本机工具围栏的实际能力（如实显示，不假装）：哪些维度真的被强制了。
        "fence": fence,
        "providers": ops.registry.providers()?,
        "models": ops.registry.models()?,
        "core": ops.registry.core_model()?,
        "agents": ops.registry.agents()?,
        "settings": ops.registry.settings()?,
        "sessions": ops.history.session_views(&history)?,
        "history": history,
    }))
}

/// 事件 → JSON（线格式唯一定义在 core::events::SessionEvent::to_json）。
fn ev_json(events: &[SessionEvent]) -> serde_json::Value {
    serde_json::Value::Array(events.iter().map(|e| e.to_json()).collect())
}

/// Pending 的 JSON 形态（前端决定下一个动作）。
pub fn pending_json(p: &Option<Pending>) -> serde_json::Value {
    match p {
        None => json!(null),
        Some(Pending::Ask { member, question }) => {
            json!({ "type": "ask", "member": member, "question": question })
        }
        Some(Pending::ConfirmSlate) => json!({ "type": "confirm_slate" }),
        Some(Pending::ConfirmBegin) => json!({ "type": "confirm_begin" }),
        Some(Pending::PlanReview) => json!({ "type": "plan_review" }),
        Some(Pending::NodeBlocked { nodes }) => json!({ "type": "node_blocked", "nodes": nodes }),
    }
}
