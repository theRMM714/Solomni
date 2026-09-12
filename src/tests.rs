//! mock 测试：不依赖网络与真实密钥。假模型脚本化应答，全链路可重复。

use crate::envelope;
use crate::model::{Chat, FakeChat, Msg};
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
    reg.providers.insert(
        "a".into(),
        Provider { kind: "llm".into(), base_url: "http://a".into(), api_key: "k-a".into(), models: vec![] },
    );
    reg.providers.insert(
        "b".into(),
        Provider { kind: "llm".into(), base_url: "http://b".into(), api_key: "k-b".into(), models: vec![] },
    );
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
    std::fs::write(
        good.join("module.yaml"),
        "id: research\nbrief: 调研与选型\nsystem: 你负责调研\ntools: []\n",
    )
    .unwrap();
    // id 与文件夹名不一致 → 拒收。
    let mismatch = dir.join("notes");
    std::fs::create_dir_all(&mismatch).unwrap();
    std::fs::write(mismatch.join("module.yaml"), "id: other\nbrief: x\nsystem: y\n").unwrap();
    // 缺 yaml → 拒收。
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

// ---- 协作五阶段：假模型全链路 ----

fn three_member_discussion() -> (Discussion<'static>, Vec<FakeChat>, FakeChat) {
    // 注：测试内用泄漏避免自引用生命周期；测试进程短暂，可接受。
    let chats: Vec<FakeChat> = vec![
        FakeChat::new(vec![FakeChat::say("a：我先说。"), FakeChat::verb_json("agree", "同意")]),
        FakeChat::new(vec![FakeChat::say("b：我补充。"), FakeChat::verb_json("agree", "同意")]),
        FakeChat::new(vec![FakeChat::say("c：我没意见。"), FakeChat::verb_json("agree", "同意")]),
    ];
    let boxed: Vec<Box<dyn Chat>> = chats.into_iter().map(|c| Box::new(c) as Box<dyn Chat>).collect();
    let leaked: Vec<&'static mut dyn Chat> = boxed
        .into_iter()
        .map(|b| Box::leak(b) as &'static mut dyn Chat)
        .collect();
    let mut members: Vec<Member> = Vec::new();
    for (i, chat) in leaked.into_iter().enumerate() {
        let id = match i {
            0 => "a",
            1 => "b",
            _ => "c",
        };
        members.push(Member { id, system: format!("{} 的职责", id), chat, present: true, agreed: false });
    }
    let disc = Discussion { members, transcript: Vec::new(), round: 0, pending_user_answers: Vec::new(), closed: false, allow_autonomy: false };
    (disc, Vec::new(), FakeChat::new(vec![]))
}

#[test]
fn discussion_full_flow_open_step_done() {
    let (mut disc, _keep, mut core) = three_member_discussion();
    disc.open("调研并选型", "讨论约定：说事、ask、leave、agree。");
    assert_eq!(disc.transcript.len(), 3);
    // 一轮之后全员同意 → Done。
    match disc.step() {
        TurnOut::Done => {}
        _ => panic!("应在一轮后收敛"),
    }
    let plan = disc.synthesize(&mut core);
    assert!(!plan.is_empty());
}

#[test]
fn ask_pauses_and_user_answer_enters_transcript() {
    // c 在 step 轮 ask：轮转中止，等用户回答；回答并入后 c 再同意。
    let chats: Vec<FakeChat> = vec![
        FakeChat::new(vec![FakeChat::say("a：我先说。"), FakeChat::verb_json("agree", "同意")]),
        FakeChat::new(vec![FakeChat::say("b：我补充。"), FakeChat::verb_json("agree", "同意")]),
        FakeChat::new(vec![FakeChat::say("c：我先看看。"), FakeChat::verb_json("ask", "选哪个库？"), FakeChat::verb_json("agree", "同意")]),
    ];
    let boxed: Vec<Box<dyn Chat>> = chats.into_iter().map(|c| Box::new(c) as Box<dyn Chat>).collect();
    let leaked: Vec<&'static mut dyn Chat> = boxed.into_iter().map(|b| Box::leak(b) as &'static mut dyn Chat).collect();
    let ids = ["a", "b", "c"];
    let mut members: Vec<Member> = leaked
        .into_iter()
        .enumerate()
        .map(|(i, chat)| Member { id: ids[i], system: format!("{} 的职责", ids[i]), chat, present: true, agreed: false })
        .collect();
    let mut disc = Discussion { members, transcript: Vec::new(), round: 0, pending_user_answers: Vec::new(), closed: false, allow_autonomy: false };
    disc.open("任务", "约定");
    match disc.step() {
        TurnOut::AskUser { member, question } => {
            assert_eq!(member, "c");
            assert_eq!(question, "选哪个库？");
        }
        _ => panic!("应中止于请教"),
    }
    // 用户回答并入转录，下一轮 c 同意 → 全员同意。
    disc.pending_user_answers.push("用 sqlite".into());
    match disc.step() {
        TurnOut::Done => {}
        _ => panic!("回答后应收敛"),
    }
    assert!(disc.transcript.iter().any(|l| l.contains("[用户] 用 sqlite")));
}

#[test]
fn leave_removes_member_from_consensus() {
    // b 退场后只剩 a、c 投票。
    let chats: Vec<FakeChat> = vec![
        FakeChat::new(vec![FakeChat::say("a：我先说。"), FakeChat::verb_json("agree", "同意")]),
        FakeChat::new(vec![FakeChat::verb_json("leave", "帮不上忙"), FakeChat::verb_json("agree", "不该被问到")]),
        FakeChat::new(vec![FakeChat::say("c：没意见。"), FakeChat::verb_json("agree", "同意")]),
    ];
    let boxed: Vec<Box<dyn Chat>> = chats.into_iter().map(|c| Box::new(c) as Box<dyn Chat>).collect();
    let leaked: Vec<&'static mut dyn Chat> = boxed.into_iter().map(|b| Box::leak(b) as &'static mut dyn Chat).collect();
    let ids = ["a", "b", "c"];
    let mut members: Vec<Member> = leaked
        .into_iter()
        .enumerate()
        .map(|(i, chat)| Member { id: ids[i], system: format!("{} 的职责", ids[i]), chat, present: true, agreed: false })
        .collect();
    let mut disc = Discussion { members, transcript: Vec::new(), round: 0, pending_user_answers: Vec::new(), closed: false, allow_autonomy: false };
    disc.open("任务", "约定");
    match disc.step() {
        TurnOut::Done => {}
        _ => panic!("b 退场后其余同意应收敛"),
    }
    assert!(disc.transcript.iter().any(|l| l.contains("[b:leave]")));
}

#[test]
fn execution_and_review_pass() {
    let (mut disc, _keep, _core) = three_member_discussion();
    disc.open("任务", "约定");
    let _ = disc.step();
    // 核心假模型：整理输出任务文本 → 验收输出结构化 pass 清单。
    let mut core = FakeChat::new(vec![
        FakeChat::say("== 任务清单 =="),
        "say|[\n  {\"item\":\"回报与方案一致\",\"status\":\"pass\",\"evidence\":\"成员回报一致\"}\n]".to_string(),
    ]);
    let plan = disc.synthesize(&mut core);
    let mut exec = Execution::run(&mut disc.members, &plan);
    exec.review(&mut core, &plan);
    assert!(exec.all_pass());
    assert_eq!(exec.rework, 0);
}

#[test]
fn review_parse_failure_is_conservative() {
    let (_disc, _keep, _core) = three_member_discussion();
    // 核心假模型：验收环节输出一个 say 对象（非清单）→ 解析失败 → 保守判否。
    let mut core = FakeChat::new(vec![FakeChat::say("不是清单")]);
    let mut exec = Execution { reports: Default::default(), checklist_raw: String::new(), items: Vec::new(), rework: 0 };
    exec.review(&mut core, "方案");
    assert!(!exec.all_pass());
    assert!(exec.items.is_empty());
}

#[test]
fn autonomy_ask_does_not_pause() {
    // c 在 step 轮 ask：allow_autonomy = true 时不中止，留档后继续收敛。
    let chats: Vec<FakeChat> = vec![
        FakeChat::new(vec![FakeChat::say("a：我先说。"), FakeChat::verb_json("agree", "同意")]),
        FakeChat::new(vec![FakeChat::say("b：我补充。"), FakeChat::verb_json("agree", "同意")]),
        FakeChat::new(vec![FakeChat::say("c：我先看看。"), FakeChat::verb_json("ask", "选哪个库？"), FakeChat::verb_json("agree", "同意")]),
    ];
    let boxed: Vec<Box<dyn Chat>> = chats.into_iter().map(|c| Box::new(c) as Box<dyn Chat>).collect();
    let leaked: Vec<&'static mut dyn Chat> = boxed.into_iter().map(|b| Box::leak(b) as &'static mut dyn Chat).collect();
    let ids = ["a", "b", "c"];
    let mut members: Vec<Member> = leaked
        .into_iter()
        .enumerate()
        .map(|(i, chat)| Member { id: ids[i], system: format!("{} 的职责", ids[i]), chat, present: true, agreed: false })
        .collect();
    let mut disc = Discussion { members, transcript: Vec::new(), round: 0, pending_user_answers: Vec::new(), closed: false, allow_autonomy: true };
    disc.open("任务", "约定");
    // 自裁模式下 ask 留档不中止；下一轮 c 投同意后收敛。
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
    // 全员永远 say，不 agree：应在轮次上限后终止。
    let chats: Vec<FakeChat> = vec![
        FakeChat::new((0..MAX_ROUNDS * 2 + 4).map(|i| FakeChat::say(&format!("a {}", i))).collect()),
        FakeChat::new((0..MAX_ROUNDS * 2 + 4).map(|i| FakeChat::say(&format!("b {}", i))).collect()),
    ];
    let boxed: Vec<Box<dyn Chat>> = chats.into_iter().map(|c| Box::new(c) as Box<dyn Chat>).collect();
    let leaked: Vec<&'static mut dyn Chat> = boxed.into_iter().map(|b| Box::leak(b) as &'static mut dyn Chat).collect();
    let ids = ["a", "b"];
    let mut members: Vec<Member> = leaked
        .into_iter()
        .enumerate()
        .map(|(i, chat)| Member { id: ids[i], system: format!("{} 的职责", ids[i]), chat, present: true, agreed: false })
        .collect();
    let mut disc = Discussion { members, transcript: Vec::new(), round: 0, pending_user_answers: Vec::new(), closed: false, allow_autonomy: false };
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