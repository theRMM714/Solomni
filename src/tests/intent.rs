//! 共享意图层（presentation/intent）的契约测试：点名、归并、唯一名、动作分发、编辑前置判断。
//! 规则只写一份，这里把它钉住——CLI 与 Web 因此不会各自漂移。

use super::doubles::module_of;
use super::{ops_with, single_work, slow_ops};
use crate::core::api::Output;
use crate::core::AgentInstance;
use crate::presentation::intent;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn instance(name: &str, modules: &[&str], model: Option<&str>) -> AgentInstance {
    AgentInstance {
        name: name.to_string(),
        transient: false,
        modules: modules.iter().map(|s| s.to_string()).collect(),
        model: model.map(|m| m.to_string()),
    }
}

#[test]
fn split_names_accepts_commas_and_whitespace() {
    assert_eq!(
        intent::split_names("甲, 乙，丙  丁"),
        vec!["甲", "乙", "丙", "丁"]
    );
    assert!(intent::split_names("   ").is_empty(), "全是空白 = 没有点名");
}

#[test]
fn merge_into_one_dedups_modules_and_keeps_order() {
    let a = instance("甲", &["m1", "m2"], Some("x"));
    let b = instance("乙", &["m2", "m3"], Some("y"));
    let merged = intent::merge_into_one(&[a, b], "组合");
    assert_eq!(merged.modules, vec!["m1", "m2", "m3"], "模块去重且保序");
    assert!(merged.transient, "并出来的「组合」是临时 agent");
    assert_eq!(merged.model, None, "组合不指定模型（用核心默认）");
    assert_eq!(merged.name, "组合");
}

#[test]
fn pick_agents_guides_on_empty_registry_and_names_the_unknown() {
    let (_h, ops) = ops_with(vec![module_of("a")], Vec::new());
    assert_eq!(
        intent::pick_agents(&ops, &["甲".to_string()]).unwrap_err(),
        intent::NO_AGENTS,
        "空登记处给引导"
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

    let views = intent::all_views(&ops).expect("读登记处");
    assert_eq!(intent::as_instances(&views).len(), 1, "视图 → 实例不丢项");
}

#[test]
fn unique_work_name_falls_back_and_appends_a_suffix() {
    let (_h, ops) = ops_with(vec![module_of("a")], Vec::new());
    assert_eq!(
        intent::unique_work_name(&ops, "", "single").expect("缺省名"),
        "single"
    );
    assert_eq!(
        intent::unique_work_name(&ops, "  取名  ", "single").expect("去空白"),
        "取名"
    );
    ops.sessions
        .create_work(single_work("w", &["a"]))
        .expect("建会话");
    assert_eq!(
        intent::unique_work_name(&ops, "w", "x").expect("重名加尾号"),
        "w-2"
    );
    ops.sessions
        .create_work(single_work("w-2", &["a"]))
        .expect("建会话");
    assert_eq!(
        intent::unique_work_name(&ops, "w", "x").expect("再重名"),
        "w-3"
    );
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
    // 回档给的是「完整重放」（线格式事件数组），不是事件批；keep_id = 0 表示一行不留。
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

#[test]
fn editing_is_refused_while_a_session_is_generating() {
    let (_h, ops, _ticks) = slow_ops(vec![module_of("a")]);
    let sid = ops
        .sessions
        .create_work(single_work("w", &["a"]))
        .expect("建会话")
        .0
        .sid;
    intent::ensure_editable(&ops, &sid).expect("没在生成时可以改");

    let worker = {
        let sessions = Arc::clone(&ops.sessions);
        let sid = sid.clone();
        std::thread::spawn(move || sessions.say(&sid, "慢慢来", Output::Stream))
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ops.sessions.is_running(&sid) {
        assert!(Instant::now() < deadline, "生成没有启动");
        std::thread::sleep(Duration::from_millis(5));
    }
    let err = intent::ensure_editable(&ops, &sid).unwrap_err();
    assert!(err.contains("正在生成中"), "生成中必须拒绝改配置：{}", err);

    assert!(ops.sessions.stop(&sid), "停掉它");
    let _ = worker.join();
    intent::ensure_editable(&ops, &sid).expect("收尾后可以改");
}

#[test]
fn edit_session_reports_core_errors_verbatim() {
    let (_h, ops) = ops_with(vec![module_of("a")], Vec::new());
    let edit = serde_json::from_value::<crate::core::SessionEdit>(serde_json::json!({
        "agents": [], "tier": "host", "base": null, "pins": {}, "net": false
    }))
    .expect("编辑体");
    assert!(
        intent::edit_session(&ops, "没这个会话", edit).is_err(),
        "无此会话要如实报错"
    );
}
