//! CLI 前端（`cli`）的测试：**传输侧**的输入解析与点名包装。
//! **业务规则不在这里**：点名归 `registry`、命名与「生成中禁改」归 `session`/`conductor`、
//! 组合语义归 `conductor::create_session`、动作分发归 `ActionOps::act`——它们的测试在 `tests/api.rs`。

use super::doubles::module_of;
use super::ops_with;
use crate::capabilities::session::api::Pending;
use crate::kernel::api::Tier;
use crate::presentation::cli;

/// 模块工具名两种写法都收：`<模块id>.<工具名>` 与完整的 `module.<模块id>.<工具名>`。
#[test]
fn module_action_id_accepts_both_forms() {
    assert_eq!(
        cli::module_action_id("toolbox.read_txt"),
        "module.toolbox.read_txt"
    );
    assert_eq!(
        cli::module_action_id("module.toolbox.read_txt"),
        "module.toolbox.read_txt"
    );
}

/// 建会话在 CLI 上走**动作表**：同一份声明、同一处授权（未知形态在分发处被拒）。
#[test]
fn cli_session_creation_goes_through_the_action_table() {
    let (_h, ops) = ops_with(vec![module_of("a")], Vec::new());
    ops.registry
        .upsert_agent("甲", &["a".to_string()], "", "")
        .expect("建 agent");
    let picked = cli::pick_agents(&ops, &["甲".to_string()]).expect("点名");
    let sid = cli::create_session_action(&ops, "cli-1", "single", &picked, None, Tier::Host)
        .expect("走动作表建会话");
    assert_eq!(sid, "cli-1");
    assert!(
        cli::create_session_action(&ops, "cli-2", "乱来", &picked, None, Tier::Host).is_err(),
        "未知形态在分发处如实拒绝"
    );
}

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

/// 目的：队首之后还在等的那几张要如实列成文本（谁在等、前面还排着几条，一行一张）。
#[test]
fn waiting_list_is_built_line_by_line() {
    // 等待者用**生产那一条派生**造（Pending → 等待者），不手搓结构体。
    let ask = Pending::Ask {
        member: "甲".to_string(),
        question: "选哪个？".to_string(),
    };
    let begin = Pending::ConfirmBegin;
    assert!(cli::waiting_lines(&[]).is_empty(), "没人等就一行都不占");
    let lines = cli::waiting_lines(&[ask.waiter("d2", ""), begin.waiter("d3", "")]);
    assert_eq!(lines.len(), 3, "一条头 + 每位等待者一行：{:?}", lines);
    assert!(
        lines[0].contains("排着 2 张"),
        "要说清前面还排着几条：{:?}",
        lines[0]
    );
    assert!(
        lines[1].contains("甲") && lines[1].contains("在等"),
        "等待者要带名字与它在等什么：{:?}",
        lines[1]
    );
    assert!(lines[2].contains("核心"), "按先来后到：{:?}", lines[2]);
}
