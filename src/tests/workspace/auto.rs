//! 目的：自动提交与沙箱采纳——只走快进，基线过期只上报。
//! 管：写工具落地后的自动提交、沙箱改动采纳、锚与基线。
//! 不管：人工冲突解决（`resolve.rs`）。
//! 联动：`docs/storage/README.md`。

use super::super::builders::*;
use super::super::prelude::*;
use super::support::*;

/// 目的：自动提交只走快进——基线必须等于主线头；主线被推进后同一路径必须被拒，不悄悄变成本地优先。
#[test]
pub(crate) fn work_commit_auto_accepts_only_fresh_bases() {
    use crate::capabilities::workspace::api::CommitRefusal;
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();

    // a 先拉、再改：基线等于主线头 → 自动提交成立。
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    let sb_a = sandbox_of(&ws, "w", "a");
    store.write_under(&sb_a, "note.md", b"v2").unwrap();
    let first = ws
        .work_commit_auto("w", "a", "w--a", &commit_req(&["note.md"], &[], "a 改", 2))
        .expect("基线新鲜的自动提交")
        .expect("有可发布的改动");
    assert_eq!(first.id, 2);

    // b 拉到的就是 v2，改完自动提交 → 主线推进到 3；a 的基线仍停在 2。
    ws.work_pull("w", "b", &["note.md".to_string()]).unwrap();
    let sb_b = sandbox_of(&ws, "w", "b");
    store.write_under(&sb_b, "note.md", b"v3").unwrap();
    let second = ws
        .work_commit_auto("w", "b", "w--b", &commit_req(&["note.md"], &[], "b 改", 3))
        .expect("b 的基线也新鲜")
        .expect("有可发布的改动");
    assert_eq!(second.id, 3);

    // a 再写同一路径：基线 2 ≠ 主线 3 → 拒绝，点到路径与两边提交号，且不留半个提交。
    store.write_under(&sb_a, "note.md", b"v4").unwrap();
    let refused = ws
        .work_commit_auto(
            "w",
            "a",
            "w--a",
            &commit_req(&["note.md"], &[], "a 再改", 4),
        )
        .expect_err("过期基线必须被拒");
    match &refused {
        CommitRefusal::Stale { main, paths } => {
            assert_eq!(main.oid, 3);
            assert_eq!(paths.len(), 1);
            assert_eq!(paths[0].path, "note.md");
            assert_eq!(paths[0].base, Some(2));
        }
        other => panic!("拒绝类型不对：{}", other),
    }
    assert!(refused.to_string().contains("work_pull"), "{}", refused);
    let head = store.head(&ws.roots("w", &[]).unwrap().store).unwrap();
    assert_eq!(head, Some(3), "拒绝不得留下提交");

    // 新文件（主线里没有、也没拉过）按新文件放行：不然模型永远写不出第一个文件。
    store.write_under(&sb_a, "fresh.md", b"new").unwrap();
    let third = ws
        .work_commit_auto(
            "w",
            "a",
            "w--a",
            &commit_req(&["fresh.md"], &[], "新文件", 5),
        )
        .expect("新文件不要求基线")
        .expect("有可发布的改动");
    assert_eq!(third.id, 4);
}

/// 目的：钉住写工具落地后的自动提交——路径取自这次调用写过的文件，锚是工具调用那一行；基线过期只上报不发布。
#[test]
pub(crate) fn write_tools_publish_to_shared_area_and_report_stale_bases() {
    use crate::capabilities::collab::service::tool_loop::commit_writes;
    use crate::capabilities::tools::api::Observations;
    use crate::capabilities::tools::domain::systool::fnv1a;
    use crate::capabilities::workspace::ports::WorkStore;

    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    let sb_dir = sandbox_of(&ws, "w", "a");
    let mut sb = test_sandbox("a", &[]);
    sb.work_name = "w".to_string();
    sb.session = "w--a".to_string();
    sb.private = sb_dir.clone();
    sb.shared = shared_of(&ws, "w");
    let mut m = member_with_tools("m0", Vec::new(), Arc::new(SilentRunner));
    {
        let t = m.tools.as_mut().expect("成员工具面");
        t.sandbox = sb;
        t.workspace = Some(ws.clone());
        // 锚点 = 这次工具调用那一行（回档据此退掉这次写入）。
        t.line.store(7, std::sync::atomic::Ordering::Relaxed);
    }

    // 主线已有 note.md，a 拉过它 → 基线新鲜，写完自动发布。
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    store.write_under(&sb_dir, "note.md", b"v2").unwrap();
    let mut obs = Observations::default();
    obs.note_written(&sb_dir.join("note.md"), 0);
    let mut out = crate::kernel::api::ToolOutcome {
        ok: true,
        output: "写入完成".to_string(),
    };
    commit_writes(m.tools.as_ref().unwrap(), &mut obs, &mut out);
    assert_eq!(out.output, "写入完成", "基线新鲜时不该有告警");
    assert_eq!(ws.work_head("w").unwrap(), Some(2), "写入应当已发布");
    let shared = shared_of(&ws, "w");
    assert_eq!(
        store.read_under(&shared, "note.md").unwrap().as_deref(),
        Some(b"v2".as_slice()),
        "共享区内容必须换成新写的这份"
    );

    // b 推进主线后，a 的基线过期：这次写入只留在沙箱，回执如实说清。
    ws.work_pull("w", "b", &["note.md".to_string()]).unwrap();
    store
        .write_under(&sandbox_of(&ws, "w", "b"), "note.md", b"v3")
        .unwrap();
    ws.work_commit_auto("w", "b", "w--b", &commit_req(&["note.md"], &[], "b 改", 3))
        .unwrap();
    store.write_under(&sb_dir, "note.md", b"v4").unwrap();
    let mut obs = Observations::default();
    obs.note_written(&sb_dir.join("note.md"), 0);
    let mut out = crate::kernel::api::ToolOutcome {
        ok: true,
        output: "写入完成".to_string(),
    };
    commit_writes(m.tools.as_ref().unwrap(), &mut obs, &mut out);
    assert!(
        out.output.contains("[自动提交未完成]"),
        "过期基线必须在回执里说清：{}",
        out.output
    );
    assert!(
        out.output.contains("work_pull"),
        "要给出下一步：{}",
        out.output
    );
    assert_eq!(ws.work_head("w").unwrap(), Some(3), "拒绝不得留下提交");

    // 删除也走自动提交：路径进 deletes，共享区同步删掉。
    // 用新文件避开上面被拒的本地改动——那条路径的基线仍是旧的，删除一样会被判过期。
    store.write_under(&sb_dir, "gone.md", b"tmp\n").unwrap();
    let mut obs = Observations::default();
    obs.note_written(&sb_dir.join("gone.md"), fnv1a("tmp\n"));
    let mut out = crate::kernel::api::ToolOutcome {
        ok: true,
        output: "已写入".to_string(),
    };
    commit_writes(m.tools.as_ref().unwrap(), &mut obs, &mut out);
    assert_eq!(out.output, "已写入", "新文件的自动提交不该有告警");
    assert_eq!(ws.work_head("w").unwrap(), Some(4), "新文件应当已发布");
    let shared = shared_of(&ws, "w");
    assert!(
        store.read_under(&shared, "gone.md").unwrap().is_some(),
        "共享区里必须有刚发布的新文件"
    );

    store.remove_under(&sb_dir, "gone.md").unwrap();
    let mut obs = Observations::default();
    obs.note_removed(&sb_dir.join("gone.md"));
    let mut out = crate::kernel::api::ToolOutcome {
        ok: true,
        output: "已删除".to_string(),
    };
    commit_writes(m.tools.as_ref().unwrap(), &mut obs, &mut out);
    assert_eq!(out.output, "已删除", "删除的自动提交不该有告警");
    assert_eq!(ws.work_head("w").unwrap(), Some(5), "删除应当已发布");
    assert!(
        store.read_under(&shared, "gone.md").unwrap().is_none(),
        "共享区里的文件必须跟着删掉"
    );
}

/// 目的：沙箱采纳把 shell / 模块工具落下的改动（新增 / 修改 / 删除）一次提交进共享区，来源记为 external；
///   重复采纳无改动即无提交——回合末扫描必须收敛。
#[test]
pub(crate) fn adopt_sandbox_publishes_shell_changes_and_converges() {
    use crate::capabilities::workspace::domain::workstore::Source;
    use crate::capabilities::workspace::ports::WorkStore;

    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    let sb_dir = sandbox_of(&ws, "w", "a");
    // 主线先有两条：note.md 要改、gone.md 要删、keep.md 不动。
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    ws.work_commit_user("w", "gone.md", b"tmp", 2, 0).unwrap();
    ws.work_commit_user("w", "keep.md", b"keep", 3, 0).unwrap();
    ws.work_pull(
        "w",
        "a",
        &[
            "note.md".to_string(),
            "gone.md".to_string(),
            "keep.md".to_string(),
        ],
    )
    .unwrap();
    let head = ws.work_head("w").unwrap().unwrap();

    // 模拟 shell / 模块工具直接改沙箱：改一条、删一条、新增一条。
    store.write_under(&sb_dir, "note.md", b"v2").unwrap();
    store.remove_under(&sb_dir, "gone.md").unwrap();
    store.write_under(&sb_dir, "new.md", b"shell").unwrap();
    let report = ws.work_adopt_sandbox("w", "a", "w--a", 9, 7).unwrap();
    let id = report.commit.expect("有改动必须提交");
    assert_eq!(id, head + 1, "一次采纳只产生一条提交");
    let mut changed = report.changed.clone();
    changed.sort();
    assert_eq!(
        changed,
        vec![
            "gone.md".to_string(),
            "new.md".to_string(),
            "note.md".to_string()
        ],
        "新增 / 修改 / 删除都在同一条提交里"
    );
    assert!(report.held.is_empty(), "全部快进，不该有没采纳的路径");
    let shared = shared_of(&ws, "w");
    assert_eq!(
        store.read_under(&shared, "note.md").unwrap().as_deref(),
        Some(b"v2".as_slice())
    );
    assert!(store.read_under(&shared, "gone.md").unwrap().is_none());
    assert_eq!(
        store.read_under(&shared, "new.md").unwrap().as_deref(),
        Some(b"shell".as_slice())
    );

    // 来源与锚：不是工具写，而是"采纳沙箱"，锚在这条转录行上（留档据此一起退掉）。
    let commit = store.read_commit(&store_of(&ws, "w"), id).unwrap().unwrap();
    assert_eq!(commit.source, Source::External);
    assert_eq!(commit.author, "a");
    assert_eq!(commit.session, "w--a");
    assert_eq!(commit.message, "沙箱改动（shell / 模块工具）");
    assert_eq!(
        commit.anchor.as_ref().map(|a| (a.agent.as_str(), a.line)),
        Some(("a", 7))
    );

    // 再扫一遍：沙箱与主线一致 → 不再产生提交（回合末扫盘不能每次都长出一条）。
    let again = ws.work_adopt_sandbox("w", "a", "w--a", 10, 8).unwrap();
    assert_eq!(again.commit, None, "没有改动就不提交");
    assert!(again.changed.is_empty());
    assert_eq!(ws.work_head("w").unwrap(), Some(id));
}

/// 目的：基线过期时的沙箱改动不被采纳——留在沙箱并逐条点名，主线一个提交都不准多；
///   拉取后同一条改动才能快进发布。
#[test]
pub(crate) fn adopt_sandbox_holds_stale_changes_until_pulled() {
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    ws.work_pull("w", "b", &["note.md".to_string()]).unwrap();
    // b 先改并发布：a 的基线从此过期。
    let sb_b = sandbox_of(&ws, "w", "b");
    store.write_under(&sb_b, "note.md", b"v2").unwrap();
    ws.work_commit_auto("w", "b", "w--b", &commit_req(&["note.md"], &[], "b 改", 2))
        .unwrap();
    // a 的沙箱里也改了（shell），但基线没跟上：不采纳。
    let sb_a = sandbox_of(&ws, "w", "a");
    store.write_under(&sb_a, "note.md", b"v3").unwrap();
    let held = ws.work_adopt_sandbox("w", "a", "w--a", 3, 4).unwrap();
    assert_eq!(held.commit, None, "过期基线不得留下提交");
    assert_eq!(held.held.len(), 1, "没采纳的路径要逐条点名");
    assert_eq!(held.held[0].path, "note.md");
    assert!(
        held.held[0].why.contains("work_pull"),
        "要给出下一步：{}",
        held.held[0].why
    );
    assert_eq!(
        store
            .read_under(&shared_of(&ws, "w"), "note.md")
            .unwrap()
            .as_deref(),
        Some(b"v2".as_slice()),
        "共享区保持 b 发布的那一版"
    );

    // 模型按回执先处置冲突（take=upstream 覆盖本地、被覆盖的那份进 lost），再改才能发布。
    ws.work_resolve(
        "w",
        "a",
        &resolve_req(
            &["note.md"],
            crate::capabilities::workspace::api::ResolveTake::Upstream,
        ),
    )
    .unwrap();
    store.write_under(&sb_a, "note.md", b"v4").unwrap();
    let published = ws.work_adopt_sandbox("w", "a", "w--a", 5, 6).unwrap();
    assert!(published.commit.is_some(), "拉取到最新基线后应当能发布");
    assert_eq!(
        store
            .read_under(&shared_of(&ws, "w"), "note.md")
            .unwrap()
            .as_deref(),
        Some(b"v4".as_slice())
    );
}
