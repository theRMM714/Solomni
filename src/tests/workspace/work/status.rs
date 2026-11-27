//! 目的：共享区只读视图：状态、版本、媒体、paths、存储形态、工作线提交链。
//! 管：五态判定、版本猜测、媒体冲突、paths 不整读、根+增量落盘、工具取用按席位面、提交链渲染。
//! 不管：提交与推送（`work/commit.rs`、`work/push.rs`）。
//! 联动：`docs/storage/README.md`。

use crate::tests::prelude::*;

use crate::tests::workspace::support::*;

/// 目的：work_status 按「工作线 / 主线 / 沙箱」报五态：未提交新增 → 未推送 → 已提交 → 提交后又改 → 未提交删除；主线有、没拉取单独标。
#[test]
pub(crate) fn work_status_reports_the_work_line_five_states() {
    use crate::units::business::versions::api::{StatusReport, WorkState};
    use crate::units::business::versions::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    let find =
        |st: &StatusReport, path: &str| st.entries.iter().find(|e| e.path == path).map(|e| e.state);
    let sb = sandbox_of(&ws, "w", "a");
    store.write_under(&sb, "draft.md", b"v1").unwrap();
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(find(&st, "draft.md"), Some(WorkState::Untracked));
    assert!(st.work_head.is_none() && st.unpushed == 0);

    let c1 = ws
        .work_commit(
            "w",
            "a",
            "w--a",
            &commit_req(&["draft.md"], &[], "第一版", 1),
        )
        .unwrap();
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(find(&st, "draft.md"), Some(WorkState::Unpushed));
    assert_eq!(st.unpushed, 1);
    assert_eq!(st.work_head.as_ref().map(|c| c.id), Some(c1.id));

    let published = push(&ws, "w", "a", "第一版", 1, 1)
        .unwrap()
        .commit
        .expect("有内容可推送");
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(find(&st, "draft.md"), Some(WorkState::Committed));
    assert_eq!(st.unpushed, 0);
    assert_eq!(st.head.as_ref().map(|c| c.id), Some(published));
    assert_eq!(st.work_head.as_ref().map(|c| c.id), Some(published));

    store.write_under(&sb, "draft.md", b"v2").unwrap();
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(find(&st, "draft.md"), Some(WorkState::Modified));
    store.remove_under(&sb, "draft.md").unwrap();
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(find(&st, "draft.md"), Some(WorkState::Deleted));

    // b 推送另一条路径：a 还没拉取，状态应说「主线有、还没拉取」而不是「未提交新增」。
    let sb_b = sandbox_of(&ws, "w", "b");
    store.write_under(&sb_b, "another.md", b"x").unwrap();
    ws.work_commit("w", "b", "w--b", &commit_req(&["another.md"], &[], "b", 2))
        .unwrap();
    push(&ws, "w", "b", "b", 2, 2).unwrap();
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(find(&st, "another.md"), Some(WorkState::NotPulled));

    // work_log 另列各 agent 工作线的提交，并标是否已推送。
    let log = ws.work_log("w", 10, None).unwrap();
    let mine = log
        .work
        .iter()
        .find(|e| e.id == c1.id)
        .expect("工作线提交在列");
    assert_eq!(mine.author, "a");
    assert!(mine.pushed, "已推送过");
}

/// 目的：work_versions 给文本预览、二进制只给指纹与大小；不把二进制冒充文本。
#[test]
pub(crate) fn work_versions_reports_text_and_binary_without_guessing() {
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    let binary: &[u8] = &[0, 159, 146, 150, 0, 255];
    ws.work_commit_user("w", "image.bin", binary, 2, 0).unwrap();
    ws.work_pull("w", "a", &["note.md".to_string(), "image.bin".to_string()])
        .unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    store.write_under(&sb, "image.bin", &[0, 1, 2]).unwrap();

    let v = ws.work_versions("w", "a", "image.bin").unwrap();
    assert_eq!(v.path, "image.bin");
    assert!(v.media_type.is_none(), "未知扩展名没有类型提示");
    assert!(v.base.is_some() && v.head.is_some() && v.local.is_some());
    let head = v.head.unwrap();
    assert!(head.text.is_none(), "二进制不能给文本预览");
    assert_eq!(head.size, binary.len() as u64);

    let note = ws.work_versions("w", "a", "note.md").unwrap();
    assert_eq!(note.media_type.as_deref(), Some("text/markdown"));
    assert_eq!(
        note.head_commit,
        Some(2),
        "work_versions 要给出 take=local 的 head"
    );
    assert_eq!(note.head.unwrap().text.as_deref(), Some("v1"));
    assert!(ws.work_versions("w", "a", "nope.bin").is_err());
}

/// 目的：work_versions 按扩展名给媒体类型；既有超限对象只给指纹/大小/类型，不再读内容。
#[test]
pub(crate) fn work_versions_reports_media_type_and_marks_oversize_metadata_only() {
    let dirs = Arc::new(InMemoryWorkspace::new());
    let store = Arc::new(InMemoryWorkStore::new());
    let ws = test_workspace_store_with_max(dirs.clone(), store.clone(), 1024);
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    let png = [0x89u8, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    ws.work_commit_user("w", "cover.png", &png, 1, 0).unwrap();
    ws.work_pull("w", "a", &["cover.png".to_string()]).unwrap();
    let v = ws.work_versions("w", "a", "cover.png").unwrap();
    assert_eq!(v.media_type.as_deref(), Some("image/png"));
    let head = v.head.unwrap();
    assert!(head.text.is_none(), "已知媒体不给文本预览");
    assert_eq!(head.size, png.len() as u64);

    // 用更小的上限重新装配同一个版本库：既有对象只给元数据，不读内容。
    let strict = test_workspace_store_with_max(dirs, store.clone(), 4);
    let v = strict.work_versions("w", "a", "cover.png").unwrap();
    assert_eq!(v.media_type.as_deref(), Some("image/png"));
    let head = v.head.unwrap();
    assert!(head.too_large, "超过新上限要标记");
    assert!(head.text.is_none());
    assert_eq!(head.size, png.len() as u64);

    // 超限提交与上传都被拒绝；head 不前进。
    let before = strict.work_head("w").unwrap();
    let sb = sandbox_of(&strict, "w", "a");
    store.write_under(&sb, "big.bin", &[0u8; 5]).unwrap();
    let err = strict
        .work_commit("w", "a", "w--a", &commit_req(&["big.bin"], &[], "big", 2))
        .unwrap_err();
    assert!(err.contains("上限") && err.contains("big.bin"), "{}", err);
    let err = strict
        .work_commit_user("w", "up.bin", &[0u8; 5], 3, 0)
        .unwrap_err();
    assert!(err.contains("上限"), "{}", err);
    assert_eq!(strict.work_head("w").unwrap(), before, "拒绝不能推进 head");
}

/// 目的：媒体资产的冲突也只能显式选边；三方视图按媒体类型给元数据，不做内容合并。
#[test]
pub(crate) fn work_media_conflict_requires_explicit_side() {
    use crate::units::business::versions::api::{ResolveAction, ResolveTake};
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    let base: &[u8] = &[0, 0, 0, 24, b'f', b't', b'y', b'p'];
    ws.work_commit_user("w", "clip.mp4", base, 1, 0).unwrap();
    let both = ["clip.mp4".to_string()];
    ws.work_pull("w", "a", &both).unwrap();
    ws.work_pull("w", "b", &both).unwrap();
    let sb_a = sandbox_of(&ws, "w", "a");
    let sb_b = sandbox_of(&ws, "w", "b");
    store.write_under(&sb_a, "clip.mp4", &[1, 2, 3]).unwrap();
    store.write_under(&sb_b, "clip.mp4", &[4, 5, 6]).unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&["clip.mp4"], &[], "a", 2))
        .unwrap();
    push(&ws, "w", "a", "a", 2, 2).unwrap();
    ws.work_commit("w", "b", "w--b", &commit_req(&["clip.mp4"], &[], "b", 3))
        .expect("本地提交不看别人");
    let err = push(&ws, "w", "b", "b", 3, 3).unwrap_err();
    assert!(err.contains("clip.mp4") && err.contains("冲突"), "{}", err);

    let v = ws.work_versions("w", "b", "clip.mp4").unwrap();
    assert_eq!(v.media_type.as_deref(), Some("video/mp4"));
    assert!(v.head.unwrap().text.is_none(), "媒体不给文本");
    assert!(v.local.unwrap().text.is_none(), "媒体不给文本");

    let rep = ws
        .work_resolve("w", "b", &resolve_req(&["clip.mp4"], ResolveTake::Upstream))
        .unwrap();
    assert_eq!(rep.entries[0].action, ResolveAction::WroteUpstream);
    assert_eq!(
        store.read_under(&sb_b, "clip.mp4").unwrap().as_deref(),
        Some(&[1, 2, 3][..])
    );
}

/// 目的：资产路径（pull / status / versions / commit / resolve / restore）不走整文件读取。
#[test]
pub(crate) fn work_asset_paths_do_not_use_whole_file_reads() {
    use crate::units::business::versions::api::ResolveTake;
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    // 之后任何整文件读取都会失败：生产资产路径必须走 stat / scan / ingest / copy。
    store.fail_times("read_under", 100);
    store.fail_times("read_object", 100);

    let sb = sandbox_of(&ws, "w", "a");
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    store.write_under(&sb, "note.md", b"v2").unwrap();
    let _ = ws.work_status("w", "a", &[]).unwrap();
    let _ = ws.work_versions("w", "a", "note.md").unwrap();
    let rep = ws
        .work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "v2", 2))
        .unwrap();
    assert_eq!(rep.changes.len(), 1);
    let _ = ws
        .work_resolve("w", "a", &resolve_req(&["note.md"], ResolveTake::Upstream))
        .unwrap();
    ws.work_restore("w", rep.id).unwrap();
}

/// 目的：提交按"根 + 增量"落盘，读树仍给出完整结果；work_status 的树由回溯重建。
/// 约束：只有根提交带整棵树，增量只记 changes。
#[test]
pub(crate) fn work_commits_store_roots_and_deltas_but_read_full_tree() {
    use crate::units::business::versions::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "a.md", b"1", 1, 0).unwrap();
    ws.work_commit_user("w", "b.md", b"2", 2, 0).unwrap();
    ws.work_commit_user("w", "c.md", b"3", 3, 0).unwrap();
    let sd = ws.roots("w", &[]).unwrap().store;

    let c1 = store.read_commit(&sd, 1).unwrap().unwrap();
    let c2 = store.read_commit(&sd, 2).unwrap().unwrap();
    let c3 = store.read_commit(&sd, 3).unwrap().unwrap();
    assert!(c1.tree.is_some(), "根提交带整棵树");
    assert!(c2.tree.is_none(), "增量提交不落整棵树");
    assert!(c3.tree.is_none(), "增量提交不落整棵树");
    assert_eq!((c1.depth, c2.depth, c3.depth), (0, 1, 2));
    assert_eq!((c2.base, c3.base), (1, 2));

    // 树由根 + 沿途增量重建：三份文件都在。
    let st = ws.work_status("w", "a", &[]).unwrap();
    let mut tree = st.tree.clone();
    tree.sort();
    assert_eq!(
        tree,
        vec!["a.md".to_string(), "b.md".to_string(), "c.md".to_string()]
    );
}

/// 目的：验成员侧工作工具执行者按席位面发放的名单现取——没发任何 `work_*` 就一个都不装。
#[test]
pub(crate) fn work_handlers_are_taken_only_when_the_seat_face_allows_them() {
    use crate::units::business::versions::api::work_handlers;

    let (ws, _dirs, _store) = work_ws();
    let sb = test_sandbox("a", &[]);
    assert!(
        work_handlers(&ws.versions, &sb, &[]).is_empty(),
        "一个 work_* 都没发就不装执行者"
    );
    assert!(
        work_handlers(&ws.versions, &sb, &["read".to_string()]).is_empty(),
        "只发别的工具也不装"
    );
    let allowed = vec!["read".to_string(), "work_pull".to_string()];
    let handlers = work_handlers(&ws.versions, &sb, &allowed);
    assert_eq!(handlers.len(), 1, "发了一个 work_* 就装一个执行者");
    assert!(handlers[0].owns("work_pull"));
    assert!(!handlers[0].owns("read"), "执行者不认领非 work_* 的名字");
}

/// 目的：status 的「有没有待推送的东西」按内容净改动算——内容收敛后不再显示“未推送 N 条”。
/// 约束：领先的空提交条数仍如实报出（辅助信息），但它不再冒充待推送。
#[test]
pub(crate) fn work_status_counts_content_not_pending_commits() {
    use crate::units::business::versions::api::{render_push, render_status};
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    let sb = sandbox_of(&ws, "w", "a");

    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!((st.pending, st.unpushed), (0, 0), "{:?}", st);

    store.write_under(&sb, "note.md", b"v1\n").unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "v1", 1))
        .unwrap();
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!((st.pending, st.unpushed), (1, 1), "{:?}", st);
    let text = render_status(&st);
    assert!(text.contains("未推送 1 项改动"), "{text}");

    // 推上去：工作线跟着主线快进，这里才是「已全部推送」。
    push(&ws, "w", "a", "v1", 1, 1).unwrap();
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!((st.pending, st.unpushed), (0, 0), "{:?}", st);
    assert!(
        render_status(&st).contains("已全部推送"),
        "{}",
        render_status(&st)
    );

    // 再把内容改出去又改回来：净改动为零，工作线却多了两条提交。
    store.write_under(&sb, "note.md", b"v2\n").unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "v2", 2))
        .unwrap();
    store.write_under(&sb, "note.md", b"v1\n").unwrap();
    ws.work_commit(
        "w",
        "a",
        "w--a",
        &commit_req(&["note.md"], &[], "回到 v1", 3),
    )
    .unwrap();

    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(st.pending, 0, "内容与主线一致：{:?}", st);
    assert_eq!(st.unpushed, 2, "领先两条净改动为零的提交：{:?}", st);
    let text = render_status(&st);
    assert!(text.contains("内容已与主线一致"), "{text}");
    assert!(
        !text.contains("未推送"),
        "内容收敛时不能说还有待推送：{text}"
    );

    let rep = push(&ws, "w", "a", "空推", 4, 2).unwrap();
    assert!(rep.commit.is_none(), "{:?}", rep);
    assert_eq!(rep.unpushed, 2, "{:?}", rep);
    let text = render_push(&rep);
    assert!(text.contains("不必再提交"), "{text}");
    assert!(
        !text.contains("先 work_commit"),
        "不能劝人再造空提交：{text}"
    );
}

/// 目的：status 给得出工作线的祖先提交与父提交——work_reset to= 与「撤回某条提交」都靠它定位。
/// 约束：只在工作线与主线不一致时列；推送后工作线快进到主线，不重复列主线「最近提交」。
#[test]
pub(crate) fn work_status_lists_the_work_line_with_parents() {
    use crate::units::business::versions::api::{ResetRequest, render_status};
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    let sb = sandbox_of(&ws, "w", "a");

    let st = ws.work_status("w", "a", &[]).unwrap();
    assert!(st.work_recent.is_empty(), "{:?}", st);
    assert!(
        !render_status(&st).contains("你的工作线最近提交"),
        "{}",
        render_status(&st)
    );

    store.write_under(&sb, "note.md", b"v1\n").unwrap();
    let id1 = ws
        .work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "v1", 1))
        .unwrap()
        .id;
    store.write_under(&sb, "note.md", b"v2\n").unwrap();
    let id2 = ws
        .work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "v2", 2))
        .unwrap()
        .id;

    let st = ws.work_status("w", "a", &[]).unwrap();
    let chain: Vec<(u64, Option<u64>, bool)> = st
        .work_recent
        .iter()
        .map(|c| (c.id, c.parent, c.pushed))
        .collect();
    assert_eq!(
        chain,
        vec![(id2, Some(id1), false), (id1, None, false)],
        "{:?}",
        st
    );
    let text = render_status(&st);
    assert!(
        text.contains(&format!("- {id2}（父 {id1}，未进主线）：v2")),
        "{text}"
    );

    // 回执点名的父提交就是能用的 reset 目标：撤掉 v2 这条提交。
    ws.work_reset(
        "w",
        "a",
        &ResetRequest {
            paths: Vec::new(),
            to: Some(id1),
            keep_changes: false,
        },
    )
    .unwrap();
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(
        st.work_recent.iter().map(|c| c.id).collect::<Vec<_>>(),
        vec![id1],
        "{:?}",
        st
    );

    // 退回被拒时指引的是 agent 拿得到的 work_status，不是呈现层的 work_log。
    let err = ws
        .work_reset(
            "w",
            "a",
            &ResetRequest {
                paths: Vec::new(),
                to: Some(id2),
                keep_changes: false,
            },
        )
        .unwrap_err();
    assert!(err.contains("work_status"), "{err}");
    assert!(!err.contains("work_log"), "{err}");

    // 推送后工作线快进到主线：与主线一致，不再重复列。
    push(&ws, "w", "a", "v1", 3, 1).unwrap();
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert!(st.work_recent.is_empty(), "{:?}", st);
}
