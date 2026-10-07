//! HTTP 入站契约的契约测试：路由目录 ↔ 处理器 ↔ 文档 ↔ 前端调用 ↔ 演示脚本，机器比对。
//! 用**假能力面**（FakeOps）直接调 `web::route`（不经过 socket），逐条路由验成功/错误/空/边界；
//! 真实传输由 L4 端到端覆盖（真二进制 + 真 HTTP）。
//! 假能力面顺带证明一件事：「按角色切分」的能力接口真能被替换——新增一种呈现不必认识 `Conductor`。

use crate::capabilities::conductor::api::{
    Acted, ActionCall, ActionOps, ActionView, Advance, Caller, ConductorOps, EventBus, Ops, Output,
    SessionOps,
};
use crate::capabilities::conductor::api::{
    AgentSuggestion, ConfigAgent, DecisionQueue, FilesAgentView, FilesRootsView, FilesView,
    RuntimeReport, SessionConfig, SessionEdit, SessionView, TierChoices, WorkMode, WorkOpened,
    WorkSpec,
};
use crate::capabilities::registry::api::AgentView;
use crate::capabilities::registry::api::RegistryOps;
use crate::capabilities::registry::api::{AppSettings, ModelView, ProviderView};
use crate::capabilities::session::api::{HistoryOps, HistoryView, SessionMeta};
use crate::capabilities::workspace::api::{Roster, WorkspaceOps};
use crate::kernel::api::Tier;
use crate::presentation::web::routes::{self, ROUTES};
use crate::presentation::web::{self, FenceInfo};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// ---------- 假能力面 ----------

/// 假能力面：不碰核心、不起线程。`fail` 一注入，所有能力一律失败（用来验错误路径）。
#[derive(Default)]
struct FakeOps {
    fail: Option<String>,
    running: AtomicBool,
}

impl FakeOps {
    fn new(fail: Option<&str>) -> FakeOps {
        FakeOps {
            fail: fail.map(|s| s.to_string()),
            running: AtomicBool::new(false),
        }
    }

    fn guard(&self) -> Result<(), String> {
        match &self.fail {
            Some(m) => Err(m.clone()),
            None => Ok(()),
        }
    }
}

fn fake_ops(fail: Option<&str>) -> Ops {
    let f = Arc::new(FakeOps::new(fail));
    Ops {
        sessions: f.clone(),
        registry: f.clone(),
        history: f.clone(),
        core: f.clone(),
        workspace: f.clone(),
        actions: f.clone(),
        events: EventBus::new(),
        log: Arc::new(super::doubles::NoopLogOps),
    }
}

/// 假动作面：目录给空表，分发按 id 给**有形状的回包**（路由契约测试只盯形状与错误传播）。
impl ActionOps for FakeOps {
    fn catalog(&self, _caller: &Caller, _sid: Option<&str>) -> Result<Vec<ActionView>, String> {
        self.guard()?;
        Ok(Vec::new())
    }
    fn act(&self, call: ActionCall) -> Result<Acted, String> {
        self.guard()?;
        Ok(match call.id.as_str() {
            "create_session" => Acted::Done(json!({ "sid": "w1", "agents": ["a"] })),
            "rewind" | "update_task" => Acted::Replayed(Vec::new()),
            "send_message" | "set_task" | "answer_card" | "withdraw" | "compact" => {
                Acted::Advanced(Advance { head: 1 })
            }
            _ => Acted::Done(json!({ "ok": true })),
        })
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
        exec: crate::capabilities::workspace::api::ExecSpec::default(),
        parent: None,
        node: None,
        delegation: None,
        run: crate::capabilities::session::api::RunState::Active,
    }
}

impl SessionOps for FakeOps {
    fn create_work(&self, _spec: WorkSpec) -> Result<(WorkOpened, u64), String> {
        self.guard()?;
        Ok((
            WorkOpened {
                sid: "w1".to_string(),
                agents: vec!["甲".to_string()],
                facts: vec![crate::capabilities::conductor::api::SessionEvent::Notice(
                    "开好了".to_string(),
                )],
            },
            7,
        ))
    }
    fn say(&self, _sid: &str, _text: &str, _out: Output) -> Result<Advance, String> {
        // 命令回包只给事件台头部序号：事实由长轮询按 since 订阅。
        self.guard()?;
        Ok(Advance { head: 7 })
    }
    fn continue_flow(&self, _sid: &str, _out: Output) -> Result<Advance, String> {
        self.guard()?;
        Ok(Advance { head: 8 })
    }
    fn set_task(&self, _sid: &str, _text: &str) -> Result<Advance, String> {
        self.guard()?;
        Ok(Advance { head: 9 })
    }
    fn answer_card(
        &self,
        _sid: &str,
        _card: &str,
        _option: &str,
        _note: &str,
    ) -> Result<Advance, String> {
        self.guard()?;
        Ok(Advance { head: 10 })
    }
    /// 假卡：路由契约测试只盯形状——封套 / 消息 / 选项都在（队列里只有它一张）。
    fn open_card(&self, sid: &str) -> Result<Option<DecisionQueue>, String> {
        self.guard()?;
        Ok(Some(DecisionQueue {
            card: serde_json::from_value(json!({
                "id": format!("d1-{}", sid),
                "envelope": { "role": "core", "name": "核心" },
                "message": { "title": "要不要继续？", "body": "假能力面", "detail": "" },
                "options": [{ "id": "go", "label": "继续" }],
            }))
            .expect("假卡形状"),
            waiting: Vec::new(),
        }))
    }
    fn withdraw_agree(&self, _sid: &str, _agent: &str) -> Result<Advance, String> {
        self.guard()?;
        Ok(Advance { head: 10 })
    }
    fn compact(&self, _sid: &str) -> Result<crate::capabilities::conductor::api::Advance, String> {
        self.guard()?;
        Ok(crate::capabilities::conductor::api::Advance { head: 0 })
    }
    fn rewind(
        &self,
        _sid: &str,
        _target: crate::capabilities::conductor::api::RewindTarget,
    ) -> Result<Vec<serde_json::Value>, String> {
        self.guard()?;
        Ok(vec![json!({ "type": "line", "line": "重放" })])
    }
    fn update_task(&self, _sid: &str, _text: &str) -> Result<Vec<serde_json::Value>, String> {
        self.guard()?;
        Ok(vec![json!({ "type": "line", "line": "重放" })])
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
                permissions: None,
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
            usage: Default::default(),
        })
    }
    fn unique_work_name(&self, _base: &str, fallback: &str) -> Result<String, String> {
        Ok(fallback.to_string())
    }

    fn exists(&self, _sid: &str) -> Result<bool, String> {
        self.guard()?;
        Ok(true)
    }
    fn stop(&self, _sid: &str) -> Vec<String> {
        self.running.swap(false, Ordering::Relaxed);
        Vec::new()
    }
    fn approve(&self, _sid: &str, _answer: crate::capabilities::conductor::api::Approval) -> bool {
        false
    }

    fn pending_approval(
        &self,
        _sid: &str,
    ) -> Option<crate::capabilities::conductor::api::ApprovalView> {
        None
    }
    fn is_running(&self, _sid: &str) -> bool {
        self.running.load(Ordering::Relaxed)
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
                running: false,
                can_update_task: h.mode == "collab",
                run: h.run.as_str().to_string(),
                pending: None,
            })
            .collect())
    }
}

impl RegistryOps for FakeOps {
    fn pick_agents(&self, names: &[String]) -> Result<Vec<AgentView>, String> {
        self.guard()?;
        Ok(names
            .iter()
            .map(|n| AgentView {
                name: n.clone(),
                modules: vec!["m".to_string()],
                model: None,
                note: String::new(),
            })
            .collect())
    }

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
            tools: crate::capabilities::llm::api::ToolMode::Envelope,
            context: 32_000,
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
        _context: u64,
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
    ) -> Result<crate::capabilities::llm::api::ReplayReport, String> {
        self.guard()?;
        Ok(crate::capabilities::llm::api::ReplayReport {
            shapes: vec![
                crate::capabilities::llm::api::ReplayShape {
                    name: "baseline-text".to_string(),
                    accepted: true,
                    understood: true,
                    detail: "finish_reason=stop".to_string(),
                },
                crate::capabilities::llm::api::ReplayShape {
                    name: "content-empty".to_string(),
                    accepted: false,
                    understood: false,
                    detail: "供应商原话：content is required".to_string(),
                },
            ],
        })
    }
    fn probe_model_tools(
        &self,
        _id: &str,
    ) -> Result<crate::capabilities::llm::api::ProbeOutcome, String> {
        self.guard()?;
        Ok(crate::capabilities::llm::api::ProbeOutcome::Supported {
            detail: "替身说支持".to_string(),
        })
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
            run: crate::capabilities::session::api::RunState::Active,
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
}

impl WorkspaceOps for FakeOps {
    fn roster(&self) -> Result<Roster, String> {
        self.guard()?;
        Ok(Roster {
            modules: Vec::new(),
            rejected: Vec::new(),
        })
    }
}

impl ConductorOps for FakeOps {
    fn runtime_report(&self, _tier: Tier) -> Result<RuntimeReport, String> {
        self.guard()?;
        Ok(report())
    }
    fn tier_choices(&self) -> Result<TierChoices, String> {
        self.guard()?;
        Ok(TierChoices {
            default: "host".to_string(),
            vm_available: false,
            vm_unavailable_reason: "guest 本体尚未接入".to_string(),
            vm_requirements: Vec::new(),
        })
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
    let log: Arc<dyn crate::capabilities::conductor::api::LogOps + Send + Sync> =
        Arc::new(super::doubles::NoopLogOps);
    let (code, _headers, text) = web::route(ops, &fence(), &log, method, url, body);
    (code, text)
}

/// 目录里的参数取样（只为本文件合成请求用）：动作路由要给出**合法**动作 id，否则先死在未知动作上。
fn sample_param(_route_id: &str, name: &str) -> &'static str {
    match name {
        "sid" => "w1",
        "name" => "a1",
        "id" => "create_session",
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
        "action" => r#"{"mode":"single","agents":[]}"#,
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
    let doc = include_str!("../../docs/presentation/contracts.md");
    let body = doc
        .split_once("<!-- ROUTES:BEGIN -->")
        .and_then(|(_, rest)| rest.split_once("<!-- ROUTES:END -->"))
        .map(|(b, _)| b)
        .expect("docs/presentation/contracts.md 必须有 ROUTES:BEGIN/END 包裹的路由表");
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
    let src = include_str!("../presentation/web/assets/app.js");
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

/// 演示脚本（`demo/*.mjs`）也只走目录里的路径：**演示是产品对外的一张脸**，改路由不该让它静默失效。
#[test]
fn demo_scripts_only_call_catalogued_paths() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("demo");
    let mut files = 0;
    let mut hits: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("demo/ 必须存在") {
        let path = entry.expect("可读的目录项").path();
        if path.extension().and_then(|e| e.to_str()) != Some("mjs") {
            continue;
        }
        files += 1;
        let src = std::fs::read_to_string(&path).expect("演示脚本必须可读");
        let mut from = 0;
        while let Some(pos) = src[from..].find("/api/") {
            let start = from + pos;
            let tail = &src[start..];
            let end = tail
                .find(|c: char| !(c.is_alphanumeric() || matches!(c, '/' | '_' | '-' | '.')))
                .unwrap_or(tail.len());
            let p = tail[..end].to_string();
            if !hits.iter().any(|h| h == &p) {
                hits.push(p);
            }
            from = start + 4;
            if from >= src.len() {
                break;
            }
        }
    }
    assert!(files > 0, "demo/ 下一个 .mjs 都没有（演示怎么跑？）");
    assert!(
        !hits.is_empty(),
        "没从演示脚本里认出任何 /api/ 调用（提取逻辑失效了？）"
    );
    for p in &hits {
        // 与前端同一条口径：字面前缀即可，动态段由脚本拼在后面。
        let known = ROUTES
            .iter()
            .any(|r| r.pattern == p.as_str() || r.pattern.starts_with(p.as_str()));
        assert!(known, "演示脚本调用了目录里没有的路径：{}", p);
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
        if !matches!(route.id, "action" | "suggest") {
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
    // 不在目录里的路径：404（动作 id 的解释权归核心，传输层不猜）。
    assert_eq!(call(&ops, "POST", "/api/sessions/w1/乱来", "{}").0, 404);
    // 登记处路由已并入动作路由：这些老路径现在也不在目录里，同样是 404。
    assert_eq!(call(&ops, "POST", "/api/providers/p1/乱来", "{}").0, 404);
    assert_eq!(call(&ops, "POST", "/api/models/m1/乱来", "{}").0, 404);
    assert_eq!(call(&ops, "POST", "/api/agents/甲/乱来", "{}").0, 404);
    // 停止走动作路由：只要没在跑也回 {ok:true}（前端不看 state）。
    let (code, text) = call(
        &ops,
        "POST",
        "/api/actions/control_session",
        r#"{"session_id":"w1","action":"stop"}"#,
    );
    assert_eq!(code, 200);
    assert!(text.contains("\"ok\":true"), "{}", text);
}

// ---------- 逐条路由：成功形态 ----------

/// 事件台的 `sid` 过滤要**解百分号**：会话名常带中文（`<工作>--<agent>`，节点与讨论都用这个名字），
/// 不解回来就永远匹配不到任何事件——长轮询只能干等到超时，客户端拿到空批。
#[test]
fn events_sid_filter_decodes_percent_escapes() {
    let ops = fake_ops(None);
    ops.events.seed_for_test("工作--甲");
    // 「工作--甲」的 encodeURIComponent 结果（前端就是这么发的）。
    let encoded = "%E5%B7%A5%E4%BD%9C--%E7%94%B2";
    let (code, text) = call(
        &ops,
        "GET",
        &format!("/api/events?sid={}&since=0", encoded),
        "",
    );
    assert_eq!(code, 200, "{}", text);
    assert!(
        text.contains("工作--甲"),
        "编码过的 sid 要解回原文才能过滤到事件：{}",
        text
    );
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
            "/api/actions/create_session",
            r#"{"mode":"single","agents":[{"name":"a","modules":["m"]}]}"#,
            200,
            "\"sid\"",
        ),
        (
            "POST",
            "/api/actions/send_message",
            r#"{"session_id":"w1","text":"你好"}"#,
            200,
            "\"head\"",
        ),
        (
            "POST",
            "/api/actions/rewind",
            r#"{"session_id":"w1","mode":"archive","id":0}"#,
            200,
            "\"events\"",
        ),
        (
            "POST",
            "/api/actions/update_task",
            r#"{"session_id":"w1","text":"新需求"}"#,
            200,
            "\"events\"",
        ),
        (
            "POST",
            "/api/actions/withdraw",
            r#"{"session_id":"w1","agent":"甲"}"#,
            200,
            "\"head\"",
        ),
        (
            "POST",
            "/api/actions/edit_session",
            r#"{"session_id":"w1","agents":[],"tier":"host"}"#,
            200,
            "\"ok\"",
        ),
        (
            "POST",
            "/api/actions/upload",
            r#"{"session_id":"w1","name":"a.txt","data_base64":"aGk=","overwrite":true}"#,
            200,
            "\"ok\"",
        ),
        ("GET", "/api/actions?sid=w1", "", 200, "\"actions\""),
        ("GET", "/api/sessions/w1/config", "", 200, "\"config\""),
        ("GET", "/api/sessions/w1/files", "", 200, "\"roots\""),
        // 登记处动作现在都走 /api/actions/{id}（上面已有成功用例盯着这条路由的形态）。
        ("GET", "/api/settings", "", 200, "\"settings\""),
        ("GET", "/api/tiers", "", 200, "\"tiers\""),
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

/// 传输**可中断**：中断判据一置位，服务器主循环立刻返回（"Ctrl+C 回 CLI"的机制在这里）。
#[test]
fn web_serve_returns_when_interrupted() {
    use std::time::{Duration, Instant};
    let ops = fake_ops(None);
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = Arc::clone(&stop);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        s2.store(true, Ordering::SeqCst);
    });
    let started = Instant::now();
    let r = web::serve_until(&ops, 0, &fence(), &stop);
    assert!(r.is_ok(), "{:?}", r);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "收到中断要立刻返回，而不是阻塞到出错"
    );
}

/// CLI 的确认回答映射：yes / no / full（含常见别名与大小写），其它一律不认、要重新问。
#[test]
fn cli_approval_answers_map_to_allow_deny_full() {
    use crate::capabilities::conductor::api::Approval;
    use crate::presentation::cli::parse_approval;
    assert_eq!(parse_approval("yes"), Some(Approval::Allow));
    assert_eq!(parse_approval(" YES "), Some(Approval::Allow));
    assert_eq!(parse_approval("no"), Some(Approval::Deny));
    assert_eq!(parse_approval("full"), Some(Approval::Full));
    assert_eq!(parse_approval("f"), Some(Approval::Full));
    assert_eq!(parse_approval("随便"), None, "不认的要重新问");
}
