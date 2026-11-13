//! 目的：钉 CLI 前端（传输侧）的输入解析与点名包装契约。
//! 管：动作 id 两种写法、会话创建走动作表、逗号与空白分名、空登记处点名引导、等待清单逐行、EOF 停卡。
//! 不管：业务规则：点名归 registry、命名与生成中禁改归 session/conductor、组合语义与动作分发。
//! 联动：`ActionOps::act`、`conductor::create_session`、src/tests/api/ 的对应测试。

use super::doubles::module_of;
use super::ops_with;
use super::prelude::*;
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
        .upsert_agent("甲", &["a".to_string()], "", "", "")
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
        .upsert_agent("甲", &["a".to_string()], "", "", "")
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
    assert!(
        cli::pick_agents(&ops, &[])
            .unwrap_err()
            .contains("没有点名")
    );
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

/// 目的：CLI 上挂着的裁决答不上（输入到尽头 EOF）时按**停止 = 拒绝**收场——不能一直吊着。
/// 约束：停止不套用发起方声明的默认项；等待方被解开、会话随后不再被派发。
#[test]
fn eof_answers_a_pending_card_by_stopping_instead_of_hanging() {
    use crate::capabilities::session::api::{AnswerSlot, Pending, SlotWake};

    let (handle, ops) = ops_with(vec![module_of("a")], Vec::new());
    let sid = "cli-eof-1";
    ops.sessions
        .create_work(work(sid, WorkMode::Single, &["a"]))
        .expect("建会话");
    let door = {
        let s = sid.to_string();
        handle
            .call(move |core| Ok(core.desk_of(&s)))
            .expect("取这个会话的裁决队")
    };
    // 造一个"工作线程正等答"的现场：卡片带等待格进队，等待格被那个线程等着。
    let slot = AnswerSlot::new();
    door.push(None, Pending::ConfirmBegin, "", Some(Arc::clone(&slot)));
    let waiter = {
        let slot = Arc::clone(&slot);
        std::thread::spawn(move || slot.wait())
    };
    // 输入到尽头：按停止收场，等待方被解开（而不是永远挂着）。
    cli::eof_stop(&ops, sid);
    assert_eq!(
        waiter.join().expect("等待线程"),
        SlotWake::Stopped,
        "EOF = 停止（拒绝），不是“没人答”"
    );
    assert!(!ops.sessions.is_running(sid), "EOF 收场后这条会话不再在跑");
}
/// 行编辑：插入、左移、再插入与退格都按光标位置生效。
#[test]
fn editor_edits_around_the_cursor() {
    cli::clear_history();
    let mut ed = cli::Editor::new();
    for c in "abc".chars() {
        assert_eq!(ed.key(cli::Key::Char(c)), cli::Step::Edit);
    }
    ed.key(cli::Key::Left);
    ed.key(cli::Key::Char('X'));
    assert_eq!(ed.line(), "abXc", "插入落在光标处");
    ed.key(cli::Key::Backspace);
    assert_eq!(ed.line(), "abc");
    ed.key(cli::Key::Home);
    ed.key(cli::Key::Delete);
    assert_eq!(ed.line(), "bc", "Delete 删光标右边那格");
    assert_eq!(ed.head(), "", "光标在行首");
}

/// 粘贴：换行折成空格，不打断单行输入框。
#[test]
fn editor_folds_pasted_newlines() {
    cli::clear_history();
    let mut ed = cli::Editor::new();
    ed.key(cli::Key::Paste("第一行\n第二行".to_string()));
    assert_eq!(ed.line(), "第一行 第二行");
}

/// 提交：回车提交并记历史，上翻能取回；连续重复不重复入栈。
#[test]
fn editor_submit_remembers_history() {
    cli::clear_history();
    let mut ed = cli::Editor::new();
    for c in "写法".chars() {
        ed.key(cli::Key::Char(c));
    }
    assert_eq!(ed.key(cli::Key::Enter), cli::Step::Submit);
    assert_eq!(ed.commit(), "写法");
    assert_eq!(ed.line(), "", "提交后输入框清空");

    let mut again = cli::Editor::new();
    again.key(cli::Key::Up);
    assert_eq!(again.line(), "写法", "上翻取回上一行");

    let mut dup = cli::Editor::new();
    for c in "写法".chars() {
        dup.key(cli::Key::Char(c));
    }
    dup.key(cli::Key::Enter);
    dup.commit();
    let mut walk = cli::Editor::new();
    walk.key(cli::Key::Up);
    assert_eq!(walk.line(), "写法");
    walk.key(cli::Key::Up);
    assert_eq!(walk.line(), "写法", "同一条不重复入栈");
}

/// 历史下翻回到空行；Esc 清空输入不提交；Ctrl+U 清行；Ctrl+C 收摊。
#[test]
fn editor_history_down_and_cancel_keys() {
    cli::clear_history();
    let mut ed = cli::Editor::new();
    for c in "alpha".chars() {
        ed.key(cli::Key::Char(c));
    }
    ed.key(cli::Key::Enter);
    ed.commit();
    for c in "beta".chars() {
        ed.key(cli::Key::Char(c));
    }
    ed.key(cli::Key::Enter);
    ed.commit();

    let mut walk = cli::Editor::new();
    walk.key(cli::Key::Up);
    assert_eq!(walk.line(), "beta");
    walk.key(cli::Key::Up);
    assert_eq!(walk.line(), "alpha");
    walk.key(cli::Key::Down);
    assert_eq!(walk.line(), "beta");
    walk.key(cli::Key::Down);
    assert_eq!(walk.line(), "", "越过最新一条回到空行");

    let mut ed = cli::Editor::new();
    ed.key(cli::Key::Char('x'));
    assert_eq!(ed.key(cli::Key::Escape), cli::Step::Cancel);
    ed.clear();
    assert_eq!(ed.line(), "");
    ed.key(cli::Key::Char('y'));
    assert_eq!(ed.key(cli::Key::KillLine), cli::Step::Edit);
    assert_eq!(ed.line(), "");
    assert_eq!(ed.key(cli::Key::Interrupt), cli::Step::Quit);
}

/// 列宽：中日韩字符占 2 格，截断不劈开宽字符并留省略号。
#[test]
fn width_counts_cjk_as_two_and_cuts_whole() {
    assert_eq!(cli::cols("abc"), 3);
    assert_eq!(cli::cols("中a"), 3);
    assert_eq!(cli::cols("中文"), 4);
    assert_eq!(cli::cut("中文abc", 5), "中文…");
    assert_eq!(cli::cut("abc", 10), "abc", "没超宽就不加省略号");
}
/// 折行：按列折、不劈开宽字符；空行保留（裁决卡正文与详情共用）。
#[test]
fn width_wraps_without_breaking_wide_chars() {
    assert_eq!(cli::wrap("中文中文abc", 7), vec!["中文中", "文abc"]);
    assert_eq!(cli::wrap("a\n\nb", 4), vec!["a", "", "b"]);
    assert_eq!(cli::wrap("", 10), vec![""]);
}

/// 生成态帧：允许动画时轮转，`--no-anim` 时始终同一帧。
#[test]
fn spinner_rotates_only_when_animation_is_on() {
    assert_eq!(cli::spin_frames(0, true), "⠋");
    assert_eq!(cli::spin_frames(1, true), "⠙");
    assert_eq!(cli::spin_frames(10, true), "⠋", "十帧一轮");
    assert_eq!(cli::spin_frames(0, false), "⠿");
    assert_eq!(cli::spin_frames(7, false), "⠿", "静止帧不随 tick 变");
}

/// 旗语解析：两种写法等价、布尔不带值、`--` 之后全当位置参数、写错给最近建议。
#[test]
fn argv_accepts_both_spellings_and_reports_typos() {
    let parse = |args: &[&str]| {
        let argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        cli::argv::parse(&argv)
    };
    let flags = parse(&["--root", "work", "--port=3082", "--web", "--no-anim"]).expect("合法旗语");
    assert_eq!(flags.root.as_deref(), Some("work"));
    assert_eq!(flags.port, Some(3082));
    assert!(flags.web && flags.no_anim && !flags.plain);

    let flags = parse(&["--plain", "--root=elsewhere"]).expect("=值写法");
    assert!(flags.plain);
    assert_eq!(flags.root.as_deref(), Some("elsewhere"));

    let flags = parse(&["--help", "--all"]).expect("--all 修饰 --help");
    assert!(flags.help && flags.all, "--help --all 连内部旗语一起列");

    let err = parse(&["--por", "1"]).expect_err("拼错要报错");
    assert!(err.contains("你是不是想用 --port"), "给最近建议：{err}");
    let err = parse(&["--web=1"]).expect_err("布尔旗不带值");
    assert!(err.contains("不带值"), "{err}");
    let err = parse(&["--root"]).expect_err("缺值要报错");
    assert!(err.contains("需要一个值"), "{err}");

    let flags = parse(&["--", "--web"]).expect("-- 之后都是位置参数");
    assert!(!flags.web);
    assert_eq!(flags.positionals, vec!["--web"]);
}

/// 命令表是唯一来源：手册、单条说明、补全与未知命令建议都由它生成。
#[test]
fn catalog_drives_help_completion_and_suggestions() {
    let help = cli::catalog::help_text(false);
    assert!(
        help.contains("全局旗语") && help.contains("语法规则"),
        "{help}"
    );
    assert!(help.contains("single"), "命令表在手册里");
    assert!(!help.contains("--probe-tools"), "内部旗语默认不列");
    assert!(cli::catalog::help_text(true).contains("--probe-tools"));

    let single = cli::catalog::detail("single").expect("单条说明");
    assert!(single.contains("<agent名…>"), "{single}");
    assert!(cli::catalog::detail("/help").is_some(), "别名也认");
    assert!(cli::catalog::detail("nope").is_none());

    assert_eq!(cli::catalog::complete("he"), vec!["help"]);
    assert!(cli::catalog::complete("").contains(&"web".to_string()));
    assert_eq!(cli::catalog::suggest("singel"), Some("single"));
    assert_eq!(cli::catalog::suggest("zzz"), None);
}

/// Tab 补全：唯一候选补全并留空格，多候选先补公共前缀，无候选不动输入。
#[test]
fn editor_tab_completion_takes_unique_and_common_prefix() {
    let mut ed = cli::Editor::new();
    for c in "he".chars() {
        ed.key(cli::Key::Char(c));
    }
    assert_eq!(ed.complete(&["help".to_string()]), 1);
    assert_eq!(ed.line(), "help ");

    let mut ed = cli::Editor::new();
    for c in "si".chars() {
        ed.key(cli::Key::Char(c));
    }
    ed.complete(&["single".to_string(), "sincere".to_string()]);
    assert_eq!(ed.line(), "sin");

    let before = ed.line();
    ed.complete(&[]);
    assert_eq!(ed.line(), before, "没有候选就不动输入");
}
