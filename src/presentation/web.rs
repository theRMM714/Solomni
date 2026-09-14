//! Web 转录中心：tiny_http 服务器 + 长轮询增量事件推送。
//! 只做协议适配（HTTP/长轮询与 Core 门面之间的转译）；渲染在浏览器（app.js）。
//! 安全底线：只绑 127.0.0.1；密钥永不进任何响应（门面已保证前端只见 id）。

use crate::core::providers::AppSettings;
use crate::core::{
    AgentInstance, CollabStep, Core, Live, Pending, SessionEdit, SessionEvent, WorkMode, WorkSpec,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tiny_http::{Header, Response, Server};

type SharedCore = Arc<Mutex<Core>>;

/// 默认监听端口（CLI 的 webui 命令与启动参数共用这一个来源）。
pub const DEFAULT_PORT: u16 = 3081;

/// 本机工具围栏能力（由组合根注入；呈现层如实显示，不假装）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct FenceInfo {
    pub fs: bool,
    pub net: bool,
    pub tree: bool,
    pub note: String,
}

/// 启动转录中心服务器（阻塞直至出错）。端口可指定，默认 3081，只绑本机回环。
pub fn serve(
    core: SharedCore,
    port: u16,
    log: std::sync::Arc<dyn crate::core::ports::Log + Send + Sync>,
    fence: FenceInfo,
) -> Result<(), String> {
    log.info("web::serve", &format!("转录中心启动，端口 {}", port));
    let addr = format!("127.0.0.1:{}", port);
    let server = Server::http(addr.as_str()).map_err(|e| e.to_string())?;
    println!("Solomni 转录中心：http://{}（只监听本机）", addr);
    // 发件箱：会话事件按序号累积；长轮询按 since 增量取（取代流式 SSE）。
    let bus: Arc<Mutex<Outbox>> = Arc::new(Mutex::new(Outbox::default()));
    // 中止开关表：/stop 只碰它，不碰核心锁——生成中核心锁被占用，否则停不下来。
    let cancels: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>> = Arc::new(Mutex::new(HashMap::new()));

    let fence = Arc::new(fence);
    let log_req = std::sync::Arc::clone(&log);
    for request in server.incoming_requests() {
        let core = Arc::clone(&core);
        let bus = Arc::clone(&bus);
        let cancels = Arc::clone(&cancels);
        let fence = Arc::clone(&fence);
        let log = std::sync::Arc::clone(&log_req);
        // 每请求一线程：长连接绝不阻塞其它请求（转录中心是多端并用的）。
        std::thread::spawn(move || {
            let mut request = request;
            let url = request.url().to_string();
            let method = request.method().to_string();
            let mut body = String::new();
            let _ = request.as_reader().read_to_string(&mut body);
            log.info("web::request", &format!("{} {}", method, url));

            let resp = route(&core, &bus, &cancels, &fence, &log, &method, &url, &body);
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

/// 路径段百分号解码（%XX；路径里没有 '+' 语义）。前端用 encodeURIComponent，故必须解回来。
fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn str_field(v: &serde_json::Value, key: &str) -> String {
    v.get(key).and_then(|t| t.as_str()).unwrap_or("").to_string()
}

fn str_list(v: &serde_json::Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|t| t.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default()
}

type Cancels = Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>;

fn route(
    core: &SharedCore,
    bus: &Bus,
    cancels: &Cancels,
    fence: &FenceInfo,
    log: &std::sync::Arc<dyn crate::core::ports::Log + Send + Sync>,
    method: &str,
    url: &str,
    body: &str,
) -> (u16, Vec<(&'static str, String)>, String) {
    match (method, url) {
        // 静态资源一律 no-store：避免浏览器拿到旧的 app.js/md.js（改了却不生效的经典陷阱）
        ("GET", "/") => {
            let mut h = vec![("Content-Type", "text/html; charset=utf-8".to_string())];
            h.push(("Cache-Control", "no-store".to_string()));
            (200, h, include_str!("web/index.html").to_string())
        }
        ("GET", "/style.css") => {
            let mut h = vec![("Content-Type", "text/css; charset=utf-8".to_string())];
            h.push(("Cache-Control", "no-store".to_string()));
            (200, h, include_str!("web/style.css").to_string())
        }
        ("GET", "/app.js") => {
            let mut h = vec![("Content-Type", "application/javascript; charset=utf-8".to_string())];
            h.push(("Cache-Control", "no-store".to_string()));
            (200, h, include_str!("web/app.js").to_string())
        }
        ("GET", "/md.js") => {
            let mut h = vec![("Content-Type", "application/javascript; charset=utf-8".to_string())];
            h.push(("Cache-Control", "no-store".to_string()));
            (200, h, include_str!("web/md.js").to_string())
        }
        ("GET", path) if path.starts_with("/api/events?") => {
            // 长轮询：?sid= 会话，&since= 客户端已见序号。最多等 20s；有新事件立即回。
            let q = &path["/api/events?".len()..];
            let mut sid: Option<String> = None;
            let mut since = 0u64;
            for pair in q.split('&') {
                let mut kv = pair.splitn(2, '=');
                match (kv.next(), kv.next()) {
                    (Some("sid"), Some(v)) if !v.is_empty() => sid = Some(v.to_string()),
                    (Some("since"), v) => since = v.and_then(|x| x.parse::<u64>().ok()).unwrap_or(0),
                    _ => {}
                }
            }
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            loop {
                let snap = bus.lock().expect("bus 锁").snapshot(sid.as_deref(), since);
                if !snap.is_empty() || std::time::Instant::now() >= deadline {
                    let head = bus.lock().expect("bus 锁").head();
                    return (200, json_head(), json!({ "lines": snap, "head": head }).to_string());
                }
                std::thread::sleep(Duration::from_millis(300));
            }
        }
        ("GET", "/api/state") => {
            let c = core.lock().expect("core 锁");
            (200, json_head(), state_json(&c, fence).to_string())
        }

        // ---- 创建/操作会话（工作） ----
        ("POST", "/api/sessions") => {
            let req = match parse_body(body) {
                Ok(v) => v,
                Err(e) => return (400, json_head(), json!({ "error": e }).to_string()),
            };
            let mode = match parse_mode(&str_field(&req, "mode")) {
                Ok(m) => m,
                Err(e) => return (400, json_head(), json!({ "error": e }).to_string()),
            };
            let agents: Vec<AgentInstance> = req
                .get("agents")
                .and_then(|t| t.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|x| AgentInstance {
                            name: x.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            transient: x.get("transient").and_then(|v| v.as_bool()).unwrap_or(false),
                            modules: x
                                .get("modules")
                                .and_then(|v| v.as_array())
                                .map(|m| m.iter().filter_map(|s| s.as_str().map(|s| s.to_string())).collect())
                                .unwrap_or_default(),
                            model: x.get("model").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(|s| s.to_string()),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let task = req.get("task").and_then(|t| t.as_str()).map(|s| s.to_string());
            let spec = WorkSpec {
                name: str_field(&req, "name"),
                mode,
                agents,
                task,
                delegate: req.get("delegate").and_then(|t| t.as_bool()).unwrap_or(false),
            };
            let mut c = core.lock().expect("core 锁");
            match c.create_work(spec) {
                Ok(o) => (
                    200,
                    json_head(),
                    json!({ "sid": o.sid, "agents": o.agents, "events": ev_json(&o.events) }).to_string(),
                ),
                Err(e) => {
                    log.error("web::create_work", &format!("创建工作失败：{}", e));
                    (400, json_head(), json!({ "error": e }).to_string())
                }
            }
        }
        ("POST", path) if path.starts_with("/api/sessions/") => {
            let rest = &path["/api/sessions/".len()..];
            let (sid, action) = match rest.split_once('/') {
                Some(x) => x,
                None => return (404, json_head(), json!({ "error": "路径应为 /api/sessions/{sid}/{action}" }).to_string()),
            };
            let sid = url_decode(sid);
            // 「停止」不需要核心锁：生成期间核心锁被占用，只有这里能立刻置位中止。
            if action == "stop" {
                let found = match cancels.lock().expect("cancels 锁").get(&sid).cloned() {
                    Some(flag) => {
                        flag.store(true, Ordering::Relaxed);
                        true
                    }
                    None => false,
                };
                log.info("web::stop", &format!("sid={} 找到在跑的生成={}", sid, found));
                return (200, json_head(), json!({ "ok": true }).to_string());
            }
            let req = match parse_body(body) {
                Ok(v) => v,
                Err(e) => return (400, json_head(), json!({ "error": e }).to_string()),
            };
            let text = str_field(&req, "text");
            let mut c = core.lock().expect("core 锁");
            // 回档返回重放后的完整事件流（前端整体重建），与其它动作的增量回包不同。
            if action == "rewind" {
                let id = req.get("id").and_then(|v| v.as_u64()).unwrap_or(u64::MAX);
                return match c.rewind(&sid, id) {
                    Ok(events) => (200, json_head(), json!({ "sid": sid, "events": events }).to_string()),
                    Err(e) => (400, json_head(), json!({ "error": e }).to_string()),
                };
            }
            // 改需求同样返回完整重放（前端整体重建）。
            if action == "update-task" {
                return match c.update_task(&sid, &text) {
                    Ok(events) => (200, json_head(), json!({ "sid": sid, "events": events }).to_string()),
                    Err(e) => (400, json_head(), json!({ "error": e }).to_string()),
                };
            }
            // 配置界面：提交编辑。正在生成中不许改（先停止或等它结束），避免改到一半的语义。
            if action == "edit" {
                if cancels.lock().expect("cancels 锁").contains_key(&sid) {
                    return (
                        400,
                        json_head(),
                        json!({ "error": "该会话正在生成中：先「停止」或等它结束，再改配置" }).to_string(),
                    );
                }
                let edit = match serde_json::from_value::<SessionEdit>(req.clone()) {
                    Ok(e) => e,
                    Err(e) => return (400, json_head(), json!({ "error": format!("编辑内容非法：{}", e) }).to_string()),
                };
                return match c.edit_session(&sid, edit) {
                    Ok(()) => (200, json_head(), json!({ "ok": true }).to_string()),
                    Err(e) => {
                        log.warn("web::edit_session", &format!("sid={} 编辑被拒：{}", sid, e));
                        (400, json_head(), json!({ "error": e }).to_string())
                    }
                };
            }
            // 上传：把文件写进本次工作的 work/；同名冲突返回 409，由用户决定覆盖/改名。
            if action == "upload" {
                let name = str_field(&req, "name");
                let overwrite = req.get("overwrite").and_then(|v| v.as_bool()).unwrap_or(false);
                let bytes = match base64_decode(&str_field(&req, "data_base64")) {
                    Ok(b) => b,
                    Err(e) => return (400, json_head(), json!({ "error": e }).to_string()),
                };
                return match c.work_upload(&sid, &name, &bytes, overwrite) {
                    Ok(true) => (200, json_head(), json!({ "ok": true }).to_string()),
                    Ok(false) => (409, json_head(), json!({ "error": "同名文件已存在", "exists": true }).to_string()),
                    Err(e) => (400, json_head(), json!({ "error": e }).to_string()),
                };
            }
            // 流式增量：边收边塞进发件箱，由长轮询实时送达（不落盘）。
            let streaming = c.app_settings().streaming;
            let generate = action == "say" || action == "continue";
            let cancel = Arc::new(AtomicBool::new(false));
            if generate {
                cancels.lock().expect("cancels 锁").insert(sid.clone(), Arc::clone(&cancel));
            }
            let bus_live = Arc::clone(bus);
            let sid_live = sid.clone();
            let mut emit = move |ev: SessionEvent| {
                let evs = serde_json::Value::Array(vec![ev.to_json()]);
                bus_live.lock().expect("bus 锁").push(&sid_live, evs);
            };
            let mut live = Live { stream: streaming, cancel: Arc::clone(&cancel), emit: &mut emit };
            let outcome = match action {
                "say" => c.single_say(&sid, &text, &mut live),
                "task" => c.collab_continue(&sid, CollabStep::SetTask, &text),
                "slate" => c.collab_continue(&sid, CollabStep::ConfirmSlate, &text),
                "begin" => c.collab_continue(&sid, CollabStep::Begin, &text),
                "answer" => c.collab_continue(&sid, CollabStep::Answer, &text),
                "continue" => c.continue_flow(&sid, &mut live),
                "withdraw" => c.withdraw_agree(&sid, &str_field(&req, "agent")),
                "pending" => c.collab_pending(&sid).map(|_| Vec::new()),
                _ => Err(format!("未知动作：{}", action)),
            };
            if generate {
                cancels.lock().expect("cancels 锁").remove(&sid);
            }
            match outcome {
                Ok(events) => {
                    let reply = if action == "pending" {
                        let p = c.collab_pending(&sid).unwrap_or(None);
                        json!({ "sid": sid, "pending": pending_json(&p) }).to_string()
                    } else {
                        // 动作回包即时返回本批事件并带发件箱序号；同批也入发件箱供其它端增量取。
                        // 客户端按 seq 去重，避免「动作回包 + 长轮询」把同一批事件派发两次。
                        let evs = ev_json(&events);
                        let seq = bus.lock().expect("bus 锁").push(&sid, evs.clone());
                        json!({ "sid": sid, "events": evs, "seq": seq }).to_string()
                    };
                    (200, json_head(), reply)
                }
                Err(e) => {
                    log.error("web::session_action", &format!("会话动作 {} 失败：{}", action, e));
                    (400, json_head(), json!({ "error": e }).to_string())
                }
            }
        }

        // ---- 供应商 ----
        ("POST", "/api/providers") => {
            let req = match parse_body(body) {
                Ok(v) => v,
                Err(e) => return (400, json_head(), json!({ "error": e }).to_string()),
            };
            let mut c = core.lock().expect("core 锁");
            match c.provider_upsert(&str_field(&req, "id"), &str_field(&req, "base_url"), &str_field(&req, "api_key")) {
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
            let id = url_decode(id);
            let mut c = core.lock().expect("core 锁");
            let outcome = match action {
                "remove" => c.provider_remove(&id).map(|ok| json!({ "ok": ok })),
                "discover" => c.discover_models(&id).map(|models| json!({ "ok": true, "models": models })),
                _ => Err(format!("未知动作：{}", action)),
            };
            match outcome {
                Ok(v) => (200, json_head(), v.to_string()),
                Err(e) => (400, json_head(), json!({ "error": e }).to_string()),
            }
        }

        // ---- 模型 ----
        ("POST", "/api/models") => {
            let req = match parse_body(body) {
                Ok(v) => v,
                Err(e) => return (400, json_head(), json!({ "error": e }).to_string()),
            };
            let mut c = core.lock().expect("core 锁");
            match c.model_upsert(
                &str_field(&req, "id"),
                &str_field(&req, "name"),
                &str_field(&req, "api_model"),
                &str_field(&req, "provider"),
                &str_field(&req, "note"),
            ) {
                Ok(()) => (200, json_head(), json!({ "ok": true }).to_string()),
                Err(e) => (400, json_head(), json!({ "error": e }).to_string()),
            }
        }
        ("POST", path) if path.starts_with("/api/models/") => {
            let rest = &path["/api/models/".len()..];
            let (id, action) = match rest.split_once('/') {
                Some(x) => x,
                None => return (404, json_head(), json!({ "error": "路径应为 /api/models/{id}/{action}" }).to_string()),
            };
            let id = url_decode(id);
            let mut c = core.lock().expect("core 锁");
            let outcome = match action {
                "remove" => c.model_remove(&id).map(|ok| json!({ "ok": ok })),
                "core" => c.core_set_model(&id).map(|ok| json!({ "ok": ok })),
                _ => Err(format!("未知动作：{}", action)),
            };
            match outcome {
                Ok(v) => (200, json_head(), v.to_string()),
                Err(e) => (400, json_head(), json!({ "error": e }).to_string()),
            }
        }

        // ---- agent ----
        ("POST", "/api/agents") => {
            let req = match parse_body(body) {
                Ok(v) => v,
                Err(e) => return (400, json_head(), json!({ "error": e }).to_string()),
            };
            let mut c = core.lock().expect("core 锁");
            match c.agent_upsert(
                &str_field(&req, "name"),
                &str_list(&req, "modules"),
                &str_field(&req, "model"),
                &str_field(&req, "note"),
            ) {
                Ok(()) => (200, json_head(), json!({ "ok": true }).to_string()),
                Err(e) => (400, json_head(), json!({ "error": e }).to_string()),
            }
        }
        ("POST", path) if path.starts_with("/api/agents/") => {
            let rest = &path["/api/agents/".len()..];
            let (name, action) = match rest.split_once('/') {
                Some(x) => x,
                None => return (404, json_head(), json!({ "error": "路径应为 /api/agents/{name}/{action}" }).to_string()),
            };
            let name = url_decode(name);
            let mut c = core.lock().expect("core 锁");
            let outcome = match action {
                "remove" => c.agent_remove(&name).map(|ok| json!({ "ok": ok })),
                _ => Err(format!("未知动作：{}", action)),
            };
            match outcome {
                Ok(v) => (200, json_head(), v.to_string()),
                Err(e) => (400, json_head(), json!({ "error": e }).to_string()),
            }
        }

        // ---- 基本设置 ----
        ("GET", "/api/settings") => {
            let c = core.lock().expect("core 锁");
            (200, json_head(), json!({ "settings": c.app_settings() }).to_string())
        }
        ("POST", "/api/settings") => {
            let req = match serde_json::from_str::<serde_json::Value>(body) {
                Ok(v) => v,
                Err(e) => return (400, json_head(), json!({ "error": format!("请求不是 JSON：{}", e) }).to_string()),
            };
            let current = core.lock().expect("core 锁").app_settings();
            let settings = AppSettings {
                streaming: req.get("streaming").and_then(|v| v.as_bool()).unwrap_or(current.streaming),
                show_reasoning: req.get("show_reasoning").and_then(|v| v.as_bool()).unwrap_or(current.show_reasoning),
                // 执行档位与围栏写权限：界面暂未暴露（后续阶段），改设置只保留现有值。
                tier: current.tier,
                fence_write: current.fence_write,
            };
            let mut c = core.lock().expect("core 锁");
            match c.set_app_settings(settings) {
                Ok(()) => (200, json_head(), json!({ "ok": true }).to_string()),
                Err(e) => (400, json_head(), json!({ "error": e }).to_string()),
            }
        }

        // ---- 会话历史 ----
        ("GET", "/api/history") => {
            let c = core.lock().expect("core 锁");
            (200, json_head(), json!({ "sessions": c.history_list() }).to_string())
        }
        ("GET", path) if path.starts_with("/api/history/") => {
            let name = url_decode(&path["/api/history/".len()..]);
            let c = core.lock().expect("core 锁");
            match c.history_open(&name) {
                Ok((meta, events)) => (200, json_head(), json!({ "meta": meta, "events": events }).to_string()),
                Err(e) => (404, json_head(), json!({ "error": e }).to_string()),
            }
        }
        ("POST", path) if path.starts_with("/api/history/") && path.ends_with("/delete") => {
            let name = url_decode(&path["/api/history/".len()..path.len() - "/delete".len()]);
            let mut c = core.lock().expect("core 锁");
            match c.history_delete(&name) {
                Ok(ok) => (200, json_head(), json!({ "ok": ok }).to_string()),
                Err(e) => (400, json_head(), json!({ "error": e }).to_string()),
            }
        }

        // ---- 会话文件清单 + 真实根（前端 @ 菜单与长路径缩写） ----
        // 配置视图（会话界面之外）：能改什么、现在是什么、缺什么，一次性如实给出。
        ("GET", path) if path.starts_with("/api/sessions/") && path.ends_with("/config") => {
            let sid = url_decode(&path["/api/sessions/".len()..path.len() - "/config".len()]);
            let c = core.lock().expect("core 锁");
            match c.session_config(&sid) {
                Ok(config) => (200, json_head(), json!({ "config": config }).to_string()),
                Err(e) => (400, json_head(), json!({ "error": e }).to_string()),
            }
        }
        ("GET", path) if path.starts_with("/api/sessions/") && path.ends_with("/files") => {
            let sid = url_decode(&path["/api/sessions/".len()..path.len() - "/files".len()]);
            let c = core.lock().expect("core 锁");
            match c.files_view(&sid) {
                Ok(v) => (
                    200,
                    json_head(),
                    json!({ "work": v.work, "agents": v.agents, "roots": v.roots }).to_string(),
                ),
                Err(e) => (404, json_head(), json!({ "error": e }).to_string()),
            }
        }

        // ---- 核心推荐模型 ----
        ("POST", "/api/suggest-models") => {
            let req = match parse_body(body) {
                Ok(v) => v,
                Err(e) => return (400, json_head(), json!({ "error": e }).to_string()),
            };
            let task = str_field(&req, "task");
            let mode = match parse_mode(&str_field(&req, "mode")) {
                Ok(m) => m,
                Err(e) => return (400, json_head(), json!({ "error": e }).to_string()),
            };
            let c = core.lock().expect("core 锁");
            match c.suggest_models(&task, mode) {
                Ok(agents) => (200, json_head(), json!({ "ok": true, "agents": agents }).to_string()),
                Err(e) => (400, json_head(), json!({ "error": e }).to_string()),
            }
        }

        _ => (404, json_head(), json!({ "error": "无此路由" }).to_string()),
    }
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
fn state_json(core: &Core, fence: &FenceInfo) -> serde_json::Value {
    let roster = core.scan();
    // 会话形态取落盘 meta（单一真相）：只读一次盘，sessions 与 history 共用。
    let history = core.history_list();
    json!({
        "modules": roster.modules.iter().map(|m| json!({
            "id": m.manifest.id,
            "brief": m.manifest.brief,
        })).collect::<Vec<_>>(),
        "rejected": roster.rejected,
        // 本机工具围栏的实际能力（如实显示，不假装）：哪些维度真的被强制了。
        "fence": fence,
        "providers": core.provider_views(),
        "models": core.model_views(),
        "core": core.core_model(),
        "agents": core.agent_views(),
        "settings": core.app_settings(),
        "sessions": core.session_views(&history),
        "history": history,
    })
}

/// 事件 → JSON（线格式唯一定义在 core::events::SessionEvent::to_json）。
fn ev_json(events: &[SessionEvent]) -> serde_json::Value {
    serde_json::Value::Array(events.iter().map(|e| e.to_json()).collect())
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
    /// (全局序号, 会话 id, 事件数组)
    lines: Vec<(u64, String, serde_json::Value)>,
}

impl Outbox {
    /// 入箱并返回该批事件的全局序号（客户端据此去重）。
    fn push(&mut self, sid: &str, events: serde_json::Value) -> u64 {
        self.seq += 1;
        self.lines.push((self.seq, sid.to_string(), events));
        // 上限保护：本地单机足够，防无界增长。
        if self.lines.len() > 10_000 {
            let drop = self.lines.len() - 5_000;
            self.lines.drain(..drop);
        }
        self.seq
    }
    /// 取某会话（None=全部）since 之后的事件批（带序号，供客户端去重）。
    fn snapshot(&self, sid: Option<&str>, since: u64) -> Vec<serde_json::Value> {
        self.lines
            .iter()
            .filter(|(sq, s, _)| *sq > since && sid.map_or(true, |x| s.as_str() == x))
            .map(|(sq, s, ev)| json!({ "seq": sq, "sid": s, "events": ev }))
            .collect()
    }
    /// 取全局最新序号（客户端首轮同步用）。
    fn head(&self) -> u64 {
        self.seq
    }
}
