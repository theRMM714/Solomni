//! 目的：读门禁与判定——门禁快照、会话授权、compaction 后失效、差异判定。
//! 管：tracked 基线/当前版本区分、未观察路径拒绝、授权随 compaction 失效、移位与改动的区分。
//! 不管：提交与历史（`work/commit.rs`）、推送与差异（`work/push.rs`）。
//! 联动：`docs/storage/README.md`。

use crate::tests::workspace::support::*;

/// 目的：门禁快照只回答三件事——是否受管、基线版本、当前版本。
#[test]
pub(crate) fn work_gate_info_distinguishes_tracked_base_and_current() {
    use crate::kernel::api::hash::content_hash;
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "base.md", b"base\n", 1, 0)
        .unwrap();
    let base_hash = content_hash(b"base\n");
    let info = |path: &str| {
        ws.work_gate_info("w", "a", &[path.to_string()])
            .unwrap()
            .into_iter()
            .next()
            .unwrap()
    };

    // 主线有、沙箱还没有：受管，当前内容不存在（写之前得先 work_pull）。
    let it = info("base.md");
    assert!(it.tracked);
    assert_eq!(
        it.base.as_ref().map(|f| f.hash.clone()),
        Some(base_hash.clone())
    );
    assert!(it.current.is_none());

    // 拉进沙箱后：当前版本就是主线这一版。
    ws.work_pull("w", "a", &["base.md".to_string()]).unwrap();
    let it = info("base.md");
    assert_eq!(it.current.as_ref().map(|f| f.hash.clone()), Some(base_hash));

    // 沙箱里新建但还没提交：当前存在、不受管。
    let sb = sandbox_of(&ws, "w", "a");
    store.write_under(&sb, "draft.md", b"draft\n").unwrap();
    let it = info("draft.md");
    assert!(!it.tracked);
    assert!(it.current.is_some());
    assert!(it.base.is_none());

    // 自动存档只进 agent 的存档线，不算受管：下次写同一个 scratch 文件不受门禁拦。
    ws.work_commit_auto("w", "a", "s", &commit_req(&["draft.md"], &[], "草稿", 2))
        .unwrap()
        .expect("应当记一次存档");
    let it = info("draft.md");
    assert!(!it.tracked, "存档线不是发布线，不应把 scratch 变成受管");
    assert!(it.base.is_none());

    // 显式提交进工作线后才受管，基线取工作线这一版。
    ws.work_commit("w", "a", "s", &commit_req(&["draft.md"], &[], "草稿", 3))
        .unwrap();
    let it = info("draft.md");
    assert!(it.tracked);
    assert_eq!(
        it.base.as_ref().map(|f| f.hash.clone()),
        Some(content_hash(b"draft\n"))
    );
    assert_eq!(
        it.current.as_ref().map(|f| f.hash.clone()),
        Some(content_hash(b"draft\n"))
    );
}

/// 目的：会话门禁只认"当前版本的观察"，且 untracked 一律放行。
#[test]
pub(crate) fn session_gate_rejects_unobserved_and_stale_tracked_paths() {
    use crate::kernel::api::hash::content_hash;
    use crate::units::business::tools::api::{
        Coverage, LineRange, ObservationFact, ObservationSource,
    };
    use crate::units::business::versions::api::SessionWriteGate;
    use crate::units::business::workbench::api::{GateFault, Need, VersionRef, WriteGate};

    let (ws, _dirs, _store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "base.md", b"one\ntwo\n", 1, 0)
        .unwrap();
    let version = content_hash(b"one\ntwo\n");
    let fact = |version: String, coverage: Coverage| ObservationFact {
        schema: crate::units::business::tools::api::OBSERVATION_FORMAT,
        reply: 1,
        path: "base.md".to_string(),
        worktree: "a".to_string(),
        version,
        coverage,
        total_lines: 2,
        source: ObservationSource::Read,
    };
    let check = |facts: &[ObservationFact], path: &str, need: Need| {
        let gate = SessionWriteGate::new(&ws.versions, "w", "a", "a", facts, None);
        gate.check(path, need, None)
    };

    assert_eq!(
        check(&[], "base.md", Need::Observed),
        Err(GateFault::NotObserved)
    );
    let full = [fact(version.clone(), Coverage::Full)];
    assert_eq!(check(&full, "base.md", Need::Full), Ok(()));
    let partial = [fact(
        version.clone(),
        Coverage::Ranges(vec![LineRange::new(1, 1)]),
    )];
    assert_eq!(
        check(&partial, "base.md", Need::Full),
        Err(GateFault::NotFull { total: 2 })
    );
    assert_eq!(check(&partial, "base.md", Need::Observed), Ok(()));
    let stale = [fact(content_hash(b"older\n"), Coverage::Full)];
    assert!(matches!(
        check(&stale, "base.md", Need::Observed),
        Err(GateFault::Stale { .. })
    ));
    let mut legacy = fact(version.clone(), Coverage::Full);
    legacy.schema = 0;
    assert_eq!(
        check(&[legacy], "base.md", Need::Observed),
        Err(GateFault::NotObserved),
        "格式版本不认的事实要降级为没有观察"
    );
    assert_eq!(
        check(&[], "scratch.md", Need::Full),
        Ok(()),
        "untracked 不受门禁限制"
    );

    // 同一版本多次分段读：并集覆盖全文就等价于完整读。
    let split = [
        fact(
            version.clone(),
            Coverage::Ranges(vec![LineRange::new(1, 1)]),
        ),
        fact(
            version.clone(),
            Coverage::Ranges(vec![LineRange::new(2, 2)]),
        ),
    ];
    assert_eq!(check(&split, "base.md", Need::Full), Ok(()));
    let gap = [
        fact(
            version.clone(),
            Coverage::Ranges(vec![LineRange::new(1, 1)]),
        ),
        fact(
            version.clone(),
            Coverage::Ranges(vec![LineRange::new(1, 1)]),
        ),
    ];
    assert_eq!(
        check(&gap, "base.md", Need::Full),
        Err(GateFault::NotFull { total: 2 }),
        "重复读同一段补不上缺口"
    );

    // 读完全文后 edit 只改了第 2 行：新版本的观察只有第 2 行，write 仍被拒。
    let edited_version = content_hash(b"one\nTWO\n");
    let after_edit = [
        fact(version.clone(), Coverage::Full),
        fact(
            edited_version.clone(),
            Coverage::Ranges(vec![LineRange::new(2, 2)]),
        ),
    ];
    let edited = VersionRef {
        hash: edited_version,
        size: None,
    };
    let gate = SessionWriteGate::new(&ws.versions, "w", "a", "a", &after_edit, None);
    assert_eq!(
        gate.check("base.md", Need::Full, Some(&edited)),
        Err(GateFault::NotFull { total: 2 }),
        "edit 记的是改动行，不是全文"
    );
    assert_eq!(
        gate.check("base.md", Need::Observed, Some(&edited)),
        Ok(()),
        "后续 edit / patch Update 只需观察过新版本"
    );
}

/// 目的：压缩把读过的工具行移出上下文后，那条观察不再授权后一轮的写。
#[test]
pub(crate) fn compaction_drops_read_authorization() {
    use crate::kernel::api::hash::content_hash;
    use crate::units::business::tools::api::{
        Coverage, ObservationFact, ObservationSource, Observations,
    };
    use crate::units::business::versions::api::SessionWriteGate;
    use crate::units::business::workbench::api::{GateFault, Need, WriteGate};

    let (ws, _dirs, _store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "base.md", b"one\ntwo\n", 1, 0)
        .unwrap();
    let fact = ObservationFact {
        schema: crate::units::business::tools::api::OBSERVATION_FORMAT,
        reply: 10,
        path: "base.md".to_string(),
        worktree: "a".to_string(),
        version: content_hash(b"one\ntwo\n"),
        coverage: Coverage::Full,
        total_lines: 2,
        source: ObservationSource::Read,
    };
    let mut obs = Observations::default();
    obs.set_reply(10);
    obs.note_fact(fact);
    obs.retain_from(11);
    assert!(obs.facts().is_empty(), "压缩点之前的读要一并丢掉");
    let gate = SessionWriteGate::new(&ws.versions, "w", "a", "a", obs.facts(), None);
    assert_eq!(
        gate.check("base.md", Need::Full, None),
        Err(GateFault::NotObserved),
        "上下文里已经没有那段读了，不能凭历史授权覆盖"
    );
}

/// 目的：判决按内容——指纹相同 = SameVersion，行号移位但内容还在 = Shifted，改到读过的行 = Changed。
#[test]
pub(crate) fn judge_reads_separates_shifted_from_changed() {
    use crate::kernel::api::hash::content_hash;
    use crate::units::business::versions::api::{ReadQuery, ReadVerdict};
    let (ws, _dirs, _store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    let v1 = b"a\nb\nc\n";
    ws.work_commit_user("w", "note.md", v1, 1, 0).unwrap();
    let q = |from: u64, to: u64, blob: String| ReadQuery {
        path: "note.md".to_string(),
        blob,
        from,
        to,
        sandbox: false,
    };
    let old = content_hash(v1);

    // 内容与读的时候完全一样：SameVersion。
    assert_eq!(
        ws.judge_reads("w", "a", &[q(1, 3, old.clone())]).unwrap(),
        vec![ReadVerdict::SameVersion]
    );

    // 只改第 3 行：读 1-2 内容还在但版本变了 = Shifted，读 3-3 = Changed。
    ws.work_commit_user("w", "note.md", b"a\nb\nC\n", 2, 0)
        .unwrap();
    assert_eq!(
        ws.judge_reads(
            "w",
            "a",
            &[
                q(1, 2, old.clone()),
                q(3, 3, old.clone()),
                q(9, 9, old.clone())
            ]
        )
        .unwrap(),
        vec![
            ReadVerdict::Shifted,
            ReadVerdict::Changed,
            ReadVerdict::Shifted
        ],
        "区间不与改动相交 = 那次读仍然可信（只是行号可能移位）"
    );

    // 文件头插一行：行号全移了，但读过的那几行一个字没变 → Shifted，不是过期。
    ws.work_commit_user("w", "note.md", b"zero\na\nb\nC\n", 3, 0)
        .unwrap();
    assert_eq!(
        ws.judge_reads("w", "a", &[q(1, 2, old.clone())]).unwrap(),
        vec![ReadVerdict::Shifted]
    );

    // 文件被删：读过的东西不在了（这里先提交、还没推送，所以内容仍在 = SameVersion）。
    ws.work_commit_user("w", "gone.md", b"x\n", 4, 0).unwrap();
    assert_eq!(
        ws.judge_reads(
            "w",
            "a",
            &[ReadQuery {
                path: "gone.md".to_string(),
                blob: content_hash(b"x\n"),
                from: 1,
                to: 1,
                sandbox: false,
            }]
        )
        .unwrap(),
        vec![ReadVerdict::SameVersion]
    );
    ws.work_pull("w", "a", &["gone.md".to_string()]).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    _store.write_under(&sb, "draft.md", b"d").unwrap();
    // 存档只记沙箱快照：共享区里这个文件还在，所以读过它的内容还不算过期。
    ws.work_commit_auto("w", "a", "s", &commit_req(&["draft.md"], &[], "草稿", 5))
        .unwrap()
        .expect("应当记一次存档");
    assert_eq!(
        ws.judge_reads(
            "w",
            "a",
            &[ReadQuery {
                path: "gone.md".to_string(),
                blob: content_hash(b"x\n"),
                from: 1,
                to: 1,
                sandbox: false,
            }]
        )
        .unwrap(),
        vec![ReadVerdict::SameVersion]
    );
    // 发布之后共享区才真的没有这个文件——读过它内容的上下文立刻过期。
    crate::units::business::versions::ports::WorkStore::remove_under(&*_store, &sb, "gone.md")
        .unwrap();
    ws.work_commit("w", "a", "s", &commit_req(&[], &["gone.md"], "删", 6))
        .unwrap();
    // 工作线提交只是本地提交：共享区要到推送后才真的没有这个文件。
    push(&ws, "w", "a", "删", 6, 6).unwrap();
    assert_eq!(
        ws.judge_reads(
            "w",
            "a",
            &[ReadQuery {
                path: "gone.md".to_string(),
                blob: content_hash(b"x\n"),
                from: 1,
                to: 1,
                sandbox: false,
            }]
        )
        .unwrap(),
        vec![ReadVerdict::Changed]
    );
}

/// 目的：沙箱里的读按沙箱当前内容判——主副本改了不影响它，沙箱自己改了才过期。
#[test]
pub(crate) fn judge_reads_follow_the_sandbox_copy() {
    use crate::kernel::api::hash::content_hash;
    use crate::units::business::versions::api::{ReadQuery, ReadVerdict};
    use crate::units::business::versions::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    let v1 = b"one\ntwo\nthree\n";
    store.write_under(&sb, "draft.md", v1).unwrap();
    // 先记一次存档：正文进对象库，行区间才有得比（读工具读到的就是这类已入库内容）。
    ws.work_commit_auto("w", "a", "s", &commit_req(&["draft.md"], &[], "草稿", 1))
        .unwrap()
        .expect("应当记一次存档");
    let q = |from: u64, to: u64| ReadQuery {
        path: "draft.md".to_string(),
        blob: content_hash(v1),
        from,
        to,
        sandbox: true,
    };
    assert_eq!(
        ws.judge_reads("w", "a", &[q(1, 3)]).unwrap(),
        vec![ReadVerdict::SameVersion]
    );
    // 主副本里另有一份同名文件、内容不同：沙箱的读不该被它影响。
    ws.work_commit_user("w", "draft.md", b"main\n", 1, 0)
        .unwrap();
    assert_eq!(
        ws.judge_reads("w", "a", &[q(1, 3)]).unwrap(),
        vec![ReadVerdict::SameVersion],
        "主副本变了不代表沙箱里的读过期"
    );
    // 沙箱里只改第 2 行：读 1-1 内容还在 = Shifted，读 2-2 = Changed。
    store
        .write_under(&sb, "draft.md", b"one\nTWO\nthree\n")
        .unwrap();
    assert_eq!(
        ws.judge_reads("w", "a", &[q(1, 1), q(2, 2)]).unwrap(),
        vec![ReadVerdict::Shifted, ReadVerdict::Changed]
    );
    // 沙箱里删掉：读过的东西不在了。
    store.remove_under(&sb, "draft.md").unwrap();
    assert_eq!(
        ws.judge_reads("w", "a", &[q(1, 3)]).unwrap(),
        vec![ReadVerdict::Changed]
    );
}
