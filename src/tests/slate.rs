//! 拟名单业务（`capabilities/slate/`）的用例：**推荐名单**与**代拟名单**共用同一条 `slate` 协议。
//!
//! 驱动入口是协调业务的分发面（`Conductor::suggest_models`，前端经那条路由进来）；
//! 装配与替身来自 `doubles`。协作会话里的代拟路径（`Pending::ConfirmSlate`）见 `tests/collab.rs`。

use super::prelude::*;

#[test]
pub(crate) fn suggest_models_recommends_agents() {
    // 不存在的模块 / 不存在的模型 / 重复占用的模块一律拒收。
    let script = "{\"type\":\"tool\",\"name\":\"slate\",\"args\":{\"picks\":[{\"name\":\"甲\",\"modules\":[\"a\"],\"model\":\"m\",\"why\":\"对口\"},{\"name\":\"乙\",\"modules\":[\"b\"],\"model\":\"m\",\"why\":\"补位\"},{\"name\":\"鬼\",\"modules\":[\"ghost\"],\"model\":\"m\",\"why\":\"模块不存在\"},{\"name\":\"丙\",\"modules\":[\"a\"],\"model\":\"nope\",\"why\":\"模型不存在\"},{\"name\":\"丁\",\"modules\":[\"a\"],\"model\":\"m\",\"why\":\"重复占模块\"}]}}".to_string();
    let core = core_with(
        vec![module_of("a"), module_of("b")],
        gw(BTreeMap::new(), vec![script.clone()]),
    );
    // 协作：两个独立 agent，各带自己的模块与模型。
    let (collab, rows) = core.suggest_models("做个东西", WorkMode::Collab).unwrap();
    assert!(!rows.is_empty(), "核心这一趟的行必须交出来（推给系统会话）");
    assert_eq!(collab.len(), 2);
    assert_eq!(collab[0].name, "甲");
    assert_eq!(collab[0].modules, vec!["a".to_string()]);
    assert_eq!(collab[1].modules, vec!["b".to_string()]);
    assert_eq!(collab[0].model, "m");
    assert_eq!(collab[0].why, "对口");
    // 单 agent：多条推荐 → 并成一个临时 agent（并过的不是任何单个已存 agent，故 reuse=false）。
    let (merged, _rows) = core.suggest_models("做个东西", WorkMode::Single).unwrap();
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].modules, vec!["a".to_string(), "b".to_string()]);
    assert!(!merged[0].reuse, "并出来的 agent 不是复用项");
}

#[test]
pub(crate) fn suggest_models_single_mode_keeps_lone_pick_as_is() {
    // 只给一条 → 原样采纳（模块数与 reuse 都保持它自己的，不裁模块、不改 reuse）。
    let core = core_with(vec![module_of("a"), module_of("b")], gw(BTreeMap::new(), vec![
        "{\"type\":\"tool\",\"name\":\"slate\",\"args\":{\"picks\":[{\"name\":\"全能\",\"modules\":[\"a\",\"b\"],\"model\":\"m\",\"why\":\"一个 AI 全包\"}]}}".to_string(),
    ]));
    let (out, _rows) = core.suggest_models("做个东西", WorkMode::Single).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].name, "全能");
    assert_eq!(
        out[0].modules,
        vec!["a".to_string(), "b".to_string()],
        "单 agent 不再被裁到一个模块"
    );
    assert!(!out[0].reuse);
    // 复用项也要原样保留 reuse 标记
    let mut reuse = core_with(
        vec![module_of("a")],
        gw(
            BTreeMap::new(),
            vec![
                "{\"type\":\"tool\",\"name\":\"slate\",\"args\":{\"picks\":[{\"agent\":\"调研\",\"why\":\"正好\"}]}}"
                    .to_string(),
            ],
        ),
    );
    agent_upsert(&mut reuse, "调研", &["a"], "m", "").unwrap();
    let (got, _rows) = reuse.suggest_models("做个东西", WorkMode::Single).unwrap();
    assert_eq!(got.len(), 1);
    assert!(got[0].reuse, "一条复用项必须保留 reuse");
    assert_eq!(got[0].model, "m");
}

#[test]
pub(crate) fn suggest_models_reuses_stored_agent_without_suggesting_model() {
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec![
        // 只有复用项（模型与模块都取登记处自己的）；幽灵项应被拒收。
        "{\"type\":\"tool\",\"name\":\"slate\",\"args\":{\"picks\":[{\"agent\":\"调研\",\"why\":\"正好用得上\"},{\"agent\":\"幽灵\",\"why\":\"不在登记处\"}]}}".to_string(),
    ]));
    agent_upsert(&mut core, "调研", &["a"], "m", "说明").unwrap();
    let (out, _rows) = core.suggest_models("做个东西", WorkMode::Collab).unwrap();
    assert_eq!(out.len(), 1, "非法的复用项必须被拒收");
    assert!(out[0].reuse, "复用项要如实标记");
    assert_eq!(out[0].name, "调研");
    assert_eq!(out[0].modules, vec!["a".to_string()]);
    assert_eq!(out[0].model, "m", "模型取登记处里那个，核心不代拟");
    // 全部拒收 → 明确报错（不悄悄给个空名单）
    let mut empty = core_with(
        vec![module_of("a")],
        gw(
            BTreeMap::new(),
            vec![
                "{\"type\":\"tool\",\"name\":\"slate\",\"args\":{\"picks\":[{\"agent\":\"幽灵\",\"why\":\"不在登记处\"}]}}"
                    .to_string(),
            ],
        ),
    );
    agent_upsert(&mut empty, "调研", &["a"], "m", "").unwrap();
    assert!(empty
        .suggest_models("做个东西", WorkMode::Collab)
        .unwrap_err()
        .contains("没有可用结果"));
}
