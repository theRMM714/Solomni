//! 目的：会话形状声明的装载契约：产品自带的两份表要合法，缺文件与坏规则都如实报。
//! 管：`YamlDecls::load` 的正反用例；表本身就是产品的 `systools/loops.yaml` 与 `systools/profiles.yaml`。
//! 不管：校验规则逐条（在 `session_model/domain/mechanism.rs` 与 `profile.rs` 的内联用例）；目录与读盘（在 `assets`）。
//! 联动：隔离根 `tests::scratch`；工具总表从 `YamlSystools` 读。

use super::*;

/// 目的：一份最小可用的流程段表（坏表用例要有它，才能走到会话类型表那一步）。
const MINIMAL_LOOPS: &str = "roles:\n  - id: solo\n    prompt: roles/executor\nloops:\n  - id: standard\n    cast:\n      - role: solo\n";

#[test]
fn the_shipped_table_covers_the_three_modes_and_its_roles_exist() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let decls = YamlDecls::new(assets_at(&root))
        .load()
        .expect("产品自带会话形状声明必须合法");
    for kind in ["single", "collab", "proxy"] {
        assert!(decls.known(kind), "缺会话类型：{}", kind);
    }
    let st = YamlSystools::new(assets_at(&root))
        .load()
        .expect("读工具总表");
    // 装载时就校验了两张表；这里再锁一遍段与工具总表的一致性。
    let problems =
        crate::units::business::conversation::session::api::loop_problems(decls.loops(), &st.tools);
    assert!(
        problems.is_empty(),
        "流程段与系统工具总表不一致：{:?}",
        problems
    );
}

#[test]
fn every_flow_segment_names_a_registered_implementation() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let decls = YamlDecls::new(assets_at(&root))
        .load()
        .expect("产品自带会话形状声明必须合法");
    // 段声明点名的实现必须有注册项：少一条就是「声明写得出、跑不起来」。
    let problems = decls
        .loops()
        .builder_problems(crate::units::business::collab::api::FLOW_BUILDERS);
    assert!(
        problems.is_empty(),
        "段声明的实现名没有注册：{:?}",
        problems
    );
}

#[test]
fn a_missing_or_broken_table_is_an_assembly_error() {
    let root = scratch("yaml-decls");
    let missing = YamlDecls::new(assets_at(&root.join("nope")))
        .load()
        .unwrap_err();
    assert!(missing.contains("流程段表读不了"), "{}", missing);

    let empty = root.join("empty");
    std::fs::create_dir_all(empty.join("systools")).expect("造空目录");
    assert!(
        YamlDecls::new(assets_at(&empty))
            .load()
            .unwrap_err()
            .contains("流程段表读不了"),
        "缺文件不能静默造默认"
    );

    let broken = root.join("broken");
    let dir = broken.join("systools");
    std::fs::create_dir_all(&dir).expect("造坏目录");
    std::fs::write(dir.join("loops.yaml"), MINIMAL_LOOPS).expect("造段表");
    std::fs::write(dir.join("profiles.yaml"), "profiles: [").expect("造坏表");
    assert!(
        YamlDecls::new(assets_at(&broken))
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
    let err = YamlDecls::new(assets_at(&broken)).load().unwrap_err();
    assert!(err.contains("流程没注册：ghost"), "{}", err);

    let _ = std::fs::remove_dir_all(&root);
}
