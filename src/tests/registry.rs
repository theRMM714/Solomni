//! 目的：钉供应商、模型、agent、设置与通道解析的不变式。
//! 管：设置解析通道、供应商生命周期与密钥不泄、上下文 0 回落 128K、核心默认守卫、目录解析、端点候选回落、探测只回写结论、保留目录名。
//! 不管：HTTP 入站（routes.rs）。
//! 联动：src/units/registry/。

use super::builders::*;
use super::prelude::*;
use crate::units::session_model::api::{MEETING, SINGLE};

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
            tools: crate::units::pure::registry_model::api::ToolMode::Native,
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
    // 缺省 = native（工具调用默认走原生）；模型视图也如实带出来
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
            .find(|v| v.id == "d")
            .map(|v| v.tools),
        Some(crate::units::pure::registry_model::api::ToolMode::Native),
        "不填 tools 的模型缺省走原生"
    );
    assert_eq!(
        s.model_views()
            .iter()
            .find(|v| v.id == "m")
            .map(|v| v.tools),
        Some(crate::units::pure::registry_model::api::ToolMode::Native),
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
        // 展示文案由呈现层拼（runtime 不再提供 CLI 行），"密钥永不出现"这条红线两处都要成立。
        let shown = format!("{}  {}", v.id, v.base_url);
        assert!(!shown.contains("sk-密钥XYZ"), "视图出现密钥：{}", shown);
        assert!(!format!("{:?}", v).contains("sk-密钥XYZ"));
    }
    // 仍被模型引用时拒绝删除供应商（不静默级联）
    core.registry_mut()
        .model_upsert(model_draft("m1", "M", "api-m", "p1", "快", 0))
        .unwrap();
    assert!(
        core.registry_mut()
            .provider_remove("p1")
            .unwrap_err()
            .contains("仍被模型引用")
    );
    assert!(core.registry_mut().model_remove("m1").unwrap());
    assert!(core.registry_mut().provider_remove("p1").unwrap());
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
    assert!(
        core.registry()
            .discover_models("ghost")
            .unwrap_err()
            .contains("无此供应商")
    );
    // 引用了不存在的供应商 → 拒绝登记
    assert!(
        core.registry_mut()
            .model_upsert(model_draft("bad", "B", "b", "ghost", "", 0))
            .unwrap_err()
            .contains("无此供应商")
    );
    // 核心默认模型不可删；换默认后旧的可删
    assert!(
        core.registry_mut()
            .model_remove("m")
            .unwrap_err()
            .contains("核心默认模型")
    );
    core.registry_mut()
        .model_upsert(model_draft("m2", "M2", "api-m2", "p2", "", 0))
        .unwrap();
    assert!(core.registry_mut().core_set_model("m2").unwrap());
    assert!(core.registry_mut().model_remove("m").unwrap());
}

#[test]
pub(crate) fn agent_meta_is_the_single_conversion_from_a_registry_view() {
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec!["[]".into()]));
    agent_upsert_chara(&mut core, "调研", &["a"], "m", "说明", " 严谨 ").unwrap();
    let view = core.registry().agent_views().remove(0);
    let meta = crate::units::pure::registry_model::api::agent_meta(&view);
    assert_eq!(meta.name, view.name);
    assert!(!meta.transient, "登记处来的 agent 不是临时 agent");
    assert_eq!(meta.modules, view.modules);
    assert_eq!(meta.model, view.model);
    assert_eq!(meta.chara, view.chara);
}

#[test]
pub(crate) fn agent_crud_and_work_with_agents() {
    let mut core = core_with(
        vec![module_of("a"), module_of("b")],
        gw(BTreeMap::new(), vec!["[]".into()]),
    );
    assert!(
        agent_upsert(&mut core, "", &["a"], "", "")
            .unwrap_err()
            .contains("不能为空")
    );
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
    assert!(
        agent_upsert(&mut core, "x", &["ghost"], "", "")
            .unwrap_err()
            .contains("无此模块")
    );
    assert!(
        agent_upsert(&mut core, "x", &["a"], "nope", "")
            .unwrap_err()
            .contains("无此模型")
    );
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
        flows: None,
        freshness: None,
        name: "w".to_string(),
        kind: SINGLE.to_string(),
        agents: vec![AgentMeta {
            name: "调研".to_string(),
            transient: false,
            modules: vec!["a".to_string(), "b".to_string()],
            model: Some("m".to_string()),
            chara: "严谨的资料整理者".to_string(),
        }],
        task: None,
        delegate: false,
        permission: None,
        tier: crate::units::pure::exec_model::api::Tier::Host,
        guest: None,
        workspace: None,
    };
    let opened = core.create_work(spec).unwrap();
    assert_eq!(opened.agents, vec!["调研".to_string()]);
    let meta = core.lifecycle.history_open("w").unwrap().0;
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
        flows: None,
        freshness: None,
        name: "cross".to_string(),
        kind: MEETING.to_string(),
        agents: vec![
            AgentMeta {
                name: "x".to_string(),
                transient: true,
                modules: vec!["a".to_string()],
                model: None,
                chara: String::new(),
            },
            AgentMeta {
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
        tier: crate::units::pure::exec_model::api::Tier::Host,
        guest: None,
        workspace: None,
    };
    let opened = core.create_work(cross).expect("同一模块可跨 agent 共用");
    assert_eq!(opened.agents, vec!["x".to_string(), "y".to_string()]);
    let (meta, _) = core.lifecycle.history_open("cross").expect("打开会话");
    assert_eq!(meta.modules, vec!["a".to_string()], "meta.modules 去重");

    assert!(core.registry_mut().agent_remove("调研").unwrap());
    assert!(core.registry().agent_views().is_empty());
}

// ---------- 模块清单（内存来源） ----------
#[test]
pub(crate) fn a_probe_writes_back_only_conclusive_results() {
    use crate::units::llm::api::ProbeOutcome;
    use crate::units::pure::registry_model::api::ToolMode;
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
    let mode_of = |core: &Core| {
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
        "替身登记成信封；探测前保持原值"
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

/// 目的：目录保留名校验（名单从 `systools/names.yaml` 注入）：大小写不敏感，普通名字放行。
#[test]
pub(crate) fn reserved_directory_names_are_rejected_case_insensitively() {
    let reserved = vec!["work".to_string(), "children".to_string()];
    assert!(
        crate::units::pure::registry_model::api::check_reserved("children", &reserved).is_err()
    );
    assert!(
        crate::units::pure::registry_model::api::check_reserved("CHILDREN", &reserved).is_err(),
        "Windows 文件系统不区分大小写"
    );
    assert!(crate::units::pure::registry_model::api::check_reserved("甲", &reserved).is_ok());
}

/// 目的：登记时就拒掉目录外的档、悬空的默认档与被保留键的片段；删掉仍被模型使用的等级也要拒绝。
#[test]
pub(crate) fn thinking_registration_rejects_unknown_levels_defaults_and_reserved_keys() {
    use crate::units::pure::registry_model::api::ThinkingSpec;
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
    // Windows 上 require 围栏必须授权写 ACL；这条只验思考目录的引用检查。
    app.sandbox.policy.windows.write_acl = true;
    let err = core.registry_mut().set_app_settings(app).unwrap_err();
    assert!(err.contains("仍被模型 m 使用"), "删档要拒：{}", err);
}
