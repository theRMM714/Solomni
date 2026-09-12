//! mock 测试：不依赖网络与真实密钥。假模型脚本化应答，全链路可重复。
//! 覆盖三层：信封/登记处（适配层）、模块扫描（装配层）、协作引擎与核心会话（核心层）。

use crate::envelope;
use crate::kernel::{Core, Pending, SessionEvent};
use crate::model::{Chat, FakeChat};
use crate::module;
use crate::orchestrator::{Discussion, Execution, Member, TurnOut, MAX_ROUNDS};

// ---- 信封：唯一的机器锚 ----

#[test]
fn envelope_parses_clean_verbs() {
    for (raw, want) in [
        (FakeChat::say("你好"), envelope::Verb::Say),
        (FakeChat::verb_json("agree", "同意"), envelope::Verb::Agree),
        (FakeChat::verb_json("leave", "先走一步"), envelope::Verb::Leave),
        (FakeChat::verb_json("ask", "用哪个方案？"), envelope::Verb::Ask),
    ] {
        let r = envelope::parse(&raw);
        assert_eq!(r.verb, want);
        assert!(!r.degraded);
    }
}

#[test]
fn envelope_degrades_to_say_on_garbage() {
    let r = envelope::parse("我觉得可以直接干，不用讨论了。");
    assert_eq!(r.verb, envelope::Verb::Say);
    assert!(r.degraded);
    assert!(r.text.contains("直接干"));
}

#[test]
fn envelope_extracts_json_from_prose() {
    let raw = format!("我的看法：{}", FakeChat::verb_json("agree", "同意方案一"));
    let r = envelope::parse(&raw);
    assert_eq!(r.verb, envelope::Verb::Agree);
    assert!(!r.degraded);
}

// ---- 供应商登记处：优先级链 ----

#[test]
fn providers_resolve_priority_chain() {
    use crate::providers::{Provider, Registry};
    let mut reg = Registry::default();
    reg.providers.insert("a".into(), Provider { kind: "llm".into(), base_url: "http://a".into(), api_key: "k-a".into(), models: vec![] });
    reg.providers.insert("b".into(), Provider { kind: "llm".into(), base_url: "http://b".into(), api_key: "k-b".into(), models: vec![] });
    reg.default = Some("b".into());
    // 模块当前选择 > 清单默认 > 全局默认。
    let (id, _) = reg.resolve(Some("a"), Some("b")).unwrap();
    assert_eq!(id, "a");
    let (id, _) = reg.resolve(None, Some("b")).unwrap();
    assert_eq!(id, "b");
    // 登记处没有的 id：回落下一个层级。
    let (id, _) = reg.resolve(Some("ghost"), None).unwrap();
    assert_eq!(id, "b");
    // 全都没有：None（如实告知，不静默造）。
    let mut empty = Registry::default();
    assert!(empty.resolve(Some("x"), Some("y")).is_none());
    empty.default = None;
    assert!(empty.resolve(None, None).is_none());
}

// ---- 模块扫描：清单即事实 ----

#[test]
fn scan_accepts_valid_and_rejects_bad_with_reasons() {
    let dir = std::env::temp_dir().join(format!("solomni-scan-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let good = dir.join("research");
    std::fs::create_dir_all(&good).unwrap();
    std::fs::write(good.join("module.yaml"), "id: research\nbrief: 调研与选型\nsystem: 你负责调研\ntools: []\n").unwrap();
    let mismatch = dir.join("notes");
    std::fs::create_dir_all(&mismatch).unwrap();
    std::fs::write(mismatch.join("module.yaml"), "id: other\nbrief: x\nsystem: y\n").unwrap();
    let empty = dir.join("ghost");
    std::fs::create_dir_all(&empty).unwrap();

    let roster = module::scan(&dir);
    let ids: Vec<&str> = roster.modules.iter().map(|m| m.manifest.id.as_str()).collect();
    assert_eq!(ids, vec!["research"]);
    assert_eq!(roster.rejected.len(), 2);
    assert!(roster.rejected.iter().any(|r| r.contains("不一致")));
    assert!(roster.rejected.iter().any(|r| r.contains("缺少 module.yaml")));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- 协作引擎：成员自有通道，假模型脚本驱动 ----

fn scripted_discussion(scripts: Vec<Vec<String>>, allow: bool) -> Discussion {
    let ids = ["a", "b", "c"];
    let members: Vec<Member> = scripts
        .into_iter()
        .enumerate()
        .map(|(i, s)| Member::new(ids[i], format!("{} 的职责", ids[i]), Box::new(FakeChat::new(s))))
        .collect();
    Discussion::new(members, allow)
}

#[test]
fn discussion_full_flow_open_step_done() {
    let mut disc = scripted_discussion(
        vec![
            vec![FakeChat::say("a：我先说。"), FakeChat::verb_json("agree", "同意")],
            vec![FakeChat::say("b：我补充。"), FakeChat::verb_json("agree", "同意")],
            vec![FakeChat::say("c：我没意见。"), FakeChat::verb_json("agree", "同意")],
        ],
        false,
    );
    disc.open("调研并选型", "讨论约定：说事、ask、leave、agree。");
    assert_eq!(disc.transcript.len(), 3);
    match disc.step() {
        TurnOut::Done => {}
        _ => panic!("应在一轮后收敛"),
    }
    let mut core = FakeChat::new(vec![FakeChat::say("== 任务清单 ==")]);
    let plan = disc.synthesize(&mut core);
    assert!(!plan.is_empty());
}

#[test]
fn ask_pauses_and_user_answer_enters_transcript() {
    let mut disc = scripted_discussion(
        vec![
            vec![FakeChat::say("a：我先说。"), FakeChat::verb_json("agree", "同意")],
            vec![FakeChat::say("b：我补充。"), FakeChat::verb_json("agree", "同意")],
            vec![FakeChat::say("c：我先看看。"), FakeChat::verb_json("ask", "选哪个库？"), FakeChat::verb_json("agree", "同意")],
        ],
        false,
    );
    disc.open("任务", "约定");
    match disc.step() {
        TurnOut::AskUser { member, question } => {
            assert_eq!(member, "c");
            assert_eq!(question, "选哪个库？");
        }
        _ => panic!("应中止于请教"),
    }
    disc.pending_user_answers.push("用 sqlite".into());
    match disc.step() {
        TurnOut::Done => {}
        _ => panic!("回答后应收敛"),
    }
    assert!(disc.transcript.iter().any(|l| l.contains("[用户] 用 sqlite")));
}

#[test]
fn leave_removes_member_from_consensus() {
    let mut disc = scripted_discussion(
        vec![
            vec![FakeChat::say("a：我先说。"), FakeChat::verb_json("agree", "同意")],
            vec![FakeChat::verb_json("leave", "帮不上忙"), FakeChat::verb_json("agree", "不该被问到")],
            vec![FakeChat::say("c：没意见。"), FakeChat::verb_json("agree", "同意")],
        ],
        false,
    );
    disc.open("任务", "约定");
    match disc.step() {
        TurnOut::Done => {}
        _ => panic!("b 退场后其余同意应收敛"),
    }
    assert!(disc.transcript.iter().any(|l| l.contains("[b:leave]")));
}

#[test]
fn autonomy_ask_does_not_pause() {
    let mut disc = scripted_discussion(
        vec![
            vec![FakeChat::say("a：我先说。"), FakeChat::verb_json("agree", "同意")],
            vec![FakeChat::say("b：我补充。"), FakeChat::verb_json("agree", "同意")],
            vec![FakeChat::say("c：我先看看。"), FakeChat::verb_json("ask", "选哪个库？"), FakeChat::verb_json("agree", "同意")],
        ],
        true,
    );
    disc.open("任务", "约定");
    let mut guard = 0;
    loop {
        match disc.step() {
            TurnOut::Done => break,
            _ => {
                guard += 1;
                assert!(guard <= 3, "自裁模式下未按预期收敛");
            }
        }
    }
    assert!(disc.transcript.iter().any(|l| l.contains("[core] 已授权小组自裁")));
}

#[test]
fn round_cap_is_enforced() {
    let mut disc = scripted_discussion(
        vec![
            vec!["say|a".to_string(); MAX_ROUNDS * 2 + 4],
            vec!["say|b".to_string(); MAX_ROUNDS * 2 + 4],
        ],
        false,
    );
    disc.open("任务", "约定");
    let mut guard = 0;
    loop {
        match disc.step() {
            TurnOut::Done => break,
            TurnOut::Round | TurnOut::AskUser { .. } => {
                guard += 1;
                assert!(guard <= MAX_ROUNDS + 2, "轮次上限未生效");
            }
        }
    }
    assert!(disc.round > MAX_ROUNDS);
}

#[test]
fn execution_review_and_rework_cap() {
    let mut disc = scripted_discussion(
        vec![
            vec![FakeChat::say("a：我先说。"), FakeChat::verb_json("agree", "同意")],
            vec![FakeChat::say("b：我补充。"), FakeChat::verb_json("agree", "同意")],
        ],
        false,
    );
    disc.open("任务", "约定");
    let _ = disc.step();
    let mut core = FakeChat::new(vec![
        FakeChat::say("== 任务清单 =="),
        "say|[{\"item\":\"回报与方案一致\",\"status\":\"pass\",\"evidence\":\"一致\"}]".to_string(),
    ]);
    let plan = disc.synthesize(&mut core);
    let mut exec = Execution::run(disc.members.as_mut_slice(), &plan);
    exec.review(&mut core, &plan);
    assert!(exec.all_pass());
    assert_eq!(exec.rework, 0);
}

#[test]
fn review_parse_failure_is_conservative() {
    let mut core = FakeChat::new(vec![FakeChat::say("不是清单")]);
    let mut exec = Execution { reports: Default::default(), checklist_raw: String::new(), items: Vec::new(), rework: 0 };
    exec.review(&mut core, "方案");
    assert!(!exec.all_pass());
    assert!(exec.items.is_empty());
}

// ---- 核心层：会话状态机 / 通道回落 / 登记处端口 ----

/// 临时产品根：modules/research + 空登记处（无供应商 → 全部走假模型回落）。
fn temp_root(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("solomni-core-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let m = dir.join("modules").join("research");
    std::fs::create_dir_all(&m).unwrap();
    std::fs::write(m.join("module.yaml"), "id: research\nbrief: 调研与选型\nsystem: 你负责调研\ntools: []\n").unwrap();
    dir
}

fn has_notice(events: &[SessionEvent], kw: &str) -> bool {
    events.iter().any(|e| matches!(e, SessionEvent::Notice(n) if n.contains(kw)))
}

#[test]
fn core_direct_falls_back_with_honest_notice() {
    let root = temp_root("direct");
    let core = Core::open(root.clone());
    let mut s = core.start_direct("research").unwrap();
    // 无供应商：回落假模型并如实告知。
    let events = s.open();
    assert!(has_notice(&events, "假模型"));
    match s.say("你好") {
        SessionEvent::Transcript(lines) => assert!(lines[0].contains("[research]")),
        _ => panic!("直连应吐转录事件"),
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn core_collab_demo_runs_full_five_stages() {
    let root = temp_root("collab");
    let core = Core::open(root.clone());
    let mut s = core.start_collab("research").unwrap();
    let ev = s.set_task(&core, "调研数据库");
    assert!(has_notice(&ev, "[建组]"));
    assert!(matches!(s.pending, Some(Pending::ConfirmBegin)));

    let ev = s.begin(&core, false);
    // 成员回落假模型（告知）→ 讨论 → 整理 → 执行 → 验收（保守判否）→ 返工至超限 → 裁决。
    assert!(has_notice(&ev, "假模型"));
    assert!(ev.iter().any(|e| matches!(e, SessionEvent::DiscussionDone { .. })));
    assert!(ev.iter().any(|e| matches!(e, SessionEvent::Plan(_))));
    assert!(ev.iter().any(|e| matches!(e, SessionEvent::Report { .. })));
    assert!(ev.iter().any(|e| matches!(e, SessionEvent::Review { .. })));
    assert!(ev.iter().any(|e| matches!(e, SessionEvent::Delivery { ok: false, over_rework: true })));
    assert!(matches!(ev.last(), Some(SessionEvent::Ended)));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn core_collab_delegated_fails_gracefully_without_channel() {
    let root = temp_root("delegate");
    let core = Core::open(root.clone());
    let mut s = core.start_collab("?").unwrap();
    let ev = s.set_task(&core, "调研数据库");
    // 代拟需要核心通道；假模型不会输出名单 JSON → 优雅失败并结束（不静默造名单）。
    assert!(has_notice(&ev, "代拟失败"));
    assert!(matches!(ev.last(), Some(SessionEvent::Ended)));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn core_provider_lifecycle_and_key_never_leaks_to_view() {
    let root = temp_root("provider");
    let mut core = Core::open(root.clone());
    core.provider_upsert("p1", "http://x", "secret-key-1", &["m1".to_string()]).unwrap();
    core.provider_upsert("p2", "http://y", "secret-key-2", &[]).unwrap();
    // 列表视图永不包含密钥。
    for line in core.provider_list() {
        assert!(!line.contains("secret-key"), "列表泄露密钥：{}", line);
    }
    // 首入者自动默认；改默认；移除。
    assert_eq!(core.provider_default(), Some("p1".into()));
    assert!(core.provider_set_default("p2").unwrap());
    assert_eq!(core.provider_default(), Some("p2".into()));
    assert!(core.provider_remove("p1").unwrap());
    assert!(!core.provider_remove("p1").unwrap());
    // 持久化落在产品根相对锚点。
    assert!(root.join(".home").join("providers.yaml").exists());
    let _ = std::fs::remove_dir_all(&root);
}
