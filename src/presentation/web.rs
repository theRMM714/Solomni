//! Web 转录中心：tiny_http 服务器 + 流式 SSE 实时推送。
//! 只做协议适配（HTTP/SSE 与 Core 门面之间的转译）；渲染在浏览器（app.js）。
//! 安全底线：只绑 127.0.0.1；密钥永不进任何响应（门面已保证前端只见 id）。

use crate::core::{CollabStep, Core, Pending, SessionEvent};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tiny_http::{Header, Response, Server};

type SharedCore = Arc<Mutex<Core>>;

/// 启动转录中心服务器（阻塞直至出错）。端口可指定，默认 3081，只绑本机回环。
pub fn serve(core: SharedCore, port: u16, log: std::sync::Arc<dyn crate::core::ports::Log + Send + Sync>) -> Result<(), String> {
    log.info("web::serve", &format!("转录中心启动，端口 {}", port));
    let addr = format!("127.0.0.1:{}", port);
    let server = Server::http(addr.as_str()).map_err(|e| e.to_string())?;
    println!("Solomni 转录中心：http://{}（只监听本机）", addr);
    // 广播总线：每个 SSE 订阅者一个通道；断开的订阅者在下次广播时自然清理。
    // 发件箱：会话事件按序号累积；长轮询按 since 增量取（取代流式 SSE）。
    let bus: Arc<Mutex<Outbox>> = Arc::new(Mutex::new(Outbox::default()));

    let log_req = std::sync::Arc::clone(&log);
    for request in server.incoming_requests() {
        let core = Arc::clone(&core);
        let bus = Arc::clone(&bus);
        let log = std::sync::Arc::clone(&log_req);
        // 每请求一线程：SSE 长连接绝不阻塞其它请求（转录中心是多端并用的）。
        std::thread::spawn(move || {
            let mut request = request;
            let url = request.url().to_string();
            let method = request.method().to_string();
            let mut body = String::new();
            let _ = request.as_reader().read_to_string(&mut body);
            log.info("web::request", &format!("{} {}", method, url));

            let resp = route(&core, &bus, &log, &method, &url, &body);
            let mut response = Response::from_string(resp.2).with_status_code(resp.0);
            for (k, v) in resp.1 {
                if let Ok(h) = Header::from_bytes(k.as_bytes(), v.as_bytes()) {
                    response = response.with_header(h);
                }
            }
            let _ = request.respond(response);
        });
    }
    Ok(())
}

fn json_head() -> Vec<(&'static str, String)> {
    vec![("Content-Type", "application/json; charset=utf-8".to_string())]
}

/// 路由：返回 (状态码, 响应头, 内容)。
type Bus = Arc<Mutex<Outbox>>;

fn route(core: &SharedCore, bus: &Bus, log: &std::sync::Arc<dyn crate::core::ports::Log + Send + Sync>, method: &str, url: &str, body: &str) -> (u16, Vec<(&'static str, String)>, String) {
    match (method, url) {
        ("GET", "/") => (
            200,
            vec![("Content-Type", "text/html; charset=utf-8".to_string())],
            include_str!("web/index.html").to_string(),
        ),
        ("GET", "/style.css") => (
            200,
            vec![("Content-Type", "text/css; charset=utf-8".to_string())],
            include_str!("web/style.css").to_string(),
        ),
        ("GET", "/app.js") => (
            200,
            vec![("Content-Type", "application/javascript; charset=utf-8".to_string())],
            include_str!("web/app.js").to_string(),
        ),
        ("GET", path) if path.starts_with("/api/events?") => {
            // 长轮询：?sid= 会话，&since= 客户端已见序号。最多等 20s；有新事件立即回。
            let q = &path["/api/events?".len()..];
            let mut sid = None;
            let mut since = 0u64;
            for pair in q.split('&') {
                let mut kv = pair.splitn(2, '=');
                match (kv.next(), kv.next()) {
                    (Some("sid"), v) => sid = v.and_then(|x| x.parse::<u64>().ok()),
                    (Some("since"), v) => since = v.and_then(|x| x.parse::<u64>().ok()).unwrap_or(0),
                    _ => {}
                }
            }
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            loop {
                let snap = bus.lock().expect("bus 锁").snapshot(sid, since);
                if !snap.is_empty() || std::time::Instant::now() >= deadline {
                    let head = bus.lock().expect("bus 锁").head();
                    return (200, json_head(), json!({ "lines": snap, "head": head }).to_string());
                }
                std::thread::sleep(Duration::from_millis(300));
            }
        }
        ("GET", "/api/state") => {
            let c = core.lock().expect("core 锁");
            (200, json_head(), state_json(&c).to_string())
        }
        ("POST", "/api/sessions") => {
            let req: serde_json::Value = match serde_json::from_str(body) {
                Ok(v) => v,
                Err(e) => return (400, json_head(), json!({ "error": format!("请求不是 JSON：{}", e) }).to_string()),
            };
            let mode = req.get("mode").and_then(|m| m.as_str()).unwrap_or("").to_string();
            let ids = req.get("ids").and_then(|m| m.as_str()).unwrap_or("").to_string();
            let mut c = core.lock().expect("core 锁");
            let outcome = match mode.as_str() {
                "direct" => c.start_direct(&ids).map(|(sid, ev)| json!({ "sid": sid, "events": ev_json(&ev) })),
                "omni" => c.start_omni(&ids).map(|(sid, ev)| json!({ "sid": sid, "events": ev_json(&ev) })),
                "collab" => c.start_collab(&ids).map(|sid| json!({ "sid": sid, "events": [] })),
                _ => Err("mode 必须是 direct / omni / collab".to_string()),
            };
            match outcome {
                Ok(v) => (200, json_head(), v.to_string()),
                Err(e) => {
                    log.error("web::create_session", &format!("建会话失败：{}", e));
                    (400, json_head(), json!({ "error": e }).to_string())
                }
            }
        }
        ("POST", path) if path.starts_with("/api/sessions/") => {
            let rest = &path["/api/sessions/".len()..];
            let (sid_str, action) = match rest.split_once('/') {
                Some(x) => x,
                None => return (404, json_head(), json!({ "error": "路径应为 /api/sessions/{id}/{action}" }).to_string()),
            };
            let Ok(sid) = sid_str.parse::<u64>() else {
                return (400, json_head(), json!({ "error": "会话 id 非法" }).to_string());
            };
            let req: serde_json::Value = match serde_json::from_str(body) {
                Ok(v) => v,
                Err(e) => return (400, json_head(), json!({ "error": format!("请求不是 JSON：{}", e) }).to_string()),
            };
            let text = req.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string();
            let mut c = core.lock().expect("core 锁");
            let outcome = match action {
                "say" => c.direct_say(sid, &text).or_else(|_| c.omni_say(sid, &text)),
                "task" => c.collab_continue(sid, CollabStep::SetTask, &text),
                "slate" => c.collab_continue(sid, CollabStep::ConfirmSlate, &text),
                "begin" => c.collab_continue(sid, CollabStep::Begin, &text),
                "answer" => c.collab_continue(sid, CollabStep::Answer, &text),
                "pending" => c.collab_pending(sid).map(|_| Vec::new()),
                _ => Err(format!("未知动作：{}", action)),
            };
            match outcome {
                Ok(events) => {
                    let reply = if action == "pending" {
                        let p = c.collab_pending(sid).unwrap_or(None);
                        json!({ "sid": sid, "pending": pending_json(&p) }).to_string()
                    } else {
                        // 动作回包即时返回本批事件；同时入发件箱供长轮询增量取（多端同看）。
                        let line = json!({ "sid": sid, "events": ev_json(&events) }).to_string();
                        bus.lock().expect("bus 锁").push(sid, &line);
                        line
                    };
                    (200, json_head(), reply)
                }
                Err(e) => {
                    log.error("web::session_action", &format!("会话动作 {} 失败：{}", action, e));
                    (400, json_head(), json!({ "error": e }).to_string())
                }
            }
        }
        ("POST", "/api/providers") => {
            let req: serde_json::Value = match serde_json::from_str(body) {
                Ok(v) => v,
                Err(e) => return (400, json_head(), json!({ "error": format!("请求不是 JSON：{}", e) }).to_string()),
            };
            let id = req.get("id").and_then(|t| t.as_str()).unwrap_or("").to_string();
            let base_url = req.get("base_url").and_then(|t| t.as_str()).unwrap_or("").to_string();
            let api_key = req.get("api_key").and_then(|t| t.as_str()).unwrap_or("").to_string();
            let models: Vec<String> = req
                .get("models")
                .and_then(|t| t.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
                .unwrap_or_default();
            let mut c = core.lock().expect("core 锁");
            match c.provider_upsert(&id, &base_url, &api_key, &models) {
                Ok(()) => (200, json_head(), json!({ "ok": true }).to_string()),
                Err(e) => (400, json_head(), json!({ "error": e }).to_string()),
            }
        }
        ("POST", path) if path.starts_with("/api/providers/") => {
            let rest = &path["/api/providers/".len()..];
            let (id, action) = match rest.split_once('/') {
                Some(x) => x,
                None => return (404, json_head(), json!({ "error": "路径应为 /api/providers/{id}/{action}" }).to_string()),
            };
            let mut c = core.lock().expect("core 锁");
            let outcome = match action {
                "default" => c.provider_set_default(id).map(|ok| json!({ "ok": ok })),
                "remove" => c.provider_remove(id).map(|ok| json!({ "ok": ok })),
                _ => Err(format!("未知动作：{}", action)),
            };
            match outcome {
                Ok(v) => (200, json_head(), v.to_string()),
                Err(e) => (400, json_head(), json!({ "error": e }).to_string()),
            }
        }
        _ => (404, json_head(), json!({ "error": "无此路由" }).to_string()),
    }
}

/// 概览状态：模块清单 + 供应商视图（无密钥）。
fn state_json(core: &Core) -> serde_json::Value {
    let roster = core.scan();
    json!({
        "modules": roster.modules.iter().map(|m| json!({
            "id": m.manifest.id,
            "brief": m.manifest.brief,
        })).collect::<Vec<_>>(),
        "rejected": roster.rejected,
        "providers": core.provider_views(),
    })
}

/// 事件 → JSON。
fn ev_json(events: &[SessionEvent]) -> serde_json::Value {
    serde_json::to_value(events.iter().map(ev_to_json).collect::<Vec<_>>()).unwrap_or(json!([]))
}

fn ev_to_json(e: &SessionEvent) -> serde_json::Value {
    match e {
        SessionEvent::Notice(n) => json!({ "type": "notice", "text": n }),
        SessionEvent::Transcript(lines) => json!({ "type": "transcript", "lines": lines }),
        SessionEvent::DiscussionDone { round, over_cap } => {
            json!({ "type": "discussion_done", "round": round, "over_cap": over_cap })
        }
        SessionEvent::Plan(p) => json!({ "type": "plan", "text": p }),
        SessionEvent::Report { id, text, rework } => json!({ "type": "report", "id": id, "text": text, "rework": rework }),
        SessionEvent::Review { items, raw } => json!({ "type": "review", "items": items, "raw": raw }),
        SessionEvent::Delivery { ok, over_rework } => json!({ "type": "delivery", "ok": ok, "over_rework": over_rework }),
        SessionEvent::Ended => json!({ "type": "ended" }),
    }
}

/// Pending 的 JSON 形态（前端决定下一个动作）。
pub fn pending_json(p: &Option<Pending>) -> serde_json::Value {
    match p {
        None => json!(null),
        Some(Pending::Ask { member, question }) => json!({ "type": "ask", "member": member, "question": question }),
        Some(Pending::ConfirmSlate) => json!({ "type": "confirm_slate" }),
        Some(Pending::ConfirmBegin) => json!({ "type": "confirm_begin" }),
    }
}

/// 服务器端发件箱：事件行按全局序号累积；每个会话独立索引。
#[derive(Default)]
struct Outbox {
    seq: u64,
    lines: Vec<(u64, u64, String)>, // (全局序号, 会话 id, JSON 行)
}

impl Outbox {
    fn push(&mut self, sid: u64, line: &str) {
        self.seq += 1;
        self.lines.push((self.seq, sid, line.to_string()));
        // 上限保护：本地单机足够，防无界增长。
        if self.lines.len() > 10_000 {
            let drop = self.lines.len() - 5_000;
            self.lines.drain(..drop);
        }
    }
    /// 取某会话（None=全部）since 之后的事件行。
    fn snapshot(&self, sid: Option<u64>, since: u64) -> Vec<String> {
        self.lines
            .iter()
            .filter(|(sq, s, _)| *sq > since && sid.map_or(true, |x| *s == x))
            .map(|(_, _, l)| l.clone())
            .collect()
    }
    /// 取全局最新序号（客户端首轮同步用）。
    fn head(&self) -> u64 {
        self.seq
    }
}