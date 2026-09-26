//! 呈现层的共享件（`presentation/intent`）测试：输入解析、点名、动作分发。
//! **规则不在这一层**：点名归 `registry`、命名与「生成中禁改」归 `session`/`core`、
//! 组合语义归 `core::create_work`——它们的测试在 `tests/api.rs`（入站契约）里。

use super::doubles::module_of;
use super::{ops_with, single_work};
use crate::core::api::Output;
use crate::presentation::intent;

#[test]
fn split_names_accepts_commas_and_whitespace() {
    assert_eq!(
        intent::split_names("甲, 乙，丙  丁"),
        vec!["甲", "乙", "丙", "丁"]
    );
    assert!(intent::split_names("   ").is_empty(), "全是空白 = 没有点名");
}

#[test]
fn pick_agents_guides_on_empty_registry_and_names_the_unknown() {
    let (_h, ops) = ops_with(vec![module_of("a")], Vec::new());
    assert_eq!(
        intent::pick_agents(&ops, &["甲".to_string()]).unwrap_err(),
        intent::NO_AGENTS,
        "空登记处给引导（文案是呈现层的事）"
    );

    ops.registry
        .upsert_agent("甲", &["a".to_string()], "", "")
        .expect("建 agent");
    let picked = intent::pick_agents(&ops, &["甲".to_string()]).expect("点名成功");
    assert_eq!(picked.len(), 1);
    assert_eq!(picked[0].name, "甲");
    assert!(!picked[0].transient, "点名已存 agent 不是临时的");
    assert_eq!(picked[0].modules, vec!["a".to_string()]);

    let err = intent::pick_agents(&ops, &["乙".to_string()]).unwrap_err();
    assert!(
        err.contains("无此 agent：乙") && err.contains("甲"),
        "报错要列出可选项：{}",
        err
    );
    assert!(intent::pick_agents(&ops, &[])
        .unwrap_err()
        .contains("没有点名"));
}

#[test]
fn act_dispatches_to_the_two_result_shapes() {
    let (_h, ops) = ops_with(vec![module_of("a")], Vec::new());
    let sid = ops
        .sessions
        .create_work(single_work("w", &["a"]))
        .expect("建会话")
        .0
        .sid;

    match intent::act(&ops, &sid, intent::Action::Say("你好"), Output::Final).expect("说一句")
    {
        intent::Acted::Advanced(adv) => assert!(adv.head > 0, "生成类只回事件台头部序号"),
        intent::Acted::Replayed(_) => panic!("说一句不该给重放"),
    }
    match intent::act(&ops, &sid, intent::Action::Rewind(0), Output::Final).expect("回档") {
        intent::Acted::Replayed(events) => {
            assert!(events.iter().all(|e| e.is_object()), "重放是线格式事件数组")
        }
        intent::Acted::Advanced(_) => panic!("回档不该给事件批"),
    }
    assert!(
        intent::act(&ops, "没这个会话", intent::Action::Say("x"), Output::Final).is_err(),
        "无此会话如实报错"
    );
}
