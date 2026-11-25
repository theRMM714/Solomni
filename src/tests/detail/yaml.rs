//! 目的：设置登记处 YAML 适配器的真实文件契约。
//! 管：默认值、保存、坏文件如实报错、非 NotFound 的读取错误原样透传。
//! 不管：登记处业务规则与内存替身。
//! 联动：隔离根 `tests::scratch`。

use super::*;

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
        let store =
            YamlSettingsStore::new(Arc::new(crate::units::home::service::HomeService::new(
                Box::new(crate::units::home::detail::FsHome::new(home.clone())),
            )));
        assert!(
            store.load().is_err(),
            "{} 是非 NotFound 读错误，必须报错而不是退回默认",
            name
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
