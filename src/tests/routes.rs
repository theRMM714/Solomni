//! HTTP 入站契约的契约测试：路由目录 ↔ 处理器 ↔ 文档 ↔ 前端调用，四者机器比对。
//! 用**假能力面**（FakeOps）直接调 `web::route`（不经过 socket），逐条路由验成功/错误/空/边界；
//! 真实传输由 L4 端到端覆盖（真二进制 + 真 HTTP）。
//! 假能力面顺带证明一件事：「按角色切分」的能力接口真能被替换——新增一种呈现不必认识 `Core`。

use crate::core::agents::AgentView;
use crate::core::api::{
    Advance, DiscoveryOps, EventBus, HistoryOps, Ops, Output, RegistryOps, SessionOps,
};
use crate::core::exec::Tier;
use crate::core::history::{AgentMeta, HistoryView, SessionMeta};
use crate::core::module::Roster;
use crate::core::ports::NoopLog;
use crate::core::providers::{AppSettings, ModelView, ProviderView};
use crate::core::{
    AgentSuggestion, ConfigAgent, FilesAgentView, FilesRootsView, FilesView, Pending,
    RuntimeReport, SessionConfig, SessionEdit, SessionView, WorkMode, WorkOpened, WorkSpec,
};
use crate::presentation::routes::{self, ROUTES};
use crate::presentation::web::{self, FenceInfo};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// ---------- 假能力面 ----------

/// 假能力面：不碰核心、不起线程。`fail` 一注入，所有能力一律失败（用来验错误路径）。
#[derive(Default)]
struct FakeOps {
    fail: Option<String>,
    running: AtomicBool,
    /// 探测结论可换（缺省"支持"）：用来验三种结论都**原样**穿过呈现层、不被改写。
    probe: Option<crate::core::ports::ProbeOutcome>,
}

impl FakeOps {
    fn new(fail: Option<&str>) -> FakeOps {
        FakeOps {
            fail: fail.map(|s| s.to_string()),
            running: AtomicBool::new(false),
            probe: None,
        }
    }

    fn guard(&self) -> Result<(), String> {
        match &self.fail {
            Some(m) => Err(m.clone()),
            None => Ok(()),
        }
    }
}

fn fake_ops_probe(fail: Option<&str>, probe: crate::core::ports::ProbeOutcome) -> Ops {
    let mut f = FakeOps::new(fail);
    f.probe = Some(probe);
    let f = Arc::new(f);
    Ops {
        sessions: f.clone(),
        registry: f.clone(),
        history: f.clone(),
        discovery: f.clone(),
        events: EventBus::new(),
    }
}

fn fake_ops(fail: Option<&str>) -> Ops {
    let f = Arc::new(FakeOps::new(fail));
    Ops {
        sessions: f.clone(),
        registry: f.clone(),
        history: f.clone(),
        discovery: f.clone(),
        events: EventBus::new(),
    }
}

fn report() -> RuntimeReport {
    RuntimeReport {
        tier: "host".to_string(),
        declared: BTreeMap::new(),
        available: BTreeMap::new(),
        missing: BTreeMap::new(),
        diagnoses: Vec::new(),
        tier_ready: true,
        tier_missing: Vec::new(),
        rejected: Vec::new(),
        rejected_packages: Vec::new(),
    }
}

fn meta(name: &str) -> SessionMeta {
    SessionMeta {
        name: name.to_string(),
        mode: "direct".to_string(),
        delegate: false,
        modules: vec!["m".to_string()],
        task: None,
        ts: 1,
        agents: Vec::new(),
        exec: crate::core::exec::ExecSpec::default(),
        parent: None,
        node: None,
    }
}

impl SessionOps for FakeOps {
    fn create_work(&self, _spec: WorkSpec) -> Result<WorkOpened, String> {
        self.guard()?;
        Ok(WorkOpened {
            sid: "w1".to_string(),
            agents: vec!["甲".to_string()],
            events: vec![crate::core::SessionEvent::Notice("开好了".to_string())],
        })
    }
    fn say(&self, _sid: &str, _text: &str, _out: Output) -> Result<Advance, String> {
        self.guard()?;
        Ok(Advance {
            events: vec![crate::core::SessionEvent::Notice("推进了".to_string())],
            seq: 7,
        })
    }
    fn continue_flow(&self, _sid: &str, _out: Output) -> Result<Advance, String> {
        self.guard()?;
        Ok(Advance {
            events: Vec::new(),
            seq: 8,
        })
    }
    fn collab_step(
        &self,
        _sid: &str,
        _step: crate::core::CollabStep,
        _text: &str,
    ) -> Result<Advance, String> {
        self.guard()?;
        Ok(Advance {
            events: Vec::new(),
            seq: 9,
        })
    }
    fn withdraw_agree(&self, _sid: &str, _agent: &str) -> Result<Advance, String> {
        self.guard()?;
        Ok(Advance {
            events: Vec::new(),
            seq: 10,
        })
    }
    fn slate(&self, _sid: &str) -> Result<Vec<AgentMeta>, String> {
        self.guard()?;
        Ok(Vec::new())
    }
    fn rewind(&self, _sid: &str, _keep_id: u64) -> Result<Vec<serde_json::Value>, String> {
        self.guard()?;
        Ok(vec![json!({ "type": "line", "line": "重放" })])
    }
    fn update_task(&self, _sid: &str, _text: &str) -> Result<Vec<serde_json::Value>, String> {
        self.guard()?;
        Ok(vec![json!({ "type": "line", "line": "重放" })])
    }
    fn pending(&self, _sid: &str) -> Result<Option<Pending>, String> {
        self.guard()?;
        Ok(Some(Pending::ConfirmBegin))
    }
    fn config(&self, _sid: &str) -> Result<SessionConfig, String> {
        self.guard()?;
        Ok(SessionConfig {
            sid: "w1".to_string(),
            mode: "direct".to_string(),
            started: true,
            agents: vec![ConfigAgent {
                name: "甲".to_string(),
                modules: vec!["m".to_string()],
                model: String::new(),
            }],
            tier: "host".to_string(),
            base: None,
            net: false,
            pins: BTreeMap::new(),
            tier_ready: true,
            tier_missing: Vec::new(),
            vm_available: true,
            vm_unavailable_reason: String::new(),
            vm_requirements: Vec::new(),
            runtime: report(),
            runtimes_dir: "runtimes".to_string(),
        })
    }
    fn edit(&self, _sid: &str, _edit: SessionEdit) -> Result<(), String> {
        self.guard()
    }
    fn upload(
        &self,
        _sid: &str,
        _name: &str,
        _bytes: &[u8],
        overwrite: bool,
    ) -> Result<bool, String> {
        self.guard()?;
        Ok(overwrite)
    }
    fn files(&self, _sid: &str) -> Result<FilesView, String> {
        self.guard()?;
        Ok(FilesView {
            work: vec!["a.txt".to_string()],
            agents: vec![FilesAgentView {
                name: "甲".to_string(),
                files: Vec::new(),
            }],
            roots: FilesRootsView {
                work: "/w/work".to_string(),
                agents: Vec::new(),
            },
        })
    }
    fn exists(&self, _sid: &str) -> Result<bool, String> {
        self.guard()?;
        Ok(true)
    }
    fn stop(&self, _sid: &str) -> bool {
        self.running.swap(false, Ordering::Relaxed)
    }
    fn is_running(&self, _sid: &str) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

impl RegistryOps for FakeOps {
    fn providers(&self) -> Result<Vec<ProviderView>, String> {
        self.guard()?;
        Ok(vec![ProviderView {
            id: "p1".to_string(),
            base_url: "http://x".to_string(),
        }])
    }
    fn upsert_provider(&self, _id: &str, _base_url: &str, _api_key: &str) -> Result<(), String> {
        self.guard()
    }
    fn remove_provider(&self, _id: &str) -> Result<bool, String> {
        self.guard()?;
        Ok(true)
    }
    fn models(&self) -> Result<Vec<ModelView>, String> {
        self.guard()?;
        Ok(vec![ModelView {
            id: "m1".to_string(),
            name: "M".to_string(),
            api_model: "m".to_string(),
            provider: "p1".to_string(),
            note: String::new(),
            tools: crate::core::providers::ToolMode::Envelope,
            is_core: true,
        }])
    }
    fn core_model(&self) -> Result<Option<String>, String> {
        self.guard()?;
        Ok(Some("m1".to_string()))
    }
    fn upsert_model(
        &self,
        _id: &str,
        _name: &str,
        _api_model: &str,
        _provider: &str,
        _note: &str,
    ) -> Result<(), String> {
        self.guard()
    }
    fn remove_model(&self, _id: &str) -> Result<bool, String> {
        self.guard()?;
        Ok(true)
    }
    fn set_core_model(&self, _id: &str) -> Result<bool, String> {
        self.guard()?;
        Ok(true)
    }
    fn agents(&self) -> Result<Vec<AgentView>, String> {
        self.guard()?;
        Ok(vec![AgentView {
            name: "甲".to_string(),
            modules: vec!["m".to_string()],
            model: None,
            note: String::new(),
        }])
    }
    fn upsert_agent(
        &self,
        _name: &str,
        _modules: &[String],
        _model: &str,
        _note: &str,
    ) -> Result<(), String> {
        self.guard()
    }
    fn remove_agent(&self, _name: &str) -> Result<bool, String> {
        self.guard()?;
        Ok(true)
    }
    fn settings(&self) -> Result<AppSettings, String> {
        self.guard()?;
        Ok(AppSettings::default())
    }
    fn set_settings(&self, _app: AppSettings) -> Result<(), String> {
        self.guard()
    }
    fn discover_models(&self, _provider_id: &str) -> Result<Vec<String>, String> {
        self.guard()?;
        Ok(vec!["m1".to_string(), "m2".to_string()])
    }
    fn probe_replay_shape(
        &self,
        _id: &str,
    ) -> Result<crate::core::providers::ReplayReport, String> {
        self.guard()?;
        Ok(crate::core::providers::ReplayReport {
            shapes: vec![
                crate::core::providers::ReplayShape {
                    name: "baseline-text".to_string(),
                    accepted: true,
                    understood: true,
                    detail: "finish_reason=stop".to_string(),
                },
                crate::core::providers::ReplayShape {
                    name: "content-empty".to_string(),
                    accepted: false,
                    understood: false,
                    detail: "供应商原话：content is required".to_string(),
                },
            ],
        })
    }
    fn probe_model_tools(&self, _id: &str) -> Result<crate::core::ports::ProbeOutcome, String> {
        self.guard()?;
        Ok(self
            .probe
            .clone()
            .unwrap_or(crate::core::ports::ProbeOutcome::Supported {
                detail: "替身说支持".to_string(),
            }))
    }
}

impl HistoryOps for FakeOps {
    fn list(&self) -> Result<Vec<HistoryView>, String> {
        self.guard()?;
        Ok(vec![HistoryView {
            name: "w1".to_string(),
            mode: "direct".to_string(),
            ts: 1,
            done: false,
            exec: Default::default(),
            parent: None,
        }])
    }
    fn open(&self, name: &str) -> Result<(SessionMeta, Vec<serde_json::Value>), String> {
        self.guard()?;
        Ok((meta(name), vec![json!({ "type": "line" })]))
    }
    fn delete(&self, _name: &str) -> Result<bool, String> {
        self.guard()?;
        Ok(true)
    }
    fn session_views(&self, history: &[HistoryView]) -> Result<Vec<SessionView>, String> {
        self.guard()?;
        Ok(history
            .iter()
            .map(|h| SessionView {
                sid: h.name.clone(),
                mode: h.mode.clone(),
                done: h.done,
                tier: h.exec.tier.as_str().to_string(),
                tier_ready: true,
                tier_missing: Vec::new(),
            })
            .collect())
    }
}

impl DiscoveryOps for FakeOps {
    fn roster(&self) -> Result<Roster, String> {
        self.guard()?;
        Ok(Roster {
            modules: Vec::new(),
            rejected: Vec::new(),
        })
    }
    fn runtime_report(&self, _tier: Tier) -> Result<RuntimeReport, String> {
        self.guard()?;
        Ok(report())
    }
    fn suggest_models(&self, _task: &str, _mode: WorkMode) -> Result<Vec<AgentSuggestion>, String> {
        self.guard()?;
        Ok(vec![AgentSuggestion {
            name: "甲".to_string(),
            modules: vec!["m".to_string()],
            model: "m1".to_string(),
            why: "因为".to_string(),
            reuse: false,
        }])
    }
}

// ---------- 请求合成与调用 ----------

fn fence() -> FenceInfo {
    FenceInfo {
        fs: true,
        net: true,
        tree: true,
        note: "测试".to_string(),
        effective_fs: true,
        effective_net: true,
        write_allowed: true,
        read_only_roots: 0,
    }
}

fn call(ops: &Ops, method: &str, url: &str, body: &str) -> (u16, String) {
    let log: Arc<dyn crate::core::ports::Log + Send + Sync> = Arc::new(NoopLog);
    let (code, _headers, text) = web::route(ops, &fence(), &log, method, url, body);
    (code, text)
}

/// 目录里的参数取样（只为本文件合成请求用）：action 要按路由给出**合法**值，否则先死在未知动作上。
fn sample_param(route_id: &str, name: &str) -> &'static str {
    match name {
        "sid" => "w1",
        "id" => "p1",
        "name" => "a1",
        "action" => match route_id {
            "provider.act" | "model.act" | "agent.act" => "remove",
            _ => "say",
        },
        _ => "x",
    }
}

fn sample_url(route: &routes::Route) -> String {
    let mut path = String::new();
    for seg in route.pattern.split('/').filter(|s| !s.is_empty()) {
        path.push('/');
        match seg.strip_prefix('{').and_then(|x| x.strip_suffix('}')) {
            Some(name) => path.push_str(sample_param(route.id, name)),
            None => path.push_str(seg),
        }
    }
    if path.is_empty() {
        path.push('/');
    }
    if route.id == "events" {
        path.push_str("?sid=&since=0");
    }
    path
}

/// 路径是否落在某条目录 pattern 的形状上（参数段匹配任意非空段；查询串不参与）。
fn shape_matches(pattern: &str, url: &str) -> bool {
    let p: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let path = url.split('?').next().unwrap_or("");
    let q: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    p.len() == q.len()
        && p.iter()
            .zip(q.iter())
            .all(|(a, b)| a.starts_with('{') || a == b)
}

/// 每条路由给一个**合法**请求体，好让错误路径真的落到能力上（而不是先死在解析上）。
fn sample_body(route: &routes::Route) -> &'static str {
    match route.id {
        "session.new" => r#"{"name":"w","mode":"single","agents":[{"name":"a","modules":["m"]}]}"#,
        "session.act" => r#"{"text":"x","id":0}"#,
        "provider.new" => r#"{"id":"p","base_url":"u","api_key":"k"}"#,
        "model.new" => r#"{"id":"m","name":"M","api_model":"m","provider":"p"}"#,
        "agent.new" => r#"{"name":"a","modules":["m"]}"#,
        "suggest" => r#"{"task":"t","mode":"single"}"#,
        _ => "{}",
    }
}

// ---------- 目录 ↔ 处理器 ----------

#[test]
fn every_catalogued_route_has_a_handler() {
    let ops = fake_ops(None);
    // 长轮询要有事件才立刻返回（否则真等 20 秒），播种一条即可。
    ops.events.seed_for_test("w1");
    for route in ROUTES {
        let (code, text) = call(&ops, route.method, &sample_url(route), sample_body(route));
        assert_ne!(
            code, 500,
            "目录里有 {} {} 但 web.rs 没有处理器：{}",
            route.method, route.pattern, text
        );
        assert!(
            !text.contains("路由目录与处理器不一致"),
            "{} {}",
            route.method,
            route.pattern
        );
    }
}

#[test]
fn unknown_route_is_reported_as_404() {
    let ops = fake_ops(None);
    let (code, text) = call(&ops, "GET", "/api/nope", "");
    assert_eq!(code, 404);
    assert!(text.contains("无此路由"), "{}", text);
    // 方法不匹配同样是 404（不是 405 —— 保持既有语义）。
    assert_eq!(call(&ops, "DELETE", "/api/state", "").0, 404);
}

#[test]
fn catalog_json_covers_every_route() {
    // `solomni --print-routes` 输出的就是这份事实，别让它与 ROUTES 脱节。
    let list = routes::catalog_json();
    let arr = list.as_array().expect("目录的 JSON 形态应是数组");
    assert_eq!(arr.len(), ROUTES.len(), "JSON 与 ROUTES 条数不一致");
    for (v, r) in arr.iter().zip(ROUTES.iter()) {
        assert_eq!(v.get("id").and_then(|x| x.as_str()), Some(r.id));
        assert_eq!(v.get("method").and_then(|x| x.as_str()), Some(r.method));
        assert_eq!(v.get("pattern").and_then(|x| x.as_str()), Some(r.pattern));
        assert!(
            v.get("statuses")
                .and_then(|x| x.as_array())
                .is_some_and(|a| !a.is_empty()),
            "{} 缺状态码",
            r.id
        );
    }
}

#[test]
fn catalog_is_free_of_duplicates_and_routes_do_not_overlap() {
    let mut seen = std::collections::HashSet::new();
    for r in ROUTES {
        assert!(
            seen.insert((r.method, r.pattern)),
            "目录里重复的 {} {}",
            r.method,
            r.pattern
        );
        assert!(r.pattern.starts_with('/'), "路径要以 / 开头：{}", r.pattern);
        assert!(
            !r.capability.is_empty() && !r.statuses.is_empty(),
            "{} {} 缺能力或状态码",
            r.method,
            r.pattern
        );
    }
    // 两条路由不该互相匹配（否则分发是概率事件）。
    for a in ROUTES {
        for b in ROUTES {
            if a.id == b.id {
                continue;
            }
            let url = sample_url(a);
            if let Some(m) = routes::matches(a.method, &url) {
                assert_eq!(m.route.id, a.id, "{} 被 {} 抢走了", url, b.pattern);
            }
        }
    }
}

// ---------- 目录 ↔ 文档 ----------

#[test]
fn documented_route_table_matches_the_catalog() {
    // 路由表本体在细则文件里（门户不复述细则正文，见 AGENTS.md「文档分层与同步」）。
    let doc = include_str!("../../docs/architecture/contracts.md");
    let body = doc
        .split_once("<!-- ROUTES:BEGIN -->")
        .and_then(|(_, rest)| rest.split_once("<!-- ROUTES:END -->"))
        .map(|(b, _)| b)
        .expect("docs/architecture/contracts.md 必须有 ROUTES:BEGIN/END 包裹的路由表");
    let mut documented: Vec<(String, String)> = Vec::new();
    for line in body.lines() {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if cells.len() < 4 {
            continue;
        }
        let method = cells[1];
        if !matches!(method, "GET" | "POST") {
            continue; // 表头与分隔行
        }
        documented.push((method.to_string(), cells[2].trim_matches('`').to_string()));
    }
    let mut catalog: Vec<(String, String)> = ROUTES
        .iter()
        .map(|r| (r.method.to_string(), r.pattern.to_string()))
        .collect();
    documented.sort();
    catalog.sort();
    assert_eq!(
        documented, catalog,
        "文档里的路由表与 presentation/routes.rs 的 ROUTES 不一致（改哪边都要改另一边）"
    );
}

// ---------- 目录 ↔ 前端调用 ----------

#[test]
fn frontend_only_calls_catalogued_paths() {
    let src = include_str!("../presentation/web/app.js");
    let mut hits: Vec<String> = Vec::new();
    let mut from = 0;
    while let Some(pos) = src[from..].find("/api/") {
        let start = from + pos;
        let tail = &src[start..];
        let end = tail
            .find(|c: char| !(c.is_alphanumeric() || matches!(c, '/' | '_' | '-' | '.')))
            .unwrap_or(tail.len());
        let path = &tail[..end];
        if !hits.iter().any(|h| h == path) {
            hits.push(path.to_string());
        }
        from = start + 4;
        if from >= src.len() {
            break;
        }
    }
    assert!(
        !hits.is_empty(),
        "没从 app.js 里认出任何 /api/ 调用（提取逻辑失效了？）"
    );
    for p in &hits {
        // 字面前缀即可：动态段由前端拼在后面（'/api/sessions/' + encodeURIComponent(sid) + '/config'）。
        let known = ROUTES
            .iter()
            .any(|r| r.pattern == p.as_str() || r.pattern.starts_with(p.as_str()));
        assert!(known, "前端调用了目录里没有的路径：{}", p);
    }
}

// ---------- 逐条路由：错误 / 空 / 边界 ----------

#[test]
fn every_api_route_reports_capability_failure_as_4xx() {
    let ops = fake_ops(Some("假能力面：一律失败"));
    for route in ROUTES {
        if route.capability == "静态资源" || route.id == "events" {
            continue; // 这两类不碰能力
        }
        let (code, text) = call(&ops, route.method, &sample_url(route), sample_body(route));
        assert!(
            (400..500).contains(&code),
            "{} {} 能力失败时该回 4xx，实际 {}：{}",
            route.method,
            route.pattern,
            code,
            text
        );
        assert!(
            text.contains("error"),
            "{} {} 的错误体要带 error：{}",
            route.method,
            route.pattern,
            text
        );
        assert!(text.contains("假能力面"), "错误要如实传播：{}", text);
    }
}

#[test]
fn request_bodies_are_validated_before_anything_else() {
    let ops = fake_ops(None);
    for route in ROUTES {
        if !matches!(
            route.id,
            "session.new"
                | "session.act"
                | "provider.new"
                | "model.new"
                | "agent.new"
                | "settings.set"
                | "suggest"
        ) {
            continue;
        }
        let (code, text) = call(&ops, route.method, &sample_url(route), "不是 JSON");
        assert_eq!(
            code, 400,
            "{} {} 坏 JSON 要回 400",
            route.method, route.pattern
        );
        assert!(text.contains("请求不是 JSON"), "{}", text);
    }
}

#[test]
fn session_action_boundaries_are_explicit() {
    let ops = fake_ops(None);
    // 未知动作：400 + 明确讲清是未知动作（老语义保留）。
    let (code, text) = call(&ops, "POST", "/api/sessions/w1/乱来", "{}");
    assert_eq!(code, 400);
    assert!(text.contains("未知动作"), "{}", text);
    // 未知模式：400。
    let (code, text) = call(
        &ops,
        "POST",
        "/api/sessions",
        r#"{"name":"w","mode":"三个和尚"}"#,
    );
    assert_eq!(code, 400);
    assert!(text.contains("未知模式"), "{}", text);
    // 上传同名冲突：409（不是 400 —— 前端据此弹「覆盖/改名」）。
    let (code, _) = call(
        &ops,
        "POST",
        "/api/sessions/w1/upload",
        r#"{"name":"a.txt","data_base64":"aGk="}"#,
    );
    assert_eq!(code, 409);
    // 供应商/模型/agent 的未知动作：404。
    assert_eq!(call(&ops, "POST", "/api/providers/p1/乱来", "{}").0, 404);
    assert_eq!(call(&ops, "POST", "/api/models/m1/乱来", "{}").0, 404);
    assert_eq!(call(&ops, "POST", "/api/agents/甲/乱来", "{}").0, 404);
    // 停止：只要没在跑也回 {ok:true}（与老语义一致，前端不看 found）。
    let (code, text) = call(&ops, "POST", "/api/sessions/w1/stop", "{}");
    assert_eq!(code, 200);
    assert!(text.contains("\"ok\":true"), "{}", text);
}

// ---------- 逐条路由：成功形态 ----------

/// 探测回包：三种结论**原样**穿过呈现层（不改写、不降级），`mode` 取自登记处（探测后的事实）。
#[test]
fn model_probe_passes_the_verdict_through_verbatim() {
    use crate::core::ports::ProbeOutcome;
    let cases = [
        (
            ProbeOutcome::Supported {
                detail: "真的调了".to_string(),
            },
            "supported",
            "真的调了",
        ),
        (
            ProbeOutcome::Unsupported {
                detail: "供应商说 tools 不认识".to_string(),
            },
            "unsupported",
            "供应商说 tools 不认识",
        ),
        (
            ProbeOutcome::Unknown {
                detail: "没发起调用".to_string(),
            },
            "unknown",
            "没发起调用",
        ),
    ];
    for (outcome, want, detail) in cases {
        let ops = fake_ops_probe(None, outcome);
        let (code, text) = call(&ops, "POST", "/api/models/m1/probe", "");
        assert_eq!(code, 200, "{}", text);
        let v: serde_json::Value = serde_json::from_str(&text).expect("探测回包是 JSON");
        assert_eq!(v["outcome"], want, "{}", text);
        assert_eq!(v["detail"], detail, "{}", text);
        assert_eq!(
            v["mode"], "envelope",
            "形态取登记处现有值（替身的模型就是 envelope），不由结论反推：{}",
            text
        );
    }
    // 能力面失败要如实传播（绝不静默降级成"不支持"）。
    let ops = fake_ops(Some("假能力面：探测失败"));
    let (code, text) = call(&ops, "POST", "/api/models/m1/probe", "");
    assert_eq!(code, 400);
    assert!(text.contains("假能力面"), "{}", text);
}

/// 回放形状探测回包：逐项如实穿过呈现层（含"收了但没读懂"这一档与供应商原话），且不改登记处。
#[test]
fn replay_probe_passes_every_shape_through_verbatim() {
    let ops = fake_ops(None);
    let (code, text) = call(&ops, "POST", "/api/models/m1/probe-replay", "");
    assert_eq!(code, 200, "{}", text);
    let v: serde_json::Value = serde_json::from_str(&text).expect("回包是 JSON");
    assert_eq!(v["ok"], true);
    let shapes = v["shapes"].as_array().expect("shapes 是数组");
    assert_eq!(shapes.len(), 2, "{}", text);
    assert_eq!(shapes[0]["name"], "baseline-text");
    assert_eq!(shapes[0]["accepted"], true);
    assert_eq!(shapes[0]["understood"], true);
    assert_eq!(shapes[1]["accepted"], false);
    assert_eq!(shapes[1]["understood"], false);
    assert!(
        shapes[1]["detail"]
            .as_str()
            .unwrap_or("")
            .contains("content is required"),
        "被拒要带供应商原话：{}",
        text
    );
    // 能力面失败要如实传播，绝不静默当成"形状被拒"。
    let ops = fake_ops(Some("假能力面：探测失败"));
    let (code, text) = call(&ops, "POST", "/api/models/m1/probe-replay", "");
    assert_eq!(code, 400);
    assert!(text.contains("假能力面"), "{}", text);
}

#[test]
fn success_shapes_are_pinned_per_route() {
    let ops = fake_ops(None);
    ops.events.seed_for_test("w1");
    let cases: Vec<(&str, &str, &str, u16, &str)> = vec![
        ("GET", "/", "", 200, "<!DOCTYPE"),
        ("GET", "/style.css", "", 200, "--"),
        ("GET", "/app.js", "", 200, "/api/"),
        ("GET", "/md.js", "", 200, "function esc"),
        ("GET", "/api/events?sid=&since=0", "", 200, "\"head\""),
        ("GET", "/api/state", "", 200, "\"modules\""),
        (
            "POST",
            "/api/sessions",
            r#"{"name":"w","mode":"single","agents":[{"name":"a","modules":["m"]}]}"#,
            200,
            "\"sid\"",
        ),
        (
            "POST",
            "/api/sessions/w1/say",
            r#"{"text":"你好"}"#,
            200,
            "\"seq\"",
        ),
        (
            "POST",
            "/api/sessions/w1/rewind",
            r#"{"id":0}"#,
            200,
            "\"events\"",
        ),
        (
            "POST",
            "/api/sessions/w1/update-task",
            r#"{"text":"新需求"}"#,
            200,
            "\"events\"",
        ),
        (
            "POST",
            "/api/sessions/w1/withdraw",
            r#"{"agent":"甲"}"#,
            200,
            "\"seq\"",
        ),
        ("POST", "/api/sessions/w1/pending", "{}", 200, "\"pending\""),
        (
            "POST",
            "/api/sessions/w1/edit",
            r#"{"agents":[],"tier":"host"}"#,
            200,
            "\"ok\"",
        ),
        (
            "POST",
            "/api/sessions/w1/upload",
            r#"{"name":"a.txt","data_base64":"aGk=","overwrite":true}"#,
            200,
            "\"ok\"",
        ),
        ("GET", "/api/sessions/w1/config", "", 200, "\"config\""),
        ("GET", "/api/sessions/w1/files", "", 200, "\"roots\""),
        (
            "POST",
            "/api/providers",
            r#"{"id":"p","base_url":"u","api_key":"k"}"#,
            200,
            "\"ok\"",
        ),
        ("POST", "/api/providers/p1/remove", "", 200, "\"ok\""),
        ("POST", "/api/providers/p1/discover", "", 200, "\"models\""),
        (
            "POST",
            "/api/models",
            r#"{"id":"m","name":"M","api_model":"m","provider":"p"}"#,
            200,
            "\"ok\"",
        ),
        ("POST", "/api/models/m1/remove", "", 200, "\"ok\""),
        ("POST", "/api/models/m1/core", "", 200, "\"ok\""),
        ("POST", "/api/models/m1/probe", "", 200, "\"outcome\""),
        (
            "POST",
            "/api/agents",
            r#"{"name":"甲","modules":["m"]}"#,
            200,
            "\"ok\"",
        ),
        ("POST", "/api/agents/%E7%94%B2/remove", "", 200, "\"ok\""),
        ("GET", "/api/settings", "", 200, "\"settings\""),
        ("POST", "/api/settings", "{}", 200, "\"ok\""),
        ("GET", "/api/history", "", 200, "\"sessions\""),
        ("GET", "/api/history/w1", "", 200, "\"meta\""),
        ("POST", "/api/history/w1/delete", "", 200, "\"ok\""),
        (
            "POST",
            "/api/suggest-models",
            r#"{"task":"t","mode":"single"}"#,
            200,
            "\"agents\"",
        ),
    ];
    for (method, url, body, want_code, needle) in &cases {
        let (code, text) = call(&ops, method, url, body);
        assert_eq!(
            code, *want_code,
            "{} {} 期望 {}，实际 {}：{}",
            method, url, want_code, code, text
        );
        if *needle != "--" {
            let shown: String = text.chars().take(300).collect();
            assert!(
                text.contains(needle),
                "{} {} 的响应里应有 {}：{}",
                method,
                url,
                needle,
                shown
            );
        }
    }
    // 每条目录里的路由都要有一条成功用例盯着（按形状比，不按具体参数值）。
    for route in ROUTES {
        let covered = cases
            .iter()
            .any(|(m, u, _, _, _)| *m == route.method && shape_matches(route.pattern, u));
        assert!(
            covered,
            "目录里的 {} {} 没有成功用例",
            route.method, route.pattern
        );
    }
}
