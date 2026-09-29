//! 登记处能力测试：供应商 / 模型 / agent / 设置与通道解析。
//! 归属判据：钉的是**本能力的不变式**；顺手经过别处只是路径，不是归属。
use super::builders::*;
use super::prelude::*;

#[test]
pub(crate) fn settings_resolves_model_to_channel() {
    let mut s = Settings::default();
    s.providers.insert(
        "p".into(),
        Provider {
            base_url: "http://x".into(),
            api_key: "k".into(),
        },
    );
    s.models.insert(
        "m".into(),
        ModelEntry {
            name: "展示名".into(),
            api_model: "real-model".into(),
            provider: "p".into(),
            note: String::new(),
            tools: crate::capabilities::llm::api::ToolMode::Native,
            context: 32_000,
        },
    );
    s.core = Some("m".into());
    let ch = s.resolve("m").unwrap();
    assert_eq!(
        ch.model, "real-model",
        "发给供应商的是 api_model，不是展示名"
    );
    assert_eq!(ch.base_url, "http://x");
    // 缺省 = envelope（手写信封：任何供应商都能用）；模型视图也如实带出来
    s.models.insert(
        "d".into(),
        ModelEntry {
            name: "缺省".into(),
            api_model: "d".into(),
            provider: "p".into(),
            note: String::new(),
            tools: Default::default(),
            context: 32_000,
        },
    );
    assert!(s.resolve("d").is_ok(), "缺省形态的模型照样能解析出通道");
    assert_eq!(
        s.model_views()
            .iter()
            .find(|v| v.id == "m")
            .map(|v| v.tools),
        Some(crate::capabilities::llm::api::ToolMode::Native),
        "模型视图要带上形态（前端显示与探测结果都靠它）"
    );
    assert!(s.resolve("ghost").is_err(), "未知模型必须报错");
    assert_eq!(s.core_channel().unwrap().model, "real-model");
}

#[test]
pub(crate) fn provider_lifecycle_and_key_never_leaks_to_view() {
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec!["[]".into()]));
    core.registry_mut()
        .provider_upsert("p1", "http://x", "sk-密钥XYZ")
        .unwrap();
    for v in core.registry().provider_views() {
        // 展示文案由呈现层拼（conductor 不再提供 CLI 行），"密钥永不出现"这条红线两处都要成立。
        let shown = format!("{}  {}", v.id, v.base_url);
        assert!(!shown.contains("sk-密钥XYZ"), "视图出现密钥：{}", shown);
        assert!(!format!("{:?}", v).contains("sk-密钥XYZ"));
    }
    // 仍被模型引用时拒绝删除供应商（不静默级联）
    core.registry_mut()
        .model_upsert("m1", "M", "api-m", "p1", "快", 0)
        .unwrap();
    assert!(core
        .registry_mut()
        .provider_remove("p1")
        .unwrap_err()
        .contains("仍被模型引用"));
    assert!(core.registry_mut().model_remove("m1").unwrap());
    assert!(core.registry_mut().provider_remove("p1").unwrap());
}

/// 模型的上下文窗口：给了就改；表单没带（0）时**保留现值**——编辑别的字段不该顺手重置它。
#[test]
pub(crate) fn model_context_is_kept_when_the_form_omits_it() {
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec!["[]".into()]));
    core.registry_mut()
        .provider_upsert("p1", "http://x", "k")
        .unwrap();
    core.registry_mut()
        .model_upsert("m", "M", "api-m", "p1", "", 64_000)
        .unwrap();
    let stored = |c: &crate::capabilities::conductor::service::Conductor| {
        c.registry()
            .model_views()
            .iter()
            .find(|v| v.id == "m")
            .map(|v| v.context)
            .unwrap_or(0)
    };
    assert_eq!(stored(&core), 64_000, "给了窗口就按它存");
    // 再存一次（比如只改说明），不带窗口 → 保留 64000，而不是被重置成缺省。
    core.registry_mut()
        .model_upsert("m", "M2", "api-m", "p1", "改了说明", 0)
        .unwrap();
    assert_eq!(stored(&core), 64_000, "表单没带窗口时保留现值");
}

#[test]
pub(crate) fn model_guards_core_default_and_discovery_uses_stored_provider() {
    let catalog = Arc::new(FakeCatalog::new(vec!["m-a".to_string(), "m-b".to_string()]));
    let mut core = core_with_catalog(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::clone(&catalog),
    );
    core.registry_mut()
        .provider_upsert("p2", "http://x", "k")
        .unwrap();
    assert_eq!(
        core.registry().discover_models("p2").unwrap(),
        vec!["m-a".to_string(), "m-b".to_string()]
    );
    assert_eq!(catalog.seen.lock().expect("锁")[0], "http://x");
    assert!(core
        .registry()
        .discover_models("ghost")
        .unwrap_err()
        .contains("无此供应商"));
    // 引用了不存在的供应商 → 拒绝登记
    assert!(core
        .registry_mut()
        .model_upsert("bad", "B", "b", "ghost", "", 0)
        .unwrap_err()
        .contains("无此供应商"));
    // 核心默认模型不可删；换默认后旧的可删
    assert!(core
        .registry_mut()
        .model_remove("m")
        .unwrap_err()
        .contains("核心默认模型"));
    core.registry_mut()
        .model_upsert("m2", "M2", "api-m2", "p2", "", 0)
        .unwrap();
    assert!(core.registry_mut().core_set_model("m2").unwrap());
    assert!(core.registry_mut().model_remove("m").unwrap());
}

#[test]
pub(crate) fn agent_crud_and_work_with_agents() {
    let mut core = core_with(
        vec![module_of("a"), module_of("b")],
        gw(BTreeMap::new(), vec!["[]".into()]),
    );
    assert!(agent_upsert(&mut core, "", &["a"], "", "")
        .unwrap_err()
        .contains("不能为空"));
    assert!(agent_upsert(&mut core, "x", &[], "", "")
        .unwrap_err()
        .contains("至少要有一个模块"));
    assert!(agent_upsert(&mut core, "x", &["ghost"], "", "")
        .unwrap_err()
        .contains("无此模块"));
    assert!(agent_upsert(&mut core, "x", &["a"], "nope", "")
        .unwrap_err()
        .contains("无此模型"));
    agent_upsert(&mut core, "调研", &["a", "b"], "m", "说明").unwrap();
    assert_eq!(core.registry().agent_views().len(), 1);
    assert_eq!(core.registry().agent_views()[0].modules.len(), 2);

    // 组合：一个 agent（多模块合并）
    let spec = WorkSpec {
        name: "w".to_string(),
        mode: WorkMode::Single,
        agents: vec![AgentInstance {
            name: "调研".to_string(),
            transient: false,
            modules: vec!["a".to_string(), "b".to_string()],
            model: Some("m".to_string()),
        }],
        task: None,
        delegate: false,
    };
    let opened = core.create_work(spec).unwrap();
    assert_eq!(opened.agents, vec!["调研".to_string()]);
    let meta = core.history_open("w").unwrap().0;
    assert_eq!(meta.agents.len(), 1);
    assert_eq!(meta.agents[0].name, "调研");

    // 协作：多 agent；同工作内重名自动加尾号
    let mut dup = collab_work("c", &["a", "b"], false, "需求");
    dup.agents[1].name = dup.agents[0].name.clone();
    let o = core.create_work(dup).unwrap();
    assert_eq!(o.agents, vec!["a".to_string(), "a-2".to_string()]);

    // 同一模块不得跨 agent 重复
    let cross = WorkSpec {
        name: "cross".to_string(),
        mode: WorkMode::Collab,
        agents: vec![
            AgentInstance {
                name: "x".to_string(),
                transient: true,
                modules: vec!["a".to_string()],
                model: None,
            },
            AgentInstance {
                name: "y".to_string(),
                transient: true,
                modules: vec!["a".to_string()],
                model: None,
            },
        ],
        task: Some("需求".to_string()),
        delegate: false,
    };
    assert!(core
        .create_work(cross)
        .unwrap_err()
        .contains("被多个 agent"));

    assert!(core.registry_mut().agent_remove("调研").unwrap());
    assert!(core.registry().agent_views().is_empty());
}

#[test]
pub(crate) fn model_catalog_parses_openai_shape_and_rejects_bad() {
    use crate::capabilities::llm::detail::model_catalog::parse_models;
    assert_eq!(
        parse_models(r#"{"data":[{"id":"gpt-4o"},{"id":"o3"},{"id":"gpt-4o"}]}"#).unwrap(),
        vec!["gpt-4o".to_string(), "o3".to_string()]
    );
    assert!(
        parse_models(r#"{"models":["a"]}"#).is_err(),
        "缺 data 必须报错"
    );
    assert!(parse_models(r#"{"data":[]}"#).is_err(), "空列表必须报错");
    assert!(parse_models("不是 JSON").is_err());
}

#[test]
pub(crate) fn endpoint_candidates_complete_and_fall_back() {
    use crate::capabilities::llm::detail::endpoint::{
        chat_candidates, models_candidates, retryable_status,
    };
    // 已带版本段：只补后缀（含尾斜杠）
    assert_eq!(
        chat_candidates("https://api.x/v1"),
        vec!["https://api.x/v1/chat/completions"]
    );
    assert_eq!(
        chat_candidates("https://api.x/v1/"),
        vec!["https://api.x/v1/chat/completions"]
    );
    assert_eq!(
        chat_candidates("https://api.x/v2"),
        vec!["https://api.x/v2/chat/completions"]
    );
    assert_eq!(
        models_candidates("https://api.x/v1"),
        vec!["https://api.x/v1/models"]
    );
    // 无版本段：先直连，连不上再回落 /v1
    assert_eq!(
        chat_candidates("https://api.x"),
        vec![
            "https://api.x/chat/completions",
            "https://api.x/v1/chat/completions"
        ]
    );
    assert_eq!(
        models_candidates("https://api.x"),
        vec!["https://api.x/models", "https://api.x/v1/models"]
    );
    // 已是完整端点：原样，防重复拼接
    assert_eq!(
        chat_candidates("https://api.x/v1/chat/completions"),
        vec!["https://api.x/v1/chat/completions"]
    );
    assert_eq!(
        models_candidates("https://api.x/v1/models"),
        vec!["https://api.x/v1/models"]
    );
    // 换候选判定：只有 404/405 换，鉴权类立即报
    assert!(retryable_status(404) && retryable_status(405));
    assert!(
        !retryable_status(401)
            && !retryable_status(403)
            && !retryable_status(429)
            && !retryable_status(500)
    );
}

#[test]
pub(crate) fn endpoint_resolve_candidates_retries_shape_mismatch_and_stops_on_fatal() {
    use crate::capabilities::llm::detail::endpoint::{resolve_candidates, Attempt};
    let cands = vec!["a".to_string(), "b".to_string(), "c".to_string()];

    // 回归：SPA catch-all 返回 200 HTML（形状不符=Retry）时必须换到下一个候选，而不是立即报错。
    let mut seen = Vec::new();
    let got = resolve_candidates(
        &cands,
        |url| {
            seen.push(url.to_string());
            if url == "b" {
                Attempt::Ok(7)
            } else {
                Attempt::Retry("响应不是 JSON".to_string())
            }
        },
        |_, _, _| {},
    );
    assert_eq!(got.unwrap(), ("b".to_string(), 7));
    assert_eq!(seen, vec!["a".to_string(), "b".to_string()]);

    // 鉴权类 Fatal 立即停，不再试后续候选。
    let mut seen2 = Vec::new();
    let fatal: Result<(String, i32), String> = resolve_candidates(
        &cands,
        |url| {
            seen2.push(url.to_string());
            Attempt::Fatal("供应商返回 401".to_string())
        },
        |_, _, _| {},
    );
    assert_eq!(fatal.unwrap_err(), "供应商返回 401");
    assert_eq!(seen2, vec!["a".to_string()]);

    // 全部 Retry 耗尽：报最后一个错误，不静默。
    let all: Result<(String, i32), String> =
        resolve_candidates(&cands, |_| Attempt::Retry("失败".to_string()), |_, _, _| {});
    assert_eq!(all.unwrap_err(), "失败");
}

// ---------- 模块清单（内存来源） ----------

#[test]
pub(crate) fn a_probe_writes_back_only_conclusive_results() {
    use crate::capabilities::conductor::api::ProbeOutcome;
    use crate::capabilities::llm::api::ToolMode;
    let fresh = |outcome: ProbeOutcome| {
        let mut core = core_with_gateway(
            vec![],
            ProbeGateway {
                outcome: Arc::new(Mutex::new(outcome)),
            },
        );
        core.registry_mut()
            .provider_upsert("p", "http://x", "k")
            .expect("登记供应商");
        core.registry_mut()
            .model_upsert("m", "M", "api-m", "p", "", 0)
            .expect("登记模型");
        core
    };
    let mode_of = |core: &Conductor| {
        core.registry()
            .model_views()
            .iter()
            .find(|v| v.id == "m")
            .map(|v| v.tools)
    };

    // 支持 → 写回 native
    let mut core = fresh(ProbeOutcome::Supported {
        detail: "真的调了".to_string(),
    });
    assert_eq!(
        mode_of(&core),
        Some(ToolMode::Envelope),
        "探测前是缺省 envelope"
    );
    assert!(matches!(
        core.registry_mut().probe_model_tools("m"),
        Ok(ProbeOutcome::Supported { .. })
    ));
    assert_eq!(mode_of(&core), Some(ToolMode::Native), "支持就写回 native");

    // 明确不支持 → 写回 envelope
    let mut core = fresh(ProbeOutcome::Unsupported {
        detail: "供应商说 tools 不认识".to_string(),
    });
    assert!(matches!(
        core.registry_mut().probe_model_tools("m"),
        Ok(ProbeOutcome::Unsupported { .. })
    ));
    assert_eq!(
        mode_of(&core),
        Some(ToolMode::Envelope),
        "不支持就老实回到信封"
    );

    // 无法判定 → 不改（不替用户拍板），但事实照样报回去
    let mut core = fresh(ProbeOutcome::Unknown {
        detail: "没发起调用".to_string(),
    });
    assert!(matches!(
        core.registry_mut().probe_model_tools("m"),
        Ok(ProbeOutcome::Unknown { .. })
    ));
    assert_eq!(
        mode_of(&core),
        Some(ToolMode::Envelope),
        "没法定论就不动登记处"
    );

    // 无此模型 → 如实报错
    assert!(core.registry_mut().probe_model_tools("ghost").is_err());
}
