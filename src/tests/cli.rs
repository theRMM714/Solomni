//! CLI 前端（`cli`）的测试：**传输侧**的输入解析与点名包装。
//! **业务规则不在这里**：点名归 `registry`、命名与「生成中禁改」归 `session`/`conductor`、
//! 组合语义归 `conductor::create_work`、动作分发归 `SessionOps::act`——它们的测试在 `tests/api.rs`。

use super::doubles::module_of;
use super::ops_with;
use crate::presentation::cli;

#[test]
fn split_names_accepts_commas_and_whitespace() {
    assert_eq!(
        cli::split_names("甲, 乙，丙  丁"),
        vec!["甲", "乙", "丙", "丁"]
    );
    assert!(cli::split_names("   ").is_empty(), "全是空白 = 没有点名");
}

#[test]
fn pick_agents_guides_on_empty_registry_and_names_the_unknown() {
    let (_h, ops) = ops_with(vec![module_of("a")], Vec::new());
    assert_eq!(
        cli::pick_agents(&ops, &["甲".to_string()]).unwrap_err(),
        cli::NO_AGENTS,
        "空登记处给引导（文案是 CLI 自己的说法）"
    );

    ops.registry
        .upsert_agent("甲", &["a".to_string()], "", "")
        .expect("建 agent");
    let picked = cli::pick_agents(&ops, &["甲".to_string()]).expect("点名成功");
    assert_eq!(picked.len(), 1);
    assert_eq!(picked[0].name, "甲");
    assert!(!picked[0].transient, "点名已存 agent 不是临时的");
    assert_eq!(picked[0].modules, vec!["a".to_string()]);

    let err = cli::pick_agents(&ops, &["乙".to_string()]).unwrap_err();
    assert!(
        err.contains("无此 agent：乙") && err.contains("甲"),
        "报错要列出可选项：{}",
        err
    );
    assert!(cli::pick_agents(&ops, &[])
        .unwrap_err()
        .contains("没有点名"));
}
