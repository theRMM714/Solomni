//! 目的：会话类型表的装载契约：产品自带的那份要合法，缺文件与坏规则都如实报。
//! 管：`YamlProfiles::load` 的正反用例；表本身就是 product 的 `systools/profiles.yaml`。
//! 不管：校验规则逐条（在 `session/domain/profile.rs` 的内联用例）。
//! 联动：隔离根 `tests::scratch`；角色表从 `YamlSystools` 读。

use super::*;

#[test]
fn the_shipped_table_covers_the_three_modes_and_its_roles_exist() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let profiles = YamlProfiles::new(root.join("systools"))
        .load()
        .expect("产品自带会话类型表必须合法");
    let ids = profiles.ids();
    for mode in ["single", "collab", "proxy"] {
        assert!(ids.iter().any(|i| i == mode), "缺会话形态：{}", mode);
    }
    let st = YamlSystools::new(root.join("systools"))
        .load()
        .expect("读角色表");
    let known: Vec<String> = st.roles.keys().cloned().collect();
    profiles
        .check_roles(&known)
        .expect("会话类型引用的角色必须在角色表里");
}

#[test]
fn a_missing_or_broken_table_is_an_assembly_error() {
    let root = scratch("yaml-profiles");
    let missing = YamlProfiles::new(root.join("nope")).load().unwrap_err();
    assert!(missing.contains("会话类型表读不了"), "{}", missing);

    let empty = root.join("empty");
    std::fs::create_dir_all(&empty).expect("造空目录");
    assert!(
        YamlProfiles::new(empty)
            .load()
            .unwrap_err()
            .contains("会话类型表读不了"),
        "缺文件不能静默造默认"
    );

    let broken = root.join("broken");
    std::fs::create_dir_all(&broken).expect("造坏目录");
    std::fs::write(broken.join("profiles.yaml"), "profiles: [").expect("造坏表");
    assert!(
        YamlProfiles::new(broken.clone())
            .load()
            .unwrap_err()
            .contains("会话类型表非法"),
        "语法坏要如实报"
    );

    // 语法对、规则不对：同样装配失败，文案点名文件。
    std::fs::write(
        broken.join("profiles.yaml"),
        "profiles:\n  - id: single\n    agents: one\n    roles: [solo]\n    mechanisms: []\n",
    )
    .expect("造缺类型的表");
    let err = YamlProfiles::new(broken).load().unwrap_err();
    assert!(err.contains("缺一个会话形态：collab"), "{}", err);

    let _ = std::fs::remove_dir_all(&root);
}
