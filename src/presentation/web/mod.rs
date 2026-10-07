//! Web 转录中心：tiny_http 服务器 + 长轮询增量事件推送。
//! 只做协议适配（HTTP/长轮询与**入站能力面**之间的转译）；渲染在浏览器（app.js）。
//! 路由由 `presentation/routes.rs` 的目录驱动：匹配不到就 404，目录里有而这里没分支 = 程序缺陷。
//! 核心状态在核心自己的线程上：这里拿不到它、也拿不到任何核心锁，「停止」直接说给核心听。
//! 安全底线：只绑 127.0.0.1；密钥永不进任何响应（能力面只给 id）。

use crate::capabilities::conductor::api::{Acted, ActionCall, Caller, SessionEvent, WorkMode};
use crate::capabilities::conductor::api::{Ops, Output};
use crate::capabilities::registry::api::AppSettings;
pub mod routes;
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

/// 启动转录中心服务器：**可中断**——收到 Ctrl+C / SIGINT 就正常关闭回到 CLI（进程仍在）。
/// 端口可指定，默认 3081，只绑本机回环。
///
/// 处理：进入前安装中断标志（Windows 控制台处理函数 / Unix SIGINT），返回前**恢复默认**，
/// 所以回到 CLI 提示符后 Ctrl+C 仍按"退出产品"的既有语义生效。
pub fn serve(ops: Ops, port: u16, fence: FenceInfo) -> Result<(), String> {
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let guard = interrupt::install(Arc::clone(&stop))?;
    let r = serve_until(&ops, port, &fence, &stop);
    drop(guard);
    r
}

/// 服务器主循环（可注入中断判据，便于测试）：每次最多等 200ms 就回来看一次标志。
/// 每个请求仍是一线独立线程——长连接绝不阻塞其它请求（转录中心是多端并用的）。
pub(crate) fn serve_until(
    ops: &Ops,
    port: u16,
    fence: &FenceInfo,
    stop: &std::sync::atomic::AtomicBool,
) -> Result<(), String> {
    // 日志经**入站能力面**（ops.log）取——呈现层不持有端口对象。
    let log = Arc::clone(&ops.log);
    log.info("web::serve", &format!("转录中心启动，端口 {}", port));
    let addr = format!("127.0.0.1:{}", port);
    let server = Server::http(addr.as_str()).map_err(|e| e.to_string())?;
    println!("Solomni 转录中心：http://{}（只监听本机）", addr);
    let fence = Arc::new(fence.clone());
    while !stop.load(std::sync::atomic::Ordering::SeqCst) {
        let request = match server.recv_timeout(Duration::from_millis(200)) {
            Ok(Some(r)) => r,
            Ok(None) => continue,
            Err(e) => return Err(e.to_string()),
        };
        let ops = ops.clone();
        let fence = Arc::clone(&fence);
        let log = Arc::clone(&log);
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
    println!("[提示] 已离开转录中心，回到终端。");
    Ok(())
}

/// Ctrl+C / SIGINT 的中断标志安装：机制按平台分，返回的守卫一放就恢复默认。
mod interrupt {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, OnceLock};

    static FLAG: OnceLock<Arc<AtomicBool>> = OnceLock::new();

    fn set() {
        if let Some(f) = FLAG.get() {
            f.store(true, Ordering::SeqCst);
        }
    }

    #[cfg(windows)]
    mod imp {
        use super::{set, Arc, AtomicBool};
        use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;

        unsafe extern "system" fn handler(_ctrl: u32) -> i32 {
            set();
            1 // TRUE = 已处理，进程继续（不按默认终止）
        }

        pub struct Guard;

        pub fn install(stop: Arc<AtomicBool>) -> Result<Guard, String> {
            let _ = super::FLAG.set(stop);
            let ok = unsafe { SetConsoleCtrlHandler(Some(handler), 1) };
            if ok == 0 {
                return Err("安装 Ctrl+C 处理失败".to_string());
            }
            Ok(Guard)
        }

        impl Drop for Guard {
            fn drop(&mut self) {
                unsafe {
                    SetConsoleCtrlHandler(Some(handler), 0);
                }
            }
        }
    }

    #[cfg(unix)]
    mod imp {
        use super::{set, Arc, AtomicBool};

        extern "C" fn handler(_sig: libc::c_int) {
            set();
        }

        pub struct Guard;

        pub fn install(stop: Arc<AtomicBool>) -> Result<Guard, String> {
            let _ = super::FLAG.set(stop);
            // 函数项先转指针再转整数：直接转整数会被 clippy 判为 function_casts_as_integer。
            let handler_ptr = handler as *const () as libc::sighandler_t;
            let prev = unsafe { libc::signal(libc::SIGINT, handler_ptr) };
            if prev == libc::SIG_ERR {
                return Err("安装 Ctrl+C 处理失败".to_string());
            }
            Ok(Guard)
        }

        impl Drop for Guard {
            fn drop(&mut self) {
                unsafe {
                    libc::signal(libc::SIGINT, libc::SIG_DFL);
                }
            }
        }
    }

    #[cfg(not(any(windows, unix)))]
    mod imp {
        use super::{Arc, AtomicBool};

        pub struct Guard;

        pub fn install(_stop: Arc<AtomicBool>) -> Result<Guard, String> {
            Ok(Guard)
        }
    }

    pub use imp::install;
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
    log: &Arc<dyn crate::capabilities::conductor::api::LogOps + Send + Sync>,
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
            include_str!("assets/index.html").to_string(),
        ),
        "style" => (
            200,
            static_head("text/css"),
            include_str!("assets/style.css").to_string(),
        ),
        "app" => (
            200,
            static_head("application/javascript"),
            include_str!("assets/app.js").to_string(),
        ),
        "md" => (
            200,
            static_head("application/javascript"),
            include_str!("assets/md.js").to_string(),
        ),

        "events" => {
            // 长轮询：?sid= 会话，&since= 客户端已见序号。最多等 20s；有新事件立即回。
            let q = url.split('?').nth(1).unwrap_or("");
            let mut sid: Option<String> = None;
            let mut since = 0u64;
            for pair in q.split('&') {
                let mut kv = pair.splitn(2, '=');
                match (kv.next(), kv.next()) {
                    // 查询参数和路径段一样是百分号编码的：会话名常带中文（<工作>--<agent>），
                    // 不解回来就永远匹配不到任何事件（真机 sid 就是中文）。
                    (Some("sid"), Some(v)) if !v.is_empty() => sid = Some(routes::url_decode(v)),
                    (Some("since"), v) => {
                        since = v.and_then(|x| x.parse::<u64>().ok()).unwrap_or(0)
                    }
                    _ => {}
                }
            }
            let deadline = Instant::now() + POLL_WAIT;
            loop {
                // 同一把锁里取「批 + 头部」：客户端据头部推进游标不会漏事件。
                let (lines, head, oldest) = ops.events.snapshot(sid.as_deref(), since);
                if !lines.is_empty() || Instant::now() >= deadline {
                    let snap: Vec<serde_json::Value> = lines
                        .iter()
                        .map(
                            |l| json!({ "seq": l.seq, "sid": l.sid, "events": ev_json(&l.events) }),
                        )
                        .collect();
                    // oldest = 事件台里还留着的最老序号：客户端发现"since 之后那段已裁掉"时据此重新对齐。
                    return ok_json(json!({ "lines": snap, "head": head, "oldest": oldest }));
                }
                std::thread::sleep(POLL_TICK);
            }
        }

        "state" => match state_json(ops, fence) {
            Ok(v) => ok_json(v),
            Err(e) => complaint(400, e),
        },

        // ---- 动作目录与动作分发（唯一路径：声明在 systools/tools.yaml） ----
        // 目录：这个调用者此刻能做什么（含可用性）。前端据此渲染，不写第二份动作清单。
        "actions" => {
            let sid = url
                .split('?')
                .nth(1)
                .unwrap_or("")
                .split('&')
                .find_map(|kv| kv.strip_prefix("sid="))
                .map(routes::url_decode)
                .filter(|s| !s.is_empty());
            match ops.actions.catalog(&Caller::User, sid.as_deref()) {
                Ok(list) => ok_json(json!({ "actions": list })),
                Err(e) => complaint(400, e),
            }
        }
        // 分发：一次动作 = 参数按声明校验 + callers 授权 + 执行 + 审计（各只有一处）。
        "action" => {
            let req = match parse_body(body) {
                Ok(v) => v,
                Err(e) => return complaint(400, e),
            };
            // 流式与否是**显示**的选择，归呈现层（取自设置）。
            let out = match ops.registry.settings() {
                Ok(s) if s.streaming => Output::Stream,
                _ => Output::Final,
            };
            let call = ActionCall {
                id: id.clone(),
                args: req,
                caller: Caller::User,
                out,
            };
            match ops.actions.act(call) {
                // 生成类只回**事件台头部序号**：事实由长轮询按 since 订阅。
                Ok(Acted::Advanced(adv)) => ok_json(json!({ "head": adv.head })),
                // 回档 / 改需求返回完整重放（前端整体重建）。
                Ok(Acted::Replayed(events)) => ok_json(json!({ "events": events })),
                // 其余给一份结构化结果（{ok:true} / {sid,agents} …）。
                Ok(Acted::Done(v)) => ok_json(v),
                Err(e) => {
                    log.error("web::action", &format!("动作 {} 失败：{}", id, e));
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
            Ok(v) => ok_json(
                json!({ "work": v.work, "agents": v.agents, "roots": v.roots, "usage": v.usage }),
            ),
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
                // 探测要真实网络（两条最小请求），结论由 conductor 按三种如实回报并只写确定的结论。
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
                // 会话权限的全局默认：界面暂未暴露，改设置保留现值（改 yaml 或后续界面）。
                permissions: current.permissions.clone(),
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
        // 历史与实时**一次给全**：盘上转录 + 事件台上**它之外**的实时尾巴 + 合流时的头部序号。
        // 前端因此只有一条带序号的流（只按序 append），不靠自己合并两个来源——
        // 刷新后整段重复（任务行 / [建议] / yes 各两遍）正是在那个合并里出的。
        "history.open" => match ops.history.open(&name) {
            Ok((meta, events)) => {
                let (live, head) = ops.events.tail_excluding(&name, &events);
                ok_json(json!({ "meta": meta, "events": events, "live": live, "head": head }))
            }
            Err(e) => complaint(404, e),
        },
        "history.delete" => match ops.history.delete(&name) {
            Ok(ok) => ok_json(json!({ "ok": ok })),
            Err(e) => complaint(400, e),
        },

        // ---- 新建工作的档位选择（创建向导用） ----
        "tiers" => match ops.core.tier_choices() {
            Ok(v) => ok_json(json!({ "tiers": v })),
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
            match ops.core.suggest_models(&task, mode) {
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
    outcome: &crate::capabilities::conductor::api::ProbeOutcome,
) -> serde_json::Value {
    use crate::capabilities::conductor::api::ProbeOutcome;
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
        // 代理形态：没有名单，用户选它就是**授予全权**（见 docs/conductor/README.md）。
        "proxy" => Ok(WorkMode::Proxy),
        other => Err(format!(
            "未知模式：{}（只接受 single / collab / proxy）",
            other
        )),
    }
}

/// 概览状态：模块清单 + 供应商/模型视图 + 核心默认 + 进行中会话（均无密钥）。
fn state_json(ops: &Ops, fence: &FenceInfo) -> Result<serde_json::Value, String> {
    let roster = ops.workspace.roster()?;
    // 会话形态取落盘 meta（单一真相）：只读一次盘，sessions 与 history 共用。
    let history = ops.history.list()?;
    // 会话快照带上"在等的工具确认"：刷新页面后界面照样画得出那张"是 / 否 / 本轮不再问"的卡
    // （与推的 `Decision{kind:"tool_approval"}` 同一份事实）。
    let sessions: Vec<serde_json::Value> = ops
        .sessions
        .session_views(&history)?
        .into_iter()
        .map(|v| {
            let mut j = serde_json::to_value(&v).unwrap_or_else(|_| json!({}));
            if let Some(a) = ops.sessions.pending_approval(&v.sid) {
                let name = match &a.module {
                    Some(m) => format!("{}.{}", m, a.tool),
                    None => a.tool.clone(),
                };
                j["pending"] = json!({
                    "kind": "tool_approval",
                    "summary": format!("agent 请求执行工具 {}。", name),
                    "advice": "",
                    "question": format!("是否执行 {}？（yes / no / full：full = 本轮不再问）", name),
                    "payload": { "tool": a.tool, "module": a.module, "args": a.args },
                });
            }
            j
        })
        .collect();
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
        "sessions": sessions,
        "history": history,
    }))
}

/// 事件 → JSON（线格式唯一定义在 session 的 SessionEvent::to_json）。
fn ev_json(events: &[SessionEvent]) -> serde_json::Value {
    serde_json::Value::Array(events.iter().map(|e| e.to_json()).collect())
}
