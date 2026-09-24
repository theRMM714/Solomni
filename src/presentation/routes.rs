//! HTTP 入站契约的**唯一定义**：路由目录 + 匹配器。
//!
//! web.rs 的分发由这份目录驱动，所以「代码里有路由但目录里没有」在结构上不可能发生；
//! 反过来「目录里有路由但没有处理器」由契约测试逐条点名叫出来（见 contract_tests/routes.rs）。
//! 文档表（docs/architecture/contracts.md 的 ROUTES 段落）也由契约测试与这里比对，杜绝文档过期。

use serde_json::json;

/// 一条路由。`id` 是分发键：web.rs 只按它分发，不再自己写 (method, path) 匹配。
pub struct Route {
    pub id: &'static str,
    pub method: &'static str,
    /// 路径模式；`{name}` 段是参数（匹配时按段取值）。
    pub pattern: &'static str,
    /// 它落到哪个入站能力（文档与覆盖检查用）。
    pub capability: &'static str,
    pub request: &'static str,
    pub response: &'static str,
    pub statuses: &'static [u16],
    pub note: &'static str,
}

/// 全部 HTTP 路由。新增一条 = 在这里加一行 + 在 web.rs 加一个分支（契约测试会盯着两者）。
pub const ROUTES: &[Route] = &[
    Route {
        id: "index",
        method: "GET",
        pattern: "/",
        capability: "静态资源",
        request: "—",
        response: "index.html",
        statuses: &[200],
        note: "界面壳（no-store，改完立刻生效）",
    },
    Route {
        id: "style",
        method: "GET",
        pattern: "/style.css",
        capability: "静态资源",
        request: "—",
        response: "style.css",
        statuses: &[200],
        note: "样式",
    },
    Route {
        id: "app",
        method: "GET",
        pattern: "/app.js",
        capability: "静态资源",
        request: "—",
        response: "app.js",
        statuses: &[200],
        note: "前端逻辑",
    },
    Route {
        id: "md",
        method: "GET",
        pattern: "/md.js",
        capability: "静态资源",
        request: "—",
        response: "md.js",
        statuses: &[200],
        note: "Markdown 渲染（先转义再拼标签）",
    },
    Route {
        id: "events",
        method: "GET",
        pattern: "/api/events",
        capability: "事件台（core::api::EventBus）",
        request: "查询 sid / since",
        response: "{lines:[{seq,sid,events}],head,oldest}",
        statuses: &[200],
        note: "长轮询：有新事件立刻回，否则最多等 20s",
    },
    Route {
        id: "state",
        method: "GET",
        pattern: "/api/state",
        capability: "DiscoveryOps + RegistryOps + HistoryOps",
        request: "—",
        response: "{modules,rejected,fence,providers,models,core,agents,settings,sessions,history}",
        statuses: &[200, 400],
        note: "概览状态；一律不含密钥",
    },
    Route {
        id: "session.new",
        method: "POST",
        pattern: "/api/sessions",
        capability: "SessionOps::create_work",
        request: "{name,mode,agents[],task?,delegate?}",
        response: "{sid,agents,events}",
        statuses: &[200, 400],
        note: "创建工作（形态与名单由用户给定）",
    },
    Route {
        id: "session.act",
        method: "POST",
        pattern: "/api/sessions/{sid}/{action}",
        capability: "SessionOps + intent::act",
        request: "{text?,agent?,id?,overwrite?,data_base64?,编辑体}",
        response: "{sid,events,seq} / {sid,events} / {ok} / {sid,pending}",
        statuses: &[200, 400, 404, 409],
        note: "动作：say / task / slate / begin / answer / approve-plan / continue / withdraw / stop / rewind / update-task / edit / upload / pending",
    },
    Route {
        id: "session.config",
        method: "GET",
        pattern: "/api/sessions/{sid}/config",
        capability: "SessionOps::config",
        request: "—",
        response: "{config}",
        statuses: &[200, 400],
        note: "配置视图：能改什么、现在是什么、缺什么",
    },
    Route {
        id: "session.files",
        method: "GET",
        pattern: "/api/sessions/{sid}/files",
        capability: "SessionOps::files",
        request: "—",
        response: "{work,agents,roots}",
        statuses: &[200, 404],
        note: "@ 菜单与长路径缩写用（相对清单 + 真实根）",
    },
    Route {
        id: "provider.new",
        method: "POST",
        pattern: "/api/providers",
        capability: "RegistryOps::upsert_provider",
        request: "{id,base_url,api_key}",
        response: "{ok}",
        statuses: &[200, 400],
        note: "api_key 留空 = 保留原密钥（界面从不回显）",
    },
    Route {
        id: "provider.act",
        method: "POST",
        pattern: "/api/providers/{id}/{action}",
        capability: "RegistryOps::remove_provider / discover_models",
        request: "—",
        response: "{ok} / {ok,models}",
        statuses: &[200, 400, 404],
        note: "动作：remove / discover",
    },
    Route {
        id: "model.new",
        method: "POST",
        pattern: "/api/models",
        capability: "RegistryOps::upsert_model",
        request: "{id,name,api_model,provider,note?}",
        response: "{ok}",
        statuses: &[200, 400],
        note: "供应商必须已存在",
    },
    Route {
        id: "model.act",
        method: "POST",
        pattern: "/api/models/{id}/{action}",
        capability: "RegistryOps::remove_model / set_core_model / probe_model_tools / probe_replay_shape",
        request: "—",
        response: "{ok} / {ok,outcome,detail,mode} / {ok,shapes}",
        statuses: &[200, 400, 404],
        note: "动作：remove / core（设为核心默认）/ probe（实测原生工具调用，结论写回登记处）/ probe-replay（实测回放形状，只报事实）",
    },
    Route {
        id: "agent.new",
        method: "POST",
        pattern: "/api/agents",
        capability: "RegistryOps::upsert_agent",
        request: "{name,modules[],model?,note?}",
        response: "{ok}",
        statuses: &[200, 400],
        note: "agent 名会成为沙箱目录名（校验更严）",
    },
    Route {
        id: "agent.act",
        method: "POST",
        pattern: "/api/agents/{name}/{action}",
        capability: "RegistryOps::remove_agent",
        request: "—",
        response: "{ok}",
        statuses: &[200, 400, 404],
        note: "动作：remove",
    },
    Route {
        id: "settings.get",
        method: "GET",
        pattern: "/api/settings",
        capability: "RegistryOps::settings",
        request: "—",
        response: "{settings}",
        statuses: &[200, 400],
        note: "基本设置",
    },
    Route {
        id: "settings.set",
        method: "POST",
        pattern: "/api/settings",
        capability: "RegistryOps::set_settings",
        request: "{streaming?,show_reasoning?}",
        response: "{ok}",
        statuses: &[200, 400],
        note: "未提交的字段保留现值（部分更新）",
    },
    Route {
        id: "history.list",
        method: "GET",
        pattern: "/api/history",
        capability: "HistoryOps::list",
        request: "—",
        response: "{sessions}",
        statuses: &[200, 400],
        note: "落盘会话清单",
    },
    Route {
        id: "history.open",
        method: "GET",
        pattern: "/api/history/{name}",
        capability: "HistoryOps::open",
        request: "—",
        response: "{meta,events}",
        statuses: &[200, 404],
        note: "读回历史（转录即内容）",
    },
    Route {
        id: "history.delete",
        method: "POST",
        pattern: "/api/history/{name}/delete",
        capability: "HistoryOps::delete",
        request: "—",
        response: "{ok}",
        statuses: &[200, 400],
        note: "删除会话时核心会请求撤销该会话的围栏授权",
    },
    Route {
        id: "suggest",
        method: "POST",
        pattern: "/api/suggest-models",
        capability: "DiscoveryOps::suggest_models",
        request: "{task,mode}",
        response: "{ok,agents}",
        statuses: &[200, 400],
        note: "核心推荐 agent 草案（带理由；用户可改）",
    },
];

/// 命中结果：路由 + 路径参数（按 pattern 里出现的名字取值；已百分号解码）。
pub struct Matched {
    pub route: &'static Route,
    pub params: Vec<(&'static str, String)>,
}

impl Matched {
    /// 取一个路径参数；目录保证只出现 pattern 里声明过的名字。
    pub fn param(&self, name: &str) -> String {
        self.params
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    }
}

/// 按目录匹配一次请求（方法 + 路径；查询串不参与匹配）。
pub fn matches(method: &str, url: &str) -> Option<Matched> {
    let path = url.split('?').next().unwrap_or("");
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    for route in ROUTES {
        if route.method != method {
            continue;
        }
        let pat: Vec<&str> = route.pattern.split('/').filter(|s| !s.is_empty()).collect();
        if pat.len() != segs.len() {
            continue;
        }
        let mut params: Vec<(&'static str, String)> = Vec::new();
        let mut hit = true;
        for (p, s) in pat.iter().zip(segs.iter()) {
            if let Some(name) = p.strip_prefix('{').and_then(|x| x.strip_suffix('}')) {
                // p 取自 route.pattern（'static），所以参数名同样是 'static。
                params.push((name, url_decode(s)));
            } else if p != s {
                hit = false;
                break;
            }
        }
        if hit {
            return Some(Matched { route, params });
        }
    }
    None
}

/// 路径段百分号解码（%XX；路径里没有 '+' 语义）。前端用 encodeURIComponent，故必须解回来。
pub fn url_decode(s: &str) -> String {
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

/// 目录的 JSON 形态（`--print-routes` 与契约测试共用同一份事实）。
pub fn catalog_json() -> serde_json::Value {
    json!(ROUTES
        .iter()
        .map(|r| json!({
            "id": r.id,
            "method": r.method,
            "pattern": r.pattern,
            "capability": r.capability,
            "request": r.request,
            "response": r.response,
            "statuses": r.statuses,
            "note": r.note,
        }))
        .collect::<Vec<_>>())
}
