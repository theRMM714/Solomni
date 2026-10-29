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
            thinking: Default::default(),
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
            thinking: Default::default(),
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
        .model_upsert(model_draft("m1", "M", "api-m", "p1", "快", 0))
        .unwrap();
    assert!(core
        .registry_mut()
        .provider_remove("p1")
        .unwrap_err()
        .contains("仍被模型引用"));
    assert!(core.registry_mut().model_remove("m1").unwrap());
    assert!(core.registry_mut().provider_remove("p1").unwrap());
}

/// 模型的上下文窗口：给了就按它存；0 = 用户不填（原样存 0），自动压缩按 128K 兜底。
#[test]
pub(crate) fn model_context_zero_means_not_filled_and_falls_back_to_128k() {
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec!["[]".into()]));
    core.registry_mut()
        .provider_upsert("p1", "http://x", "k")
        .unwrap();
    core.registry_mut()
        .model_upsert(model_draft("m", "M", "api-m", "p1", "", 64_000))
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
    assert_eq!(core.registry().context_of(Some("m")), 64_000);
    // 再存一次带 0：0 是"用户不填"，原样存 0；自动压缩用 128K 兜底。
    core.registry_mut()
        .model_upsert(model_draft("m", "M2", "api-m", "p1", "改了说明", 0))
        .unwrap();
    assert_eq!(stored(&core), 0, "0 = 不填，原样保存");
    assert_eq!(
        core.registry().context_of(Some("m")),
        crate::capabilities::registry::domain::providers::DEFAULT_CONTEXT_TOKENS,
        "自动压缩的兜底阈值是 128K"
    );
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
        .model_upsert(model_draft("bad", "B", "b", "ghost", "", 0))
        .unwrap_err()
        .contains("无此供应商"));
    // 核心默认模型不可删；换默认后旧的可删
    assert!(core
        .registry_mut()
        .model_remove("m")
        .unwrap_err()
        .contains("核心默认模型"));
    core.registry_mut()
        .model_upsert(model_draft("m2", "M2", "api-m2", "p2", "", 0))
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
    // 模块可以为空：零模块 agent 只用内建文件工具，照样合法。
    agent_upsert(&mut core, "零模块", &[], "", "").unwrap();
    assert_eq!(
        core.registry()
            .agent_views()
            .iter()
            .find(|v| v.name == "零模块")
            .unwrap()
            .modules
            .len(),
        0
    );
    assert!(core.registry_mut().agent_remove("零模块").unwrap());
    assert!(agent_upsert(&mut core, "x", &["ghost"], "", "")
        .unwrap_err()
        .contains("无此模块"));
    assert!(agent_upsert(&mut core, "x", &["a"], "nope", "")
        .unwrap_err()
        .contains("无此模型"));
    agent_upsert_chara(
        &mut core,
        "调研",
        &["a", "b"],
        "m",
        "说明",
        "  严谨的资料整理者  ",
    )
    .unwrap();
    assert_eq!(core.registry().agent_views().len(), 1);
    assert_eq!(core.registry().agent_views()[0].modules.len(), 2);
    assert_eq!(
        core.registry().agent_views()[0].chara,
        "严谨的资料整理者",
        "人设随登记处保存，首尾空白去掉"
    );

    // 组合：一个 agent（多模块合并）
    let spec = WorkSpec {
        name: "w".to_string(),
        mode: WorkMode::Single,
        agents: vec![AgentInstance {
            name: "调研".to_string(),
            transient: false,
            modules: vec!["a".to_string(), "b".to_string()],
            model: Some("m".to_string()),
            chara: "严谨的资料整理者".to_string(),
        }],
        task: None,
        delegate: false,
        permission: None,
        tier: crate::kernel::api::Tier::Host,
        guest: None,
    };
    let opened = core.create_work(spec).unwrap();
    assert_eq!(opened.agents, vec!["调研".to_string()]);
    let meta = core.history_open("w").unwrap().0;
    assert_eq!(meta.agents.len(), 1);
    assert_eq!(meta.agents[0].name, "调研");
    assert_eq!(
        meta.agents[0].chara, "严谨的资料整理者",
        "建会话时人设进会话快照"
    );

    // 协作：多 agent；同工作内重名自动加尾号
    let mut dup = collab_work("c", &["a", "b"], false, "需求");
    dup.agents[1].name = dup.agents[0].name.clone();
    let o = core.create_work(dup).unwrap();
    assert_eq!(o.agents, vec!["a".to_string(), "a-2".to_string()]);

    // 同一模块可跨 agent 共用
    let cross = WorkSpec {
        name: "cross".to_string(),
        mode: WorkMode::Collab,
        agents: vec![
            AgentInstance {
                name: "x".to_string(),
                transient: true,
                modules: vec!["a".to_string()],
                model: None,
                chara: String::new(),
            },
            AgentInstance {
                name: "y".to_string(),
                transient: true,
                modules: vec!["a".to_string()],
                model: None,
                chara: String::new(),
            },
        ],
        task: Some("需求".to_string()),
        delegate: false,
        permission: None,
        tier: crate::kernel::api::Tier::Host,
        guest: None,
    };
    let opened = core.create_work(cross).expect("同一模块可跨 agent 共用");
    assert_eq!(opened.agents, vec!["x".to_string(), "y".to_string()]);
    let (meta, _) = core.history_open("cross").expect("打开会话");
    assert_eq!(meta.modules, vec!["a".to_string()], "meta.modules 去重");

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
            .model_upsert(model_draft("m", "M", "api-m", "p", "", 0))
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

/// 目录保留名校验（名单从 `systools/names.yaml` 注入）：大小写不敏感，普通名字放行。
#[test]
pub(crate) fn reserved_directory_names_are_rejected_case_insensitively() {
    let reserved = vec!["work".to_string(), "children".to_string()];
    assert!(crate::capabilities::registry::api::check_reserved("children", &reserved).is_err());
    assert!(
        crate::capabilities::registry::api::check_reserved("CHILDREN", &reserved).is_err(),
        "Windows 文件系统不区分大小写"
    );
    assert!(crate::capabilities::registry::api::check_reserved("甲", &reserved).is_ok());
}

/// 目的：默认目录就是四档；映射的字符串是 reasoning_effort 简写，对象原样合并，空对象 = 不发。
#[test]
pub(crate) fn thinking_defaults_and_map_shapes_are_resolved() {
    use crate::capabilities::registry::domain::providers::{ThinkingCatalog, ThinkingSpec};
    let mut s = Settings::default();
    assert_eq!(
        s.app
            .thinking
            .levels
            .iter()
            .map(|l| l.id.as_str())
            .collect::<Vec<_>>(),
        ["off", "low", "high", "max"],
        "默认四档"
    );
    let levels = BTreeMap::from([
        (
            "off".to_string(),
            serde_json::json!({ "thinking": { "type": "disabled" } }),
        ),
        ("low".to_string(), serde_json::json!("low")),
        ("high".to_string(), serde_json::json!("high")),
        ("mo".to_string(), serde_json::json!({})),
    ]);
    s.models.insert(
        "m".to_string(),
        ModelEntry {
            name: "M".to_string(),
            api_model: "m".to_string(),
            provider: "p".to_string(),
            note: String::new(),
            tools: Default::default(),
            context: 32_000,
            thinking: ThinkingSpec {
                default: Some("low".to_string()),
                levels,
            },
        },
    );
    s.core = Some("m".to_string());
    assert_eq!(
        s.thinking_fragment(None, None).unwrap(),
        Some(serde_json::json!({ "reasoning_effort": "low" })),
        "未指定会话档时用模型默认档"
    );
    assert_eq!(
        s.thinking_fragment(Some("m"), Some("off")).unwrap(),
        Some(serde_json::json!({ "thinking": { "type": "disabled" } })),
        "对象原样合并"
    );
    assert_eq!(
        s.thinking_fragment(Some("m"), Some("mo")).unwrap(),
        None,
        "空对象 = 不发字段"
    );
    let err = s.thinking_fragment(Some("m"), Some("max")).unwrap_err();
    assert!(err.contains("未登记思考档位 max"), "缺档要如实报：{}", err);
    assert!(s.thinking_supports(Some("m"), "high"));
    assert!(!s.thinking_supports(Some("m"), "max"));
    assert_eq!(
        crate::capabilities::registry::domain::providers::validate_thinking_catalog(
            &ThinkingCatalog::default()
        ),
        Ok(())
    );
}

/// 目的：登记时就拒掉目录外的档、悬空的默认档与被保留键的片段；删掉仍被模型使用的等级也要拒绝。
#[test]
pub(crate) fn thinking_registration_rejects_unknown_levels_defaults_and_reserved_keys() {
    use crate::capabilities::registry::api::ThinkingSpec;
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec!["[]".into()]));
    core.registry_mut()
        .provider_upsert("p1", "http://x", "k")
        .unwrap();
    let bad_level = ThinkingSpec {
        default: None,
        levels: BTreeMap::from([("ultra".to_string(), serde_json::json!("ultra"))]),
    };
    let err = core
        .registry_mut()
        .model_upsert(ModelDraft {
            thinking: bad_level,
            ..model_draft("m", "M", "m", "p1", "", 0)
        })
        .unwrap_err();
    assert!(err.contains("不在全局目录里"), "目录外的档要拒：{}", err);
    let bad_default = ThinkingSpec {
        default: Some("max".to_string()),
        levels: BTreeMap::from([("low".to_string(), serde_json::json!("low"))]),
    };
    let err = core
        .registry_mut()
        .model_upsert(ModelDraft {
            thinking: bad_default,
            ..model_draft("m", "M", "m", "p1", "", 0)
        })
        .unwrap_err();
    assert!(err.contains("默认档位"), "默认档必须被支持：{}", err);
    let bad_key = ThinkingSpec {
        default: None,
        levels: BTreeMap::from([("high".to_string(), serde_json::json!({ "messages": [] }))]),
    };
    let err = core
        .registry_mut()
        .model_upsert(ModelDraft {
            thinking: bad_key,
            ..model_draft("m", "M", "m", "p1", "", 0)
        })
        .unwrap_err();
    assert!(err.contains("不能覆盖请求协议字段"), "保留键要拒：{}", err);
    // 登记一个只用 high 的模型，然后把全局目录里的 high 删掉 = 拒绝并点名模型。
    core.registry_mut()
        .model_upsert(ModelDraft {
            thinking: ThinkingSpec {
                default: None,
                levels: BTreeMap::from([("high".to_string(), serde_json::json!("high"))]),
            },
            ..model_draft("m", "M", "m", "p1", "", 0)
        })
        .unwrap();
    let mut app = core.registry().app_settings();
    app.thinking.levels.retain(|l| l.id != "high");
    let err = core.registry_mut().set_app_settings(app).unwrap_err();
    assert!(err.contains("仍被模型 m 使用"), "删档要拒：{}", err);
}
