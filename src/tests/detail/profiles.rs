//! 目的：会话类型表的装载契约：产品自带的那份要合法，缺文件与坏规则都如实报。
//! 管：`YamlProfiles::load` 的正反用例；表本身就是 product 的 `systools/profiles.yaml`。
//! 不管：校验规则逐条（在 `session/domain/profile.rs` 的内联用例）；目录与读盘（在 `assets`）。
//! 联动：隔离根 `tests::scratch`；机制席位从 `YamlSystools` 读。

use super::*;

#[test]
fn the_shipped_table_covers_the_three_modes_and_its_roles_exist() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let profiles = YamlProfiles::new(assets_at(&root))
        .load()
        .expect("产品自带会话类型表必须合法");
    for kind in ["single", "collab", "proxy"] {
        assert!(profiles.known(kind), "缺会话类型：{}", kind);
    }
    let st = YamlSystools::new(assets_at(&root))
        .load()
        .expect("读工具总表");
    // 会话类型表在装载时就校验了流程段与流程；这里再锁一遍角色面与工具总表的一致性。
    let problems = crate::units::business::conversation::session::api::loop_problems(&st.tools);
    assert!(
        problems.is_empty(),
        "角色面与系统工具总表不一致：{:?}",
        problems
    );
}

#[test]
fn a_missing_or_broken_table_is_an_assembly_error() {
    let root = scratch("yaml-profiles");
    let missing = YamlProfiles::new(assets_at(&root.join("nope")))
        .load()
        .unwrap_err();
    assert!(missing.contains("会话类型表读不了"), "{}", missing);

    let empty = root.join("empty");
    std::fs::create_dir_all(empty.join("systools")).expect("造空目录");
    assert!(
        YamlProfiles::new(assets_at(&empty))
            .load()
            .unwrap_err()
            .contains("会话类型表读不了"),
        "缺文件不能静默造默认"
    );

    let broken = root.join("broken");
    let dir = broken.join("systools");
    std::fs::create_dir_all(&dir).expect("造坏目录");
    std::fs::write(dir.join("profiles.yaml"), "profiles: [").expect("造坏表");
    assert!(
        YamlProfiles::new(assets_at(&broken))
            .load()
            .unwrap_err()
            .contains("会话类型表非法"),
        "语法坏要如实报"
    );

    // 语法对、规则不对：同样装配失败，文案点名文件与出问题的那一项。
    std::fs::write(
        dir.join("profiles.yaml"),
        "profiles:\n  - id: single\n    flows: [ghost]\n",
    )
    .expect("造没注册流程的表");
    let err = YamlProfiles::new(assets_at(&broken)).load().unwrap_err();
    assert!(err.contains("流程没注册：ghost"), "{}", err);

    let _ = std::fs::remove_dir_all(&root);
}
