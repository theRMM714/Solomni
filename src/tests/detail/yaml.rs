//! 目的：设置登记处 YAML 适配器的真实文件契约。
//! 管：默认值、保存、坏文件如实报错、非 NotFound 的读取错误原样透传。
//! 不管：登记处业务规则与内存替身。
//! 联动：隔离根 `tests::scratch`。

use super::*;

#[test]
fn yaml_settings_store_defaults_saves_and_reports_malformed_files() {
    let root = scratch("yaml-settings");
    let home = root.join(".home");
    let paths = || {
        (
            home.join("providers.yaml"),
            home.join("models.yaml"),
            home.join("settings.yaml"),
            home.join("agents.yaml"),
        )
    };
    let (p, m, s, a) = paths();
    let store = YamlSettingsStore::new(p, m, s, a);

    // 四份文件都不存在 = 空登记处 + 默认设置（不是错误）。
    let empty = store.load().expect("缺文件必须取默认，不报错");
    assert!(empty.providers.is_empty() && empty.models.is_empty() && empty.core.is_none());
    assert!(empty.app.streaming && !empty.app.sandbox.policy.windows.write_acl);
    assert!(empty.agents.is_empty());

    // 未知字段 / 拼错键 = 配置非法（拒绝，不读取、不静默忽略）。
    std::fs::create_dir_all(&home).expect("建 .home");
    std::fs::write(home.join("settings.yaml"), "fence_write: true\n").expect("造非法配置");
    let err = store.load().unwrap_err();
    assert!(
        err.contains("settings.yaml 配置非法") && err.contains("unknown field"),
        "{}",
        err
    );
    std::fs::remove_file(home.join("settings.yaml")).expect("删非法配置");

    let mut want = Settings::default();
    want.providers.insert(
        "p".to_string(),
        Provider {
            base_url: "http://x".to_string(),
            api_key: "sk-secret".to_string(),
        },
    );
    want.models.insert(
        "m".to_string(),
        crate::capabilities::registry::api::ModelEntry {
            name: "M".to_string(),
            api_model: "m".to_string(),
            provider: "p".to_string(),
            note: String::new(),
            tools: crate::capabilities::llm::api::ToolMode::Envelope,
            context: 32_000,
            thinking: Default::default(),
        },
    );
    want.core = Some("m".to_string());
    want.app.streaming = false;
    // Windows 上 require + write_acl=false 是非法组合；这条只验落盘/读回，先给授权。
    want.app.sandbox.policy.windows.write_acl = true;
    want.agents.insert(
        "甲".to_string(),
        crate::capabilities::registry::domain::agents::Agent {
            modules: vec!["a".to_string()],
            model: None,
            note: "n".to_string(),
            chara: "人设".to_string(),
        },
    );
    store.save(&want).expect("保存成功");
    let got = store.load().expect("读回");
    assert_eq!(
        got.providers.get("p").map(|p| p.base_url.as_str()),
        Some("http://x")
    );
    assert_eq!(
        got.providers.get("p").map(|p| p.api_key.as_str()),
        Some("sk-secret"),
        "密钥落 providers.yaml"
    );
    assert_eq!(got.core.as_deref(), Some("m"));
    assert!(!got.app.streaming && got.agents.contains_key("甲"));
    assert_eq!(
        got.agents["甲"].chara, "人设",
        "人设随 agents.yaml 落盘/读回"
    );

    // unix：密钥文件 0600。
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(home.join("providers.yaml"))
            .expect("stat")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "密钥文件必须是 0600");
    }

    // 非法 yaml：哪一份坏了要说清是哪一份（不静默兜底）。
    std::fs::write(home.join("models.yaml"), "models: [不是映射").expect("造坏文件");
    let err = store.load().unwrap_err();
    assert!(err.contains("models.yaml 非法"), "{}", err);

    // 工具调用形态是**封闭枚举**：envelope / native 之外的写法一律拒收（不猜、不静默降级）
    std::fs::write(
        home.join("models.yaml"),
        "models:\n  m:\n    name: M\n    api_model: m\n    provider: p\n    tools: auto\n",
    )
    .expect("造非法形态");
    let err = store.load().unwrap_err();
    assert!(
        err.contains("models.yaml 非法"),
        "非法形态要拒收并说明是哪个文件：{}",
        err
    );

    // 合法形态能落盘也能读回（envelope 是缺省，native 要显式写）
    std::fs::write(
        home.join("models.yaml"),
        "models:\n  m:\n    name: M\n    api_model: m\n    provider: p\n    tools: native\n",
    )
    .expect("造合法形态");
    let back = store.load().expect("合法形态必须能读回");
    assert_eq!(
        back.models.get("m").map(|m| m.tools),
        Some(crate::capabilities::llm::api::ToolMode::Native),
        "注册表里的形态要原样读回"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn yaml_settings_store_propagates_non_not_found_read_errors() {
    // 读错误不止"文件不存在"：拿目录当文件读会得到非 NotFound 的错误，
    // 这时必须如实报错，不能悄悄退回默认值（否则"配置读不到"会变成"没配置"）。
    for name in [
        "providers.yaml",
        "models.yaml",
        "settings.yaml",
        "agents.yaml",
    ] {
        let root = scratch(&format!("yaml-settings-readerr-{}", name));
        let home = root.join(".home");
        std::fs::create_dir_all(home.join(name)).expect("拿目录占住这个路径");
        let store = YamlSettingsStore::new(
            home.join("providers.yaml"),
            home.join("models.yaml"),
            home.join("settings.yaml"),
            home.join("agents.yaml"),
        );
        assert!(
            store.load().is_err(),
            "{} 是非 NotFound 读错误，必须报错而不是退回默认",
            name
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}

