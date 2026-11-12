//! 目的：外部工作区的绑定、导入与磁盘回执。
//! 管：首次导入、跨会话写锁、pull 回执、根 meta 绑定、UNC 全链路、起 guest 失败撤销。
//! 不管：共享区提交算法（`work.rs`）。
//! 联动：`docs/storage/README.md`。

use super::super::prelude::*;
use super::support::*;

/// 目的：撤销一次准备只删**空**目录——建会话失败时不留残骸，也不碰用户的东西。
#[test]
pub(crate) fn workspace_discard_removes_only_empty_dirs() {
    use crate::capabilities::workspace::detail::FsWorkspace;
    let root = crate::tests::scratch("ws-discard");
    let ws = FsWorkspace::new(root.join("session"));
    // 准备出来 → 撤销：三个空目录都该消失。
    ws.prepare("w", &["a".to_string()]).expect("准备");
    ws.discard("w", &["a".to_string()]).expect("撤销");
    assert!(!root.join("session").join("w").exists(), "空目录要删掉");
    // 里面有东西 → 撤销什么都不该删。
    ws.prepare("w2", &["a".to_string()]).expect("准备");
    let keep = root.join("session").join("w2").join("a").join("notes.txt");
    std::fs::write(&keep, b"user data").expect("写文件");
    ws.discard("w2", &["a".to_string()]).expect("撤销");
    assert!(keep.is_file(), "有内容的目录绝不能删");
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：首次绑定时自动导入工作区已有文件成初始提交；VCS / 依赖目录不导入。
#[test]
pub(crate) fn import_root_seeds_initial_external_commit_without_vcs_dirs() {
    use crate::capabilities::workspace::api::Workspace;
    use crate::capabilities::workspace::detail::{FsWorkStore, FsWorkspace};
    let root = crate::tests::scratch("ws-external-import");
    let sessions = root.join("session");
    let external = root.join("outside");
    std::fs::create_dir_all(external.join(".git")).expect("建 .git");
    std::fs::create_dir_all(external.join("node_modules")).expect("建 node_modules");
    std::fs::create_dir_all(external.join("sub")).expect("建 sub");
    std::fs::write(external.join("a.txt"), b"a").expect("写 a");
    std::fs::write(external.join(".git").join("config"), b"git").expect("写 git");
    std::fs::write(external.join("node_modules").join("x.js"), b"x").expect("写 dep");
    std::fs::write(external.join("sub").join("b.md"), b"b").expect("写 b");
    let ws = crate::capabilities::workspace::service::WorkspaceService::new(
        Arc::new(FsWorkspace::new(sessions.clone())),
        Arc::new(FsWorkStore::new()),
    );
    ws.bind("w", Some(external.as_path())).expect("绑定");
    ws.prepare("w", &["a".to_string()]).expect("准备");
    let seeded = ws.work_import_root("w", 1).expect("初始导入");
    assert!(seeded.commit.is_some(), "首次导入必须产生初始提交");
    assert_eq!(seeded.changes.len(), 2, "只导入 a.txt 与 sub/b.md");
    let pulled = ws
        .work_pull("w", "a", &["a.txt".to_string()])
        .expect("拉取初始内容");
    assert_eq!(pulled.updated, vec!["a.txt".to_string()]);
    assert_eq!(
        std::fs::read(sessions.join("w").join("a").join("a.txt")).unwrap(),
        b"a"
    );
    let again = ws.work_import_root("w", 2).expect("已有 head");
    assert!(again.commit.is_none(), "已有 head 时无操作");
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：同一外部工作区的跨会话写闸共用一把锁；第二个写者必须等第一个释放。
#[test]
pub(crate) fn external_workspace_lock_is_shared_and_blocks_a_second_writer() {
    use crate::capabilities::workspace::detail::FsWorkspace;
    use crate::capabilities::workspace::ports::Workdirs;
    use std::sync::mpsc;
    use std::time::Duration;
    let root = crate::tests::scratch("ws-external-lock");
    let sessions = root.join("session");
    let external = root.join("outside");
    std::fs::create_dir_all(&external).expect("建外部目录");
    let ws = FsWorkspace::new(sessions.clone());
    ws.bind("w1", Some(external.as_path())).expect("绑定 1");
    ws.bind("w2", Some(external.as_path())).expect("绑定 2");
    let guard = ws.lock_shared("w1").expect("加锁").expect("外部要有锁");
    let (tx, rx) = mpsc::channel();
    let sessions2 = sessions.clone();
    let external2 = external.clone();
    let worker = std::thread::spawn(move || {
        let other = FsWorkspace::new(sessions2);
        other.bind("w2", Some(external2.as_path())).expect("绑定");
        let got = other.lock_shared("w2").map(|g| g.is_some());
        tx.send(got).unwrap();
    });
    assert!(
        rx.recv_timeout(Duration::from_millis(250)).is_err(),
        "第二个写者必须等这把工作区锁"
    );
    drop(guard);
    assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Ok(true));
    worker.join().unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：外部工作区的磁盘改动让 pull 回执点名、commit 拒绝静默覆盖；work_sync 显式采纳后 pull 才更新。
#[test]
pub(crate) fn external_workspace_surfaces_disk_changes_and_sync_adopts_them() {
    use crate::capabilities::workspace::api::{ChangeKind, Workspace};
    use crate::capabilities::workspace::detail::{FsWorkStore, FsWorkspace};
    let root = crate::tests::scratch("ws-external-sync");
    let sessions = root.join("session");
    let external = root.join("outside");
    std::fs::create_dir_all(&external).expect("建外部目录");
    let ws = crate::capabilities::workspace::service::WorkspaceService::new(
        Arc::new(FsWorkspace::new(sessions.clone())),
        Arc::new(FsWorkStore::new()),
    );
    ws.bind("w", Some(external.as_path())).expect("绑定");
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .expect("准备");
    let sandbox_a = sessions.join("w").join("a");
    std::fs::write(sandbox_a.join("note.md"), b"v1").expect("写沙箱");
    let first = ws
        .work_commit(
            "w",
            "a",
            "w--a",
            &commit_req(&["note.md"], &[], "加 note", 1),
        )
        .expect("提交")
        .id;
    assert_eq!(std::fs::read(external.join("note.md")).unwrap(), b"v1");

    // 产品之外改了工作区：pull 不拿旧对象覆盖沙箱，而是点名 external。
    std::fs::write(external.join("note.md"), b"v2").expect("外部改");
    // 状态也要把外部改动标出来（不靠拉取才发现）。
    let st = ws
        .work_status("w", "b", &["note.md".to_string()])
        .expect("status");
    assert_eq!(st.external, vec!["note.md".to_string()]);

    let pull = ws
        .work_pull("w", "b", &["note.md".to_string()])
        .expect("pull");
    assert_eq!(pull.external, vec!["note.md".to_string()]);
    assert!(pull.updated.is_empty());
    assert!(!sessions.join("w").join("b").join("note.md").exists());

    // 沙箱里也改了：提交被整次拒绝，不静默覆盖外部版本。
    std::fs::write(sandbox_a.join("note.md"), b"v3").expect("改沙箱");
    let err = ws
        .work_commit(
            "w",
            "a",
            "w--a",
            &commit_req(&["note.md"], &[], "改 note", 2),
        )
        .expect_err("外部改动后提交要拒绝");
    assert!(err.contains("工作区被外部改动"), "{}", err);

    // work_sync 把磁盘当前版本采纳成外部提交；pull 才拿到它。
    let synced = ws
        .work_sync("w", &["note.md".to_string()], 9)
        .expect("同步");
    assert!(synced.commit.is_some(), "有外部改动必须产生提交");
    assert_eq!(synced.changes.len(), 1);
    assert_eq!(synced.changes[0].kind, ChangeKind::Modify);
    let pulled = ws
        .work_pull("w", "b", &["note.md".to_string()])
        .expect("再 pull");
    assert_eq!(pulled.updated, vec!["note.md".to_string()]);
    assert_eq!(
        std::fs::read(sessions.join("w").join("b").join("note.md")).unwrap(),
        b"v2"
    );

    // 工作区独有的新文件也能被 work_sync 纳入，再被别人 pull 到。
    std::fs::write(external.join("extra.md"), b"user").expect("外部新增");
    let imported = ws
        .work_sync("w", &["extra.md".to_string()], 10)
        .expect("导入");
    assert_eq!(imported.changes.len(), 1);
    assert_eq!(imported.changes[0].kind, ChangeKind::Add);
    ws.work_pull("w", "b", &["extra.md".to_string()])
        .expect("拉取导入的文件");
    assert_eq!(
        std::fs::read(sessions.join("w").join("b").join("extra.md")).unwrap(),
        b"user"
    );

    // 工作区又被外部改过一次：回档会覆盖它，先拒绝并要求 work_sync（危险确认尚未接界面）。
    std::fs::write(external.join("note.md"), b"v4").expect("再次外部改");
    let err = ws
        .work_restore("w", first)
        .expect_err("外部改动时回档必须拒绝");
    assert!(err.contains("被外部改动"), "{}", err);
    ws.work_restore_force("w", first)
        .expect("用户危险确认后允许覆盖");
    assert_eq!(std::fs::read(external.join("note.md")).unwrap(), b"v1");

    // 回档只动受管路径：未跟踪文件永不被物化删除，受管路径按目标树恢复。
    std::fs::write(external.join("keep.txt"), b"keep").expect("未跟踪文件");
    ws.work_restore("w", first).expect("回档");
    assert_eq!(std::fs::read(external.join("keep.txt")).unwrap(), b"keep");
    assert_eq!(std::fs::read(external.join("note.md")).unwrap(), b"v1");
    assert!(
        !external.join("extra.md").exists(),
        "受管但目标树没有：删除"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：建会话时把外部工作区绑定写进根 meta；产品根内的工作区在创建期如实拒绝。
#[test]
pub(crate) fn conductor_records_external_workspace_and_rejects_overlap() {
    use crate::capabilities::workspace::detail::FsWorkspace;
    let root = crate::tests::scratch("ws-conductor-external");
    let product = root.join("product");
    let external = root.join("outside");
    std::fs::create_dir_all(&product).expect("建产品根");
    std::fs::create_dir_all(&external).expect("建外部目录");
    let gateway = gw(BTreeMap::new(), vec![]);
    let mut core = core_with_workdirs(
        product.clone(),
        vec![module_of("a")],
        gateway,
        Arc::new(FsWorkspace::new(product.join("session"))),
    );
    let mut spec = work("wx", WorkMode::Single, &["a"]);
    spec.workspace = Some(external.clone());
    let sid = core.create_work(spec).expect("外部工作区可建").sid;
    let (meta, _) = core.history_open(&sid).expect("会话");
    assert_eq!(
        meta.workspace.as_ref().map(|b| b.root.clone()),
        Some(crate::kernel::api::native(
            &external.canonicalize().expect("规范化外部路径")
        )),
        "绑定写进根会话 meta（规范化绝对路径）"
    );
    let session = product.join("session").join("wx");
    assert!(!session.join("work").exists(), "外部绑定不建会话内 work/");
    assert!(session.join(".work-root").is_file(), "布局投影已落盘");
    // 产品根内 / 与产品根互相包含的目录：创建期拒绝。
    let inside = product.join("inside");
    std::fs::create_dir_all(&inside).expect("建产品内目录");
    let mut bad = work("bad", WorkMode::Single, &["a"]);
    bad.workspace = Some(inside);
    let err = core.create_work(bad).expect_err("产品根内的工作区必须拒绝");
    assert!(err.contains("不能与产品目录互相包含"), "{}", err);
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：真机验收（默认跳过）：UNC 外部工作区的全链路——虚拟机档绑定落成 native UNC、写探测真的落在共享上、
///   首绑导入 / pull / commit / status / sync 都走真实共享、删会话不删外部；产品根的 UNC 别名必须按落点身份拒。
/// 约束：只在 SOLOMNI_UNC_ROOT 指向一个可写的 UNC 目录时执行（如 \\\\localhost\D$\\\\AIproject\\target\\unc）；
///   CI 没有 UNC，缺省打印 [探针] 如实跳过；用例自己在 UNC 根下建子目录并在结束时清掉。
#[cfg(windows)]
#[test]
pub(crate) fn unc_external_workspace_round_trip_on_a_real_share() {
    use crate::capabilities::conductor::service::workspace_root::{product_root, resolve};
    use crate::capabilities::workspace::api::{ChangeKind, Workspace};
    use crate::capabilities::workspace::detail::{FsWorkStore, FsWorkspace};
    use crate::capabilities::workspace::service::WorkspaceService;
    use crate::kernel::api::{Tier, is_network_path, native};
    use std::path::PathBuf;

    let probe = crate::kernel::detail::HostProbeAdapter;

    let unc = match std::env::var("SOLOMNI_UNC_ROOT") {
        Ok(v) if !v.trim().is_empty() => PathBuf::from(v.trim()),
        _ => {
            eprintln!(
                r"[探针] 未开启 UNC 外部工作区真机验收：把 SOLOMNI_UNC_ROOT 指向可写 UNC 目录后重跑（如 \\\\localhost\D$\\\\AIproject\\target\\unc）"
            );
            return;
        }
    };
    assert!(
        is_network_path(&unc),
        "SOLOMNI_UNC_ROOT 要是网络路径：{}",
        unc.display()
    );
    let base = unc.join(format!("solomni-unc-{}", std::process::id()));
    let external = base.join("proj");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&external).expect("建 UNC 外部目录");
    std::fs::write(external.join("existing.txt"), b"seeded").expect("写共享区已有文件");

    let root = crate::tests::scratch("ws-unc-host");
    let product = root.join("product");
    std::fs::create_dir_all(&product).expect("建产品根");
    let product = product_root(&product, &probe);

    // 1) 绑定层：虚拟机档接受 UNC；规范化结果仍是 native UNC（不是相对路径 UNC\server\…）。
    //    resolve 里的写探测会真的在共享上建删一个文件——能过就说明共享可写。
    let bound =
        resolve(&external, Tier::Vm, &product, &probe).expect("虚拟机档要接受 UNC 外部工作区");
    assert_eq!(
        bound,
        native(&external.canonicalize().expect("规范化 UNC 根"))
    );
    let text = bound.to_string_lossy().into_owned();
    assert!(
        text.starts_with(r"\\"),
        "绑定要保留 UNC 的 \\ 前缀：{}",
        text
    );
    assert!(
        !text.contains(r"\\?\"),
        "绑定不能留 verbatim 前缀：{}",
        text
    );

    // 2) 本机档：拒绝网络工作区，而且**不触网**（耗时是"没去连共享"的可判定证据）。
    let started = std::time::Instant::now();
    let err =
        resolve(&external, Tier::Host, &product, &probe).expect_err("本机档必须拒绝网络工作区");
    assert!(err.contains("本机档不能用网络工作区"), "{}", err);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(3),
        "本机档的拒绝不能触网：耗时 {:?}",
        started.elapsed()
    );

    let share_root = share_root_of(&unc).expect("UNC 根要能拆出 server 与 share");
    // 3) 产品根的 UNC 别名：同一物理目录换个写法，文本 starts_with 拦不住，要靠落点身份拒。
    match drive_share_alias(&share_root, &product) {
        Some(alias) => {
            let err =
                resolve(&alias, Tier::Vm, &product, &probe).expect_err("产品根的 UNC 别名必须拒");
            assert!(err.contains("互相包含"), "{}", err);
        }
        None => eprintln!("[探针] SOLOMNI_UNC_ROOT 不是盘符共享的写法：跳过产品根 UNC 别名断言"),
    }
    let err = resolve(&share_root, Tier::Vm, &product, &probe).expect_err("共享根要拒");
    assert!(err.contains("网络共享的根目录"), "{}", err);

    // 4) 真实共享上的版本化全链路：布局投影、首绑导入、pull / commit / status / sync。
    let sessions = root.join("session");
    let ws = WorkspaceService::new(
        Arc::new(FsWorkspace::new(sessions.clone())),
        Arc::new(FsWorkStore::new()),
    );
    ws.bind("u", Some(external.as_path())).expect("绑定");
    let projection = std::fs::read_to_string(sessions.join("u").join(".work-root"))
        .expect("读布局投影 .work-root");
    assert_eq!(projection.trim(), text, "布局投影要写成 native UNC");
    ws.prepare("u", &["a".to_string()]).expect("准备");
    let seeded = ws.work_import_root("u", 1).expect("首绑导入");
    assert!(seeded.commit.is_some(), "共享区已有文件要导成初始提交");
    let pulled = ws
        .work_pull("u", "a", &["existing.txt".to_string()])
        .expect("拉取导入内容");
    assert_eq!(pulled.updated, vec!["existing.txt".to_string()]);
    assert_eq!(
        std::fs::read(sessions.join("u").join("a").join("existing.txt")).unwrap(),
        b"seeded"
    );
    // 提交要真的落到 UNC 目录（写的是共享上的文件，不是会话里的副本）。
    std::fs::write(sessions.join("u").join("a").join("new.txt"), b"from-agent").expect("写沙箱");
    ws.work_commit(
        "u",
        "a",
        "u--a",
        &commit_req(&["new.txt"], &[], "加 new", 2),
    )
    .expect("提交");
    assert_eq!(
        std::fs::read(external.join("new.txt")).unwrap(),
        b"from-agent"
    );
    // 共享区被外部改：status / pull 点名，sync 采纳之后 pull 才更新。
    std::fs::write(external.join("existing.txt"), b"outside").expect("共享区改");
    let st = ws
        .work_status("u", "a", &["existing.txt".to_string()])
        .expect("状态");
    assert_eq!(st.external, vec!["existing.txt".to_string()]);
    let synced = ws
        .work_sync("u", &["existing.txt".to_string()], 3)
        .expect("同步");
    assert!(synced.commit.is_some(), "有外部改动必须产生提交");
    assert_eq!(synced.changes[0].kind, ChangeKind::Modify);
    ws.work_pull("u", "a", &["existing.txt".to_string()])
        .expect("再拉");
    assert_eq!(
        std::fs::read(sessions.join("u").join("a").join("existing.txt")).unwrap(),
        b"outside"
    );

    // 5) 删会话不删外部：会话目录整棵删掉，共享区文件原样。
    std::fs::remove_dir_all(sessions.join("u")).expect("删会话目录");
    assert!(!sessions.join("u").exists());
    assert_eq!(
        std::fs::read(external.join("new.txt")).unwrap(),
        b"from-agent"
    );
    assert_eq!(
        std::fs::read(external.join("existing.txt")).unwrap(),
        b"outside"
    );

    let _ = std::fs::remove_dir_all(&base);
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：外部绑定把共享区主副本放到会话目录之外，版本库与沙箱仍留在会话目录内。
///   删除会话目录不碰外部工作区。
#[test]
pub(crate) fn external_workspace_keeps_store_in_session_and_outside_untouched() {
    use crate::capabilities::workspace::detail::FsWorkspace;
    use crate::capabilities::workspace::ports::Workdirs;
    let root = crate::tests::scratch("ws-external");
    let external = root.join("outside");
    let sessions = root.join("session");
    std::fs::create_dir_all(&external).expect("建外部目录");
    let ws = FsWorkspace::new(sessions.clone());
    ws.bind("w", Some(external.as_path())).expect("绑定");
    ws.prepare("w", &["a".to_string()]).expect("准备");
    let session = sessions.join("w");
    assert!(!session.join("work").exists(), "外部绑定不在会话内建 work/");
    assert!(session.join(".work-root").is_file(), "布局投影要落盘");
    let roots = ws.roots("w", &["a".to_string()]).expect("根");
    assert_eq!(roots.shared, external, "共享区根 = 外部目录");
    assert_eq!(
        roots.session_root, session,
        "会话树根 = 会话目录（两套布局下都是布局事实，不由 shared.parent() 反推）"
    );
    assert_eq!(roots.store, session.join(".work"), "版本库仍在会话目录");
    assert_eq!(
        roots.agents.get("a"),
        Some(&session.join("a")),
        "沙箱仍在会话目录"
    );
    ws.write_work("w", "n.txt", b"hi").expect("写共享区");
    assert_eq!(std::fs::read(external.join("n.txt")).unwrap(), b"hi");
    assert!(ws.work_has("w", "n.txt"));
    let files = ws.list("w", &["a".to_string()]).expect("列文件");
    assert_eq!(files.work, vec!["n.txt".to_string()]);
    let usage = ws.usage("w", &["a".to_string()]).expect("用量");
    assert_eq!(usage.work.files, 1, "外部工作区计入本次会话的用量视图");
    // 删会话目录（会话删除的真实动作）不碰外部工作区。
    std::fs::remove_dir_all(&session).expect("删会话目录");
    assert_eq!(std::fs::read(external.join("n.txt")).unwrap(), b"hi");
    // 清掉绑定后回到产品内布局。
    let managed = sessions.join("m");
    ws.bind("m", None).expect("清绑定");
    ws.prepare("m", &[]).expect("准备");
    assert!(
        managed.join("work").is_dir(),
        "无绑定时仍是 session/<名>/work"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：会话树根在两套布局下都等于会话目录——虚拟机档的稳定挂载按它挂，不再从 shared.parent() 猜。
#[test]
pub(crate) fn session_root_is_the_session_dir_for_both_layouts() {
    use crate::capabilities::workspace::detail::FsWorkspace;
    use crate::capabilities::workspace::ports::Workdirs;
    let root = crate::tests::scratch("ws-session-root");
    let sessions = root.join("session");
    let external = root.join("outside");
    std::fs::create_dir_all(&external).expect("建外部目录");
    let ws = FsWorkspace::new(sessions.clone());
    // 内部布局：shared 就在会话树内，三处根同父。
    ws.prepare("managed", &["a".to_string()]).expect("准备");
    let inner = ws.roots("managed", &["a".to_string()]).expect("根");
    let managed = sessions.join("managed");
    assert_eq!(inner.session_root, managed, "内部布局：会话树根 = 会话目录");
    assert_eq!(
        inner.shared,
        managed.join("work"),
        "内部布局：共享区在会话树内"
    );
    assert_eq!(inner.store, managed.join(".work"), "版本库在会话树内");
    assert_eq!(
        inner.agents.get("a"),
        Some(&managed.join("a")),
        "私沙在会话树内"
    );
    assert_eq!(
        inner.shared.parent(),
        Some(inner.session_root.as_path()),
        "内部布局下 shared.parent() 恰好是会话树根"
    );
    assert!(inner.store.starts_with(&inner.session_root));
    for private in inner.agents.values() {
        assert!(private.starts_with(&inner.session_root));
    }
    // 外部绑定：shared 在产品外，但会话树根、版本库与私沙仍锚在会话目录。
    ws.bind("bound", Some(external.as_path())).expect("绑定");
    ws.prepare("bound", &["a".to_string()]).expect("准备");
    let outer = ws.roots("bound", &["a".to_string()]).expect("根");
    let bound = sessions.join("bound");
    assert_eq!(outer.session_root, bound, "外部布局：会话树根 = 会话目录");
    assert_eq!(outer.shared, external, "外部布局：共享区 = 外部根");
    assert_eq!(
        outer.store,
        bound.join(".work"),
        "外部布局：版本库仍在会话树内"
    );
    assert_eq!(
        outer.agents.get("a"),
        Some(&bound.join("a")),
        "外部布局：私沙仍在会话树内"
    );
    assert!(outer.store.starts_with(&outer.session_root));
    for private in outer.agents.values() {
        assert!(private.starts_with(&outer.session_root));
    }
    assert_ne!(
        outer.shared.parent(),
        Some(outer.session_root.as_path()),
        "外部布局下 shared.parent() 不是会话树根——这正是虚拟机档曾挂错的地方"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：首次导入后起 guest 失败（建会话失败）→ 撤销连 .work 一起清；外部工作区文件不动。
///   内部布局的变体：work/ 里有用户文件时撤销不删它，但产品自己的 .work 仍清掉。
#[test]
pub(crate) fn discard_removes_failed_store_and_keeps_external_files() {
    use crate::capabilities::workspace::api::Workspace;
    use crate::capabilities::workspace::detail::{FsWorkStore, FsWorkspace};
    use crate::capabilities::workspace::service::WorkspaceService;
    let root = crate::tests::scratch("ws-discard-failed-create");
    let sessions = root.join("session");
    let external = root.join("outside");
    std::fs::create_dir_all(&external).expect("建外部目录");
    std::fs::write(external.join("keep.txt"), b"user").expect("用户文件");
    let ws = WorkspaceService::new(
        Arc::new(FsWorkspace::new(sessions.clone())),
        Arc::new(FsWorkStore::new()),
    );
    // 外部绑定：走真实的首次导入，让 session/.work 真有对象与提交（模拟“导入完、起 guest 失败”）。
    ws.bind("w", Some(external.as_path())).expect("绑定");
    ws.prepare("w", &["a".to_string()]).expect("准备");
    let seeded = ws.work_import_root("w", 1).expect("首次导入");
    assert!(seeded.commit.is_some(), "首次导入必须落出初始提交");
    let session = sessions.join("w");
    assert!(session.join(".work").is_dir(), "版本库已落在会话目录");
    assert!(session.join(".work-root").is_file(), "布局投影已落盘");
    ws.discard("w", &["a".to_string()]).expect("撤销失败的准备");
    assert!(
        !session.exists(),
        "会话目录（含 .work 与 .work-root）要整棵清掉"
    );
    assert!(external.is_dir(), "撤销不删外部根");
    assert_eq!(
        std::fs::read(external.join("keep.txt")).unwrap(),
        b"user",
        "外部工作区里的用户文件必须保留"
    );
    // 内部布局变体：work/ 非空 → 会话目录保留、用户文件保留；.work 是产品自己的，仍清掉。
    ws.prepare("m", &["b".to_string()]).expect("准备");
    let managed = sessions.join("m");
    std::fs::write(managed.join("work").join("keep.txt"), b"user").expect("用户文件");
    std::fs::create_dir_all(managed.join(".work").join("commits")).expect("造版本库");
    std::fs::write(managed.join(".work").join("commits").join("1.json"), b"{}").expect("造提交");
    ws.discard("m", &["b".to_string()]).expect("撤销");
    assert!(managed.is_dir(), "work/ 有用户文件时会话目录保留");
    assert_eq!(
        std::fs::read(managed.join("work").join("keep.txt")).unwrap(),
        b"user",
        "撤销不删用户在 work/ 里的文件"
    );
    assert!(!managed.join(".work").exists(), "产品自己的版本库仍要清掉");
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：起 guest 失败后同名重建能重新做初始导入——不被上次残留的 .work head 跳过。
///   回归点：旧 .work（有 head）若留下，重建换外部根时会静默跳过导入，agent 看到的是上一次的内容。
#[test]
pub(crate) fn recreated_external_workspace_imports_again_after_failed_create() {
    use crate::capabilities::workspace::api::Workspace;
    use crate::capabilities::workspace::detail::{FsWorkStore, FsWorkspace};
    use crate::capabilities::workspace::service::WorkspaceService;
    let root = crate::tests::scratch("ws-external-recreate");
    let sessions = root.join("session");
    let external = root.join("outside");
    std::fs::create_dir_all(&external).expect("建外部目录");
    std::fs::write(external.join("a.txt"), b"a").expect("用户文件");
    let ws = WorkspaceService::new(
        Arc::new(FsWorkspace::new(sessions.clone())),
        Arc::new(FsWorkStore::new()),
    );
    // 第一次建：绑定 → 准备 → 初始导入（有 head）→ 起 guest 失败 → 撤销。
    ws.bind("w", Some(external.as_path())).expect("绑定");
    ws.prepare("w", &["a".to_string()]).expect("准备");
    let first = ws.work_import_root("w", 1).expect("首次导入");
    assert!(first.commit.is_some(), "第一次建会话要导入出初始提交");
    assert!(
        ws.work_head("w").expect("读 head").is_some(),
        "撤销前版本库有 head"
    );
    ws.discard("w", &["a".to_string()]).expect("撤销失败的准备");
    assert!(!sessions.join("w").exists(), "撤销后会话目录不存在");
    // 同名重建：上一次的 .work 必须已经没了，初始导入要重新发生而不是“已有 head”空转。
    ws.bind("w", Some(external.as_path())).expect("重新绑定");
    ws.prepare("w", &["a".to_string()]).expect("重新准备");
    let again = ws.work_import_root("w", 2).expect("重建后重新导入");
    assert!(
        again.commit.is_some(),
        "同名重建必须重新导入（旧 .work 的 head 不能让它空转）"
    );
    assert_eq!(
        again
            .changes
            .iter()
            .map(|c| c.path.as_str())
            .collect::<Vec<_>>(),
        vec!["a.txt"],
        "重新导入的是当前外部根里的文件"
    );
    assert!(
        ws.work_head("w").expect("读 head").is_some(),
        "重建后的版本库重新有 head"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：外部根被移走 / 删除时，工作区操作用同一句错误如实拒绝；会话目录本身还能清掉。
#[test]
pub(crate) fn missing_external_workspace_reports_same_error_and_discard_keeps_it_absent() {
    use crate::capabilities::workspace::detail::FsWorkspace;
    use crate::capabilities::workspace::ports::Workdirs;
    let root = crate::tests::scratch("ws-external-missing");
    let external = root.join("gone");
    let sessions = root.join("session");
    let ws = FsWorkspace::new(sessions.clone());
    std::fs::create_dir_all(&external).expect("建外部目录");
    ws.bind("w", Some(external.as_path())).expect("绑定");
    ws.prepare("w", &["a".to_string()]).expect("准备");
    std::fs::remove_dir_all(&external).expect("移走外部目录");
    let err = ws
        .roots("w", &["a".to_string()])
        .expect_err("外部根缺失要拒绝");
    assert!(err.contains("工作区已不存在"), "{}", err);
    assert!(!ws.work_has("w", "n.txt"), "缺失时上传冲突判定按不存在");
    assert!(
        ws.write_work("w", "n.txt", b"x").is_err(),
        "缺失时写入要拒绝"
    );
    ws.discard("w", &["a".to_string()]).expect("撤销");
    assert!(!sessions.join("w").exists(), "会话目录可清掉");
    assert!(!external.exists(), "撤销不会重建外部根");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
pub(crate) fn sandboxes_lookup_is_by_agent() {
    let boxes = crate::capabilities::workspace::api::Sandboxes {
        shared: abs(&["w", "work"]),
        session_root: abs(&["w"]),
        list: vec![test_sandbox("甲", &["a"])],
        codec: None,
    };
    assert!(boxes.for_agent("甲").is_some(), "按 agent 实例名取沙箱");
    assert!(boxes.for_agent("a").is_none(), "沙箱不再按模块 id 反查");
}
