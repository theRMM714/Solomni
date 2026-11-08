//! 工作区能力测试：沙箱寻址与共享区版本化。
//! 归属判据：钉的是**本能力的不变式**；顺手经过别处只是路径，不是归属。
use super::builders::*;
use super::prelude::*;

#[test]
pub(crate) fn sandbox_resolve_accepts_only_absolute_paths_inside_roots() {
    use crate::capabilities::workspace::domain::workspace::Place;
    let sb = test_sandbox("a1", &["data"]);
    // 绝对且在根内 → 通过（返回归一化后的绝对路径）
    let (place, path) = sb
        .resolve(&p(&["demo", "work", "notes", "a.txt"]))
        .expect("共享区可达");
    assert_eq!(place, Place::Shared);
    assert_eq!(path, abs(&["demo", "work", "notes", "a.txt"]));
    let (place, path) = sb
        .resolve(&p(&["demo", "a1", "b.txt"]))
        .expect("私有沙箱可达");
    assert_eq!(place, Place::Private);
    assert_eq!(path, abs(&["demo", "a1", "b.txt"]));
    let (place, path) = sb
        .resolve(&p(&["mods", "data", "c.txt"]))
        .expect("成员模块目录可达");
    assert_eq!(place, Place::Module("data".to_string()));
    assert_eq!(path, abs(&["mods", "data", "c.txt"]));
    // 允许的根就是这三处：错误文案要把它们列全
    let roots_line = s(&["demo", "work"]);
    // 拒绝：越界 / 非绝对路径（相对、裸文件名、带冒号前缀的伪路径）/ .. 与 . 段 / 空段 / 空
    let bad: Vec<String> = vec![
        p(&["outside", "x.txt"]),
        "a.txt".to_string(),
        "demo/work/a.txt".to_string(),
        "work:/a.txt".to_string(),
        "sandbox:/b.txt".to_string(),
        "module:data:/c.txt".to_string(),
        format!(
            "{}{}..{}x.txt",
            p(&["demo", "work"]),
            std::path::MAIN_SEPARATOR,
            std::path::MAIN_SEPARATOR
        ),
        format!(
            "{}{}a{}..{}b.txt",
            p(&["demo", "work"]),
            std::path::MAIN_SEPARATOR,
            std::path::MAIN_SEPARATOR,
            std::path::MAIN_SEPARATOR
        ),
        format!("{}//a.txt", p(&["demo", "work"])),
        "   ".to_string(),
    ];
    for b in &bad {
        let err = sb.resolve(b).unwrap_err();
        assert!(
            err.contains(&roots_line),
            "错误要把允许的真实根列全（{}）：{}",
            b,
            err
        );
    }
}

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
    use crate::kernel::api::{is_network_path, native, Tier};
    use std::path::PathBuf;

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
    let product = product_root(&product);

    // 1) 绑定层：虚拟机档接受 UNC；规范化结果仍是 native UNC（不是相对路径 UNC\server\…）。
    //    resolve 里的写探测会真的在共享上建删一个文件——能过就说明共享可写。
    let bound = resolve(&external, Tier::Vm, &product).expect("虚拟机档要接受 UNC 外部工作区");
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
    let err = resolve(&external, Tier::Host, &product).expect_err("本机档必须拒绝网络工作区");
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
            let err = resolve(&alias, Tier::Vm, &product).expect_err("产品根的 UNC 别名必须拒");
            assert!(err.contains("互相包含"), "{}", err);
        }
        None => eprintln!("[探针] SOLOMNI_UNC_ROOT 不是盘符共享的写法：跳过产品根 UNC 别名断言"),
    }
    let err = resolve(&share_root, Tier::Vm, &product).expect_err("共享根要拒");
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

/// 目的：从一个 UNC 目录取出它的共享根（\\server\share）——用来断言"共享根不能当工作区"。
#[cfg(windows)]
fn share_root_of(unc: &std::path::Path) -> Option<std::path::PathBuf> {
    let (server, share, _) = crate::kernel::api::unc_parts(unc)?;
    Some(std::path::PathBuf::from(format!(
        r"\\{}\{}",
        server.to_string_lossy(),
        share.to_string_lossy()
    )))
}

/// 目的：把本机盘符路径换成同一个盘符共享（X$）下的 UNC 写法——用来造"产品根的 UNC 别名"。
/// 返回：共享名不是 <字母>$ 或盘符对不上时为 None（调用方如实跳过这条断言）。
#[cfg(windows)]
fn drive_share_alias(unc: &std::path::Path, local: &std::path::Path) -> Option<std::path::PathBuf> {
    let (_, share, rest) = crate::kernel::api::unc_parts(unc)?;
    if !rest.as_os_str().is_empty() {
        return None;
    }
    let share = share.to_string_lossy().into_owned();
    let letter = share.strip_suffix('$')?;
    let mut letter_chars = letter.chars();
    let drive = letter_chars.next()?.to_ascii_uppercase();
    if letter_chars.next().is_some() {
        return None;
    }
    let local_text = local.to_str()?;
    let mut chars = local_text.chars();
    if chars.next()?.to_ascii_uppercase() != drive || chars.next()? != ':' {
        return None;
    }
    let tail = chars.as_str().trim_start_matches(std::path::MAIN_SEPARATOR);
    let mut alias = unc.to_path_buf();
    for part in tail.split(std::path::MAIN_SEPARATOR) {
        if !part.is_empty() {
            alias.push(part);
        }
    }
    Some(alias)
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

/// 目的：虚拟机档"能不能选"只看**机器能力**，与会话级选哪个虚拟根无关——Web 换档死锁的回归锚。
/// 约束：虚拟根只在保存 /「开始」时校验（构造承载判据时合并进去）。
#[test]
pub(crate) fn vm_availability_is_machine_only_and_base_is_session_level() {
    use crate::capabilities::vm::domain::exec::host_verified;
    use crate::tests::doubles::core_with_gateway_probe;
    // 虚拟根由 test_guest 造在 .vm/guest/<名> 下，并声明三平台客户端；探测用真实路径 + 本机可起虚拟机。
    crate::tests::test_guest("vm-avail");
    let mut core = core_with_gateway_probe(
        vec![module_of("a")],
        gw(BTreeMap::new(), Vec::new()),
        crate::tests::doubles::vm_capable_host_probe(),
    );
    let choices = core.tier_choices();
    let expected = host_verified(std::env::consts::OS);
    assert_eq!(
        choices.vm_available, expected,
        "vm_available 只看机器能力：{:?}",
        choices.vm_requirements
    );
    assert!(
        !choices.vm_requirements.iter().any(|r| r.id == "base"),
        "机器能力清单不含会话级虚拟根：{:?}",
        choices.vm_requirements
    );
    if !expected {
        return;
    }
    // 建一条本机档会话（没选虚拟根）：会话配置仍说虚拟机档可选——虚拟根只在保存/开始时校验。
    let spec = crate::tests::single_work("wl", &["a"]);
    core.create_work(spec).expect("建本机档会话");
    let cfg = core.session_config("wl").expect("配置视图");
    assert!(cfg.guest.is_none(), "新会话没有选虚拟根");
    assert!(
        cfg.vm_available,
        "机器能力齐就该能选：{:?}",
        cfg.vm_requirements
    );
    assert!(
        !cfg.vm_requirements.iter().any(|r| r.id == "base"),
        "配置视图的机器能力清单也不含虚拟根：{:?}",
        cfg.vm_requirements
    );
    // 建虚拟机档：没选虚拟根 → 如实拒绝；选了在场的一格 → 建得起来（虚拟根是会话级，不是机器能力）。
    let mut no_guest = crate::tests::single_work("wv", &["a"]);
    no_guest.tier = Tier::Vm;
    let err = core
        .create_work(no_guest)
        .expect_err("虚拟机档没选虚拟根要如实拒绝");
    assert!(err.contains("虚拟根"), "{}", err);
    let guest = crate::tests::test_guest("vm-avail");
    let mut with_guest = crate::tests::single_work("wv2", &["a"]);
    with_guest.tier = Tier::Vm;
    with_guest.guest = Some(guest.clone());
    core.create_work(with_guest)
        .expect("机器能力齐 + 虚拟根在场就该建得起来");
    let _ = std::fs::remove_dir_all(
        crate::tests::root_dir()
            .join(".vm")
            .join("guest")
            .join("vm-avail"),
    );
}

/// 虚拟机档的承载校验：前置条件不具备时，**创建与编辑都如实拒绝**（用户环境问题，不是选型问题）。
/// 与界面上的"能不能选"同源（`exec::tier_readiness`），两处不会各说各话。
#[test]
pub(crate) fn vm_tier_is_refused_when_the_machine_cannot_carry_it() {
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec!["[]".into()]);
    let gateway: Arc<dyn ChatGateway + Send + Sync> =
        Arc::new(gw(member.clone(), vec!["[]".into()]));
    let llm = test_llm(
        Arc::clone(&gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    let mut core = Conductor::new(
        crate::tests::root_dir(),
        registry_service(InMemorySettings::with_tier(Tier::Vm), Arc::clone(&llm)),
        test_history(),
        test_workspace(Arc::new(InMemoryWorkspace::new())),
        test_vm(),
        test_packages(Arc::new(InMemoryPackages::empty())),
        test_provision(),
        test_modules(Arc::new(VecSource(vec![module_of("a")]))),
        Arc::new(crate::capabilities::builds::api::NoBuilds),
        llm,
        test_tools_svc_with(
            Arc::new(SilentRunner),
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        ),
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        crate::tests::doubles::test_host_probe(),
        Arc::new(crate::capabilities::residents::api::NoResidents),
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    );
    // 创建路径的档位来自**用户的选择**（WorkSpec.tier；基础根留空）：成立与否随本机而定，
    // 这里钉的是**接线**——机器承载不了就必须拒绝，且什么都不留下。
    let default_vm = ExecSpec {
        tier: Tier::Vm,
        ..ExecSpec::default()
    };
    let mut vm_spec = work("vm-default", WorkMode::Single, &["a"]);
    vm_spec.tier = Tier::Vm;
    let opened = core.create_work(vm_spec);
    if exec::tier_readiness(
        &crate::tests::root_dir(),
        &default_vm,
        None,
        None,
        &crate::kernel::detail::HostProbeAdapter,
    )
    .ready()
    {
        opened.expect("本机能承载虚拟机档时不该拒绝");
    } else {
        let err = opened.expect_err("本机承载不了虚拟机档就不许建");
        assert!(err.contains("虚拟机档现在不可用"), "{}", err);
        assert!(!core.session_exists("vm-default"), "拒绝就该什么都不留下");
    }

    // 编辑路径：基础根由用户给定（这里给一个不存在的），所以这一条不随机器变——必须拒绝、档位保持原样。
    let gateway2: Arc<dyn ChatGateway + Send + Sync> = Arc::new(gw(member, vec!["[]".into()]));
    let llm2 = test_llm(
        Arc::clone(&gateway2),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    let mut core2 = Conductor::new(
        crate::tests::root_dir(),
        registry_service(InMemorySettings::new(), Arc::clone(&llm2)),
        test_history(),
        test_workspace(Arc::new(InMemoryWorkspace::new())),
        test_vm(),
        test_packages(Arc::new(InMemoryPackages::empty())),
        test_provision(),
        test_modules(Arc::new(VecSource(vec![module_of("a")]))),
        Arc::new(crate::capabilities::builds::api::NoBuilds),
        llm2,
        test_tools_svc_with(
            Arc::new(SilentRunner),
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        ),
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        crate::tests::doubles::test_host_probe(),
        Arc::new(crate::capabilities::residents::api::NoResidents),
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    );
    let sid = core2
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let err = core2
        .edit_session(
            &sid,
            SessionEdit {
                agents: vec![ConfigAgent {
                    name: "a".to_string(),
                    modules: vec!["a".to_string()],
                    model: String::new(),
                    chara: String::new(),
                }],
                tier: "vm".to_string(),
                guest: Some("definitely-not-a-real-guest".to_string()),
                pins: BTreeMap::new(),
                permission: None,
            },
        )
        .expect_err("虚拟根不在场就不许改入虚拟机档");
    assert!(
        err.contains("虚拟机档现在不可用") && err.contains("虚拟根"),
        "{}",
        err
    );
    let cfg = core2.session_config(&sid).unwrap();
    assert_eq!(cfg.tier, "host", "被拒后档位保持原样");
    // 界面读的是**已保存的**配置（用户没提交的 base 输入后端看不到），两个字段必须自洽：
    // 可用时没有理由、不可用时必须有理由——界面据此决定禁用与说明。
    assert_eq!(
        cfg.vm_available,
        cfg.vm_unavailable_reason.is_empty(),
        "能不能选与为什么不能选必须说同一件事"
    );
}

#[test]
pub(crate) fn runtime_report_is_tier_aware() {
    let cores = |pkgs: Arc<InMemoryPackages>, modules: Vec<Module>| {
        core_with_pkgs(
            modules,
            gw(BTreeMap::new(), vec!["[]".into()]),
            Arc::new(SilentRunner),
            Arc::new(FakeCatalog::new(vec!["m".to_string()])),
            Arc::new(InMemoryHistory::new()),
            Arc::new(InMemorySysIo::new()),
            pkgs,
        )
    };
    let core = cores(
        Arc::new(InMemoryPackages::empty()),
        vec![module_with_runtimes("a", &["no-such-capability"])],
    );
    let host = core.runtime_report(Tier::Host);
    assert_eq!(host.tier, "host");
    assert_eq!(
        host.declared.get("a"),
        Some(&vec!["no-such-capability".to_string()])
    );
    assert_eq!(
        host.missing.get("a"),
        Some(&vec!["no-such-capability".to_string()]),
        "宿主确实没有这个能力时照实报"
    );
    assert!(host.available.is_empty());
    assert!(
        matches!(
            host.diagnoses.as_slice(),
            [
                crate::capabilities::conductor::api::RuntimeDiagnosis::Package(
                    crate::capabilities::packages::api::Problem::Missing { .. }
                )
            ]
        ),
        "本机档缺能力也要如实诊断：{:?}",
        host.diagnoses
    );
    assert_eq!(core.runtime_report(Tier::Vm).diagnoses.len(), 1);
    // 包库里有包 = 缺失消失、诊断清空
    let with_pkg = cores(
        Arc::new(InMemoryPackages::with(&[
            "id: python\nversion: 3.12.4\nprefix: opt/rt/python3.12",
        ])),
        vec![module_with_runtimes("a", &["python"])],
    );
    let r = with_pkg.runtime_report(Tier::Vm);
    assert!(r.missing.is_empty(), "{:?}", r.missing);
    assert_eq!(r.available.get("python"), Some(&vec!["3.12.4".to_string()]));
    assert!(r.diagnoses.is_empty(), "{:?}", r.diagnoses);
}

#[test]
pub(crate) fn host_runtime_report_uses_effective_path() {
    let probe = Arc::new(FixedProbe {
        exes: vec!["python".to_string()],
        ..Default::default()
    });
    let core = core_with_pkgs_probe(
        vec![module_with_runtimes("a", &["python"])],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::new(InMemoryHistory::new()),
        Arc::new(InMemorySysIo::new()),
        Arc::new(InMemoryPackages::empty()),
        probe,
    );
    let host = core.runtime_report(Tier::Host);
    assert!(
        host.missing.is_empty(),
        "宿主 PATH 上有 python 就不算缺：{:?}",
        host.missing
    );
}

#[test]
pub(crate) fn module_without_runtime_is_denied_with_reason() {
    // 虚拟机档 + 空包库：模块声明的运行包没人提供 → 改入那一刻如实拒绝（缺包不再降级）。
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            TOOL_CALL.into(),
            "{\"type\":\"say\",\"text\":\"改用内置工具\"}".into(),
        ],
    );
    let mut mod_a = module_with_runtimes("a", &["python"]);
    mod_a
        .manifest
        .tools
        .insert("grep".to_string(), decl("python tools/grep.py"));
    let runner = Arc::new(RecordingRunner {
        calls: Mutex::new(Vec::new()),
        out: "ok".into(),
        ok: true,
    });
    // 设置里的默认档位是**创建**时的档位来源：这里用本机档建（虚拟机档现在一律不可选），
    // 建好之后再把这条件裁成"已存在的虚拟机档会话"。
    let hist = Arc::new(InMemoryHistory::new());
    let gateway: Arc<dyn ChatGateway + Send + Sync> = Arc::new(gw(member, vec!["[]".into()]));
    let llm = test_llm(
        Arc::clone(&gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    let mut core = Conductor::new(
        crate::tests::root_dir(),
        registry_service(InMemorySettings::with_tier(Tier::Host), Arc::clone(&llm)),
        test_history_of(Arc::clone(&hist)),
        test_workspace(Arc::new(InMemoryWorkspace::new())),
        test_vm(),
        test_packages(Arc::new(InMemoryPackages::empty())),
        test_provision(),
        test_modules(Arc::new(VecSource(vec![mod_a]))),
        Arc::new(crate::capabilities::builds::api::NoBuilds),
        llm,
        test_tools_svc_with(
            Arc::clone(&runner) as Arc<dyn ProcessRunner>,
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        ),
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        crate::tests::doubles::test_host_probe(),
        Arc::new(crate::capabilities::residents::api::NoResidents),
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    );
    // 虚拟机档 + 空包库 + 基础根没声明运行能力：模块声明的 python 没人提供 →
    // **改入那一刻就拒绝**（不再降级：没有工具用户完不成工作），一个进程都不落。
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    hist.force_tier(&sid, Tier::Vm);
    let guest = crate::tests::test_guest("module-without-runtime");
    let mut rebuild = edit_of(vec![("a", &["a"], "")]);
    rebuild.tier = "vm".to_string();
    rebuild.guest = Some(guest.clone());
    let refused = core.edit_session(&sid, rebuild).unwrap_err();
    assert!(
        refused.contains("python") && refused.contains("runtimes/"),
        "要说清缺哪个能力、怎么补：{}",
        refused
    );
    assert!(
        runner.calls.lock().expect("锁").is_empty(),
        "拒绝时不落任何进程"
    );
    // 同一个模块在本机档照旧执行（本机档不装载运行包）。
    let mut member2 = BTreeMap::new();
    member2.insert(
        "a".to_string(),
        vec![
            TOOL_CALL.into(),
            "{\"type\":\"say\",\"text\":\"跑完了\"}".into(),
        ],
    );
    let mut mod_b = module_with_runtimes("a", &["python"]);
    mod_b
        .manifest
        .tools
        .insert("grep".to_string(), decl("python tools/grep.py"));
    let runner2 = Arc::new(RecordingRunner {
        calls: Mutex::new(Vec::new()),
        out: "ok".into(),
        ok: true,
    });
    let mut core2 = core_with_runner(
        vec![mod_b],
        gw(member2, vec!["[]".into()]),
        Arc::clone(&runner2),
    );
    let sid2 = core2
        .create_work(work("w2", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    with_live(|l| core2.single_say(&sid2, "干活", l)).unwrap();
    assert_eq!(
        runner2.calls.lock().expect("锁").len(),
        1,
        "本机档不受包库影响"
    );
}

// ---------- 配置视图：读、改、冻结 ----------

#[test]
pub(crate) fn session_config_reports_tier_missing_and_runtimes_dir() {
    let hist = Arc::new(InMemoryHistory::new());
    seed_session(
        &hist,
        "w",
        "single",
        vec![agent_meta("a", &["a"], Some("m"))],
        ExecSpec {
            tier: Tier::Vm,
            ..Default::default()
        },
    );
    let core = core_with_pkgs(
        vec![module_with_runtimes("a", &["python"])],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
        Arc::new(InMemoryPackages::empty()),
    );
    let cfg = core.session_config("w").unwrap();
    assert_eq!(cfg.sid, "w");
    assert_eq!(cfg.mode, "single");
    assert!(!cfg.started, "没有内容的会话 = 还没开过");
    assert_eq!(cfg.tier, "vm");
    assert_eq!(cfg.agents[0].model, "m");
    assert_eq!(
        cfg.runtime.missing.get("a"),
        Some(&vec!["python".to_string()])
    );
    assert!(
        cfg.runtimes_dir.ends_with("/runtimes"),
        "{}",
        cfg.runtimes_dir
    );
    assert!(core.session_config("没有这个会话").is_err());
}

// ---------- 共享区版本化工作区（work_pull / work_commit / work_status / work_versions / work_resolve） ----------

type WorkFace = Arc<dyn crate::capabilities::workspace::api::Workspace + Send + Sync>;

/// 装配一个只用内存端口的工作区能力面：目录布局 + 版本库都是内存替身。
fn work_ws() -> (WorkFace, Arc<InMemoryWorkspace>, Arc<InMemoryWorkStore>) {
    let dirs = Arc::new(InMemoryWorkspace::new());
    let store = Arc::new(InMemoryWorkStore::new());
    let ws = test_workspace_store(dirs.clone(), store.clone());
    (ws, dirs, store)
}

fn commit_req(
    paths: &[&str],
    deletes: &[&str],
    message: &str,
    time: i64,
) -> crate::capabilities::workspace::api::CommitRequest {
    crate::capabilities::workspace::api::CommitRequest {
        paths: paths.iter().map(|s| s.to_string()).collect(),
        deletes: deletes.iter().map(|s| s.to_string()).collect(),
        message: message.to_string(),
        time,
        line: 0,
    }
}

fn resolve_req(
    paths: &[&str],
    take: crate::capabilities::workspace::api::ResolveTake,
) -> crate::capabilities::workspace::api::ResolveRequest {
    crate::capabilities::workspace::api::ResolveRequest {
        paths: paths.iter().map(|s| s.to_string()).collect(),
        take,
        expect_head: None,
    }
}

/// take=local 的请求要带当前主副本提交 id（并发窗口守卫）；测试统一从 work_head 取。
fn resolve_local_req(
    ws: &WorkFace,
    work: &str,
    paths: &[&str],
) -> crate::capabilities::workspace::api::ResolveRequest {
    crate::capabilities::workspace::api::ResolveRequest {
        paths: paths.iter().map(|s| s.to_string()).collect(),
        take: crate::capabilities::workspace::api::ResolveTake::Local,
        expect_head: ws.work_head(work).unwrap(),
    }
}

fn sandbox_of(ws: &WorkFace, work: &str, agent: &str) -> PathBuf {
    ws.roots(work, &[agent.to_string()])
        .unwrap()
        .agents
        .get(agent)
        .cloned()
        .unwrap()
}

fn shared_of(ws: &WorkFace, work: &str) -> PathBuf {
    ws.roots(work, &[]).unwrap().shared
}

/// 三方比较的核心承诺：a、b 同基线各改同一文件，后者提交必须**报冲突**而不是覆盖；
/// 任一提交点都能还原出当时的共享区内容。
#[test]
pub(crate) fn work_commit_reports_conflicts_and_restore_returns_a_commit_point() {
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();

    // a、b 都对准同一版（v1）。
    assert_eq!(
        ws.work_pull("w", "a", &["note.md".to_string()])
            .unwrap()
            .updated,
        vec!["note.md"]
    );
    assert_eq!(
        ws.work_pull("w", "b", &["note.md".to_string()])
            .unwrap()
            .updated,
        vec!["note.md"]
    );

    // a 改并提交 → 提交点 2。
    let sb_a = sandbox_of(&ws, "w", "a");
    store.write_under(&sb_a, "note.md", b"v2").unwrap();
    let rep = ws
        .work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "a 改", 2))
        .expect("a 的提交");
    assert_eq!(rep.id, 2);

    // b 基于 v1 改成 v3：上游已变 → 冲突，整个提交被拒。
    let sb_b = sandbox_of(&ws, "w", "b");
    store.write_under(&sb_b, "note.md", b"v3").unwrap();
    let err = ws
        .work_commit("w", "b", "w--b", &commit_req(&["note.md"], &[], "b 改", 3))
        .unwrap_err();
    assert!(
        err.contains("note.md") && err.contains("冲突"),
        "要逐条点名冲突：{}",
        err
    );
    // 冲突没有留下半个提交：head 仍在 2。
    assert_eq!(
        store.head(&ws.roots("w", &[]).unwrap().store).unwrap(),
        Some(2)
    );

    // 还原到提交点 1：共享区内容回到 v1。
    ws.work_restore("w", 1).unwrap();
    let shared = shared_of(&ws, "w");
    assert_eq!(
        store.read_under(&shared, "note.md").unwrap().as_deref(),
        Some(&b"v1"[..])
    );
}

/// 目的：提交历史与提交详情：列表按新→旧、on_head 标当前线；详情给改动大小；文件预览文本/二进制分开。
/// 约束：非破坏性回滚后旧提交仍在列表里（on_head=false），从旧点继续提交会形成新分支。
#[test]
pub(crate) fn work_log_and_commit_preview_keep_off_line_commits_visible() {
    let (ws, _dirs, _store) = work_ws();
    ws.prepare("w", &[]).unwrap();
    let id1 = ws.work_commit_user("w", "a.txt", b"one", 10, 0).unwrap();
    let id2 = ws
        .work_commit_user("w", "b.bin", &[0, 1, 2], 11, 0)
        .unwrap();

    let log = ws.work_log("w", 10, None).unwrap();
    assert_eq!(log.head.as_ref().map(|h| h.id), Some(id2));
    assert_eq!(
        log.commits.iter().map(|c| c.id).collect::<Vec<_>>(),
        vec![id2, id1]
    );
    assert!(
        log.commits.iter().all(|c| c.on_head),
        "两个提交都在当前线上"
    );
    assert_eq!(log.total, 2);
    assert!(!log.truncated);

    let d = ws.work_commit_view("w", id1).unwrap();
    assert_eq!(d.changes.len(), 1);
    assert_eq!(d.changes[0].path, "a.txt");
    assert_eq!(d.changes[0].size, 3);
    assert_eq!(d.changes[0].media_type.as_deref(), Some("text/plain"));
    let f = ws.work_commit_file("w", id1, "a.txt").unwrap();
    assert_eq!(f.text.as_deref(), Some("one"));
    assert!(!f.truncated && !f.too_large);
    let b = ws.work_commit_file("w", id2, "b.bin").unwrap();
    assert_eq!(b.text, None, "含 NUL 的二进制不给文本预览");
    assert_eq!(b.size, 3);
    assert!(
        ws.work_commit_file("w", id1, "nope.txt").is_err(),
        "路径必须真在这条提交的树里"
    );

    // 非破坏性回滚：head 回到 id1，id2 仍在历史里，但标成"已离开当前线"。
    ws.work_restore("w", id1).unwrap();
    let back = ws.work_log("w", 10, None).unwrap();
    assert_eq!(back.head.as_ref().map(|h| h.id), Some(id1));
    assert_eq!(
        back.commits
            .iter()
            .map(|c| (c.id, c.on_head))
            .collect::<Vec<_>>(),
        vec![(id2, false), (id1, true)]
    );

    // 从旧点继续提交：新提交的 parent = id1，旧 id2 仍保留（可再次回滚）。
    let id3 = ws.work_commit_user("w", "a.txt", b"three", 12, 0).unwrap();
    let branched = ws.work_log("w", 10, None).unwrap();
    assert_eq!(branched.head.as_ref().map(|h| h.id), Some(id3));
    assert_eq!(
        branched
            .commits
            .iter()
            .map(|c| (c.id, c.on_head))
            .collect::<Vec<_>>(),
        vec![(id3, true), (id2, false), (id1, true)]
    );
    assert_eq!(branched.commits[0].parent, Some(id1));

    // before 分页：只取 id 更小的提交。
    let page = ws.work_log("w", 1, Some(id3)).unwrap();
    assert_eq!(
        page.commits.iter().map(|c| c.id).collect::<Vec<_>>(),
        vec![id2]
    );
    assert!(page.truncated, "后面还有 id1");
}

/// 按行锚精确物化共享区：agent 第 1 行提交 v1、第 3 行提交 v3；
/// 回档到第 1 行 → 共享区回到 v1；再丢弃非祖先提交 → 第 3 行的提交记录真的没了。
#[test]
pub(crate) fn work_rewind_to_materializes_by_line_anchor_and_discards_later_commits() {
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    let shared = shared_of(&ws, "w");
    let root = ws.roots("w", &[]).unwrap().store;

    // 第 1 行：a 提交 v1。
    store.write_under(&sb, "note.md", b"v1").unwrap();
    let mut r1 = commit_req(&["note.md"], &[], "v1", 1);
    r1.line = 1;
    let id1 = ws.work_commit("w", "a", "w--a", &r1).unwrap().id;

    // 第 3 行：a 再提交 v3。
    store.write_under(&sb, "note.md", b"v3").unwrap();
    let mut r3 = commit_req(&["note.md"], &[], "v3", 3);
    r3.line = 3;
    let id3 = ws.work_commit("w", "a", "w--a", &r3).unwrap().id;
    assert_eq!((id1, id3), (1, 2));

    // 回档到第 1 行（保留行 = 1）：只保留锚 ≤ 1 的提交 → v1。
    let mut keep = std::collections::BTreeMap::new();
    keep.insert("a".to_string(), 1u64);
    assert_eq!(ws.work_head("w").unwrap(), Some(id3));
    assert_eq!(ws.work_rewind_to("w", &keep).unwrap(), Some(id1));
    assert_eq!(
        store.read_under(&shared, "note.md").unwrap().as_deref(),
        Some(&b"v1"[..])
    );
    assert_eq!(ws.work_head("w").unwrap(), Some(id1));

    // 丢弃第 1 行之后的提交：第 3 行的提交记录真的没了。
    ws.work_discard_after("w", Some(id1)).unwrap();
    assert_eq!(store.list_commits(&root).unwrap(), vec![id1]);

    // 恢复到空共享区。
    ws.work_restore_point("w", None).unwrap();
    assert!(store.read_under(&shared, "note.md").unwrap().is_none());
    assert_eq!(ws.work_head("w").unwrap(), None);
}

/// 目的：用户回滚共享区后，新开的 agent 拉取到的是目标提交点的内容。
#[test]
pub(crate) fn work_restore_is_visible_to_a_new_agent_on_pull() {
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    let id1 = ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    let id2 = ws.work_commit_user("w", "note.md", b"v2", 2, 0).unwrap();
    assert_ne!(id1, id2);

    // 回滚到 v1；b 从没拉取过，之后首次拉取必须看到 v1。
    ws.work_restore("w", id1).unwrap();
    let pulled = ws
        .work_pull("w", "b", &["note.md".to_string()])
        .expect("新 agent 首次拉取");
    assert_eq!(
        pulled.updated,
        vec!["note.md"],
        "回滚后的 head 内容写进新沙箱"
    );
    let sb = sandbox_of(&ws, "w", "b");
    assert_eq!(
        store.read_under(&sb, "note.md").unwrap().as_deref(),
        Some(&b"v1"[..])
    );
}

/// 目的：回滚离开过的提交不在当前 head 祖先链上：转录回档只沿当前线选点，不能把旧分支选回来。
#[test]
pub(crate) fn work_rewind_to_ignores_commits_off_the_current_head() {
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    let shared = shared_of(&ws, "w");

    store.write_under(&sb, "a.txt", b"one").unwrap();
    let mut r1 = commit_req(&["a.txt"], &[], "一", 1);
    r1.line = 1;
    let id1 = ws.work_commit("w", "a", "w--a", &r1).unwrap().id;

    store.write_under(&sb, "a.txt", b"two").unwrap();
    let mut r2 = commit_req(&["a.txt"], &[], "二", 2);
    r2.line = 3;
    let id2 = ws.work_commit("w", "a", "w--a", &r2).unwrap().id;

    // 回到 id1，再从当前线继续提交（先拉齐沙箱，否则会被三方比较判冲突）。
    ws.work_restore("w", id1).unwrap();
    ws.work_pull("w", "a", &["a.txt".to_string()]).unwrap();
    store.write_under(&sb, "a.txt", b"three").unwrap();
    let mut r3 = commit_req(&["a.txt"], &[], "三", 3);
    r3.line = 5;
    let id3 = ws.work_commit("w", "a", "w--a", &r3).unwrap().id;

    // 保留行 4：id2 的锚 3 会通过，但它已离开当前线；当前线上 id1(1) 通过、id3(5) 不过。
    let mut keep = std::collections::BTreeMap::new();
    keep.insert("a".to_string(), 4u64);
    assert_eq!(ws.work_head("w").unwrap(), Some(id3));
    assert_eq!(ws.work_rewind_to("w", &keep).unwrap(), Some(id1));
    assert_eq!(
        store.read_under(&shared, "a.txt").unwrap().as_deref(),
        Some(&b"one"[..])
    );
    assert_eq!(ws.work_head("w").unwrap(), Some(id1));
    // id2 / id3 仍是历史（非破坏性）；这里只是不再被选为当前点。
    let log = ws.work_log("w", 10, None).unwrap();
    assert_eq!(
        log.commits
            .iter()
            .map(|c| (c.id, c.on_head))
            .collect::<Vec<_>>(),
        vec![(id3, false), (id2, false), (id1, true)]
    );
}

/// 用户投喂 = 权威提交；agent 拉取后按基线提交，新路径是新增。
#[test]
pub(crate) fn work_user_commit_is_authoritative_and_pull_tracks_the_baseline() {
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    // 权威覆盖：同名再投喂直接成为新的 head，不报冲突。
    let id = ws.work_commit_user("w", "note.md", b"v2", 2, 0).unwrap();
    assert_eq!(id, 2);

    let r = ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    assert_eq!(r.updated, vec!["note.md"]);
    let sb = sandbox_of(&ws, "w", "a");
    assert_eq!(
        store.read_under(&sb, "note.md").unwrap().as_deref(),
        Some(&b"v2"[..])
    );

    // 沙箱里改成本地基线之上的新内容 → modify。
    store.write_under(&sb, "note.md", b"v3").unwrap();
    let rep = ws
        .work_commit(
            "w",
            "a",
            "w--a",
            &commit_req(&["note.md"], &[], "改成 v3", 3),
        )
        .unwrap();
    assert_eq!(rep.changes.len(), 1);
    assert_eq!(rep.changes[0].path, "note.md");
}

/// 拉取绝不静默覆盖本地已有内容；提交空集与空说明都如实拒绝。
#[test]
pub(crate) fn work_pull_never_clobbers_and_empty_commits_are_refused() {
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    let sb_a = sandbox_of(&ws, "w", "a");
    let sb_b = sandbox_of(&ws, "w", "b");
    const DRAFT: &[u8] = "本地草稿".as_bytes();
    const B_VERSION: &[u8] = "b 版".as_bytes();
    const USER_EDIT: &[u8] = "用户改".as_bytes();

    // 无基线的同沙箱路径：按"已占用"处理，不覆盖。
    store.write_under(&sb_a, "note.md", DRAFT).unwrap();
    let r = ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    assert_eq!(r.occupied, vec!["note.md"]);
    assert_eq!(
        store.read_under(&sb_a, "note.md").unwrap().as_deref(),
        Some(DRAFT)
    );

    // 有基线、本地改了、上游也改了 → 冲突，不覆盖本地。
    ws.work_pull("w", "b", &["note.md".to_string()]).unwrap();
    store.write_under(&sb_b, "note.md", B_VERSION).unwrap(); // 本地改
    ws.work_commit_user("w", "note.md", USER_EDIT, 2, 0)
        .unwrap(); // 上游也改
    let r = ws.work_pull("w", "b", &["note.md".to_string()]).unwrap();
    assert_eq!(r.conflicts.len(), 1);
    assert_eq!(r.conflicts[0].path, "note.md");
    assert_eq!(
        store.read_under(&sb_b, "note.md").unwrap().as_deref(),
        Some(B_VERSION)
    );

    // 空提交与空说明都被如实拒绝。
    let empty = ws
        .work_commit("w", "a", "w--a", &commit_req(&[], &[], "说明", 3))
        .unwrap_err();
    assert!(empty.contains("没有可提交的改动"), "{}", empty);
    let no_msg = ws
        .work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "   ", 3))
        .unwrap_err();
    assert!(no_msg.contains("提交说明"), "{}", no_msg);
    let bad_pull = ws.work_pull("w", "a", &[]).unwrap_err();
    assert!(bad_pull.contains("路径"), "{}", bad_pull);
}

/// 目的：pull 中途失败保留已完成部分的基线；上游再动后重拉不产生伪冲突。
#[test]
pub(crate) fn work_pull_persists_partial_baseline_on_later_failure() {
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "a.txt", b"a1", 1, 0).unwrap();
    ws.work_commit_user("w", "b.txt", b"b1", 2, 0).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    let both = ["a.txt".to_string(), "b.txt".to_string()];
    ws.work_pull("w", "a", &both).unwrap();

    ws.work_commit_user("w", "a.txt", b"a2", 3, 0).unwrap();
    ws.work_commit_user("w", "b.txt", b"b2", 4, 0).unwrap();
    store.fail_on("copy_object_to_under", "b.txt", 1);
    let err = ws.work_pull("w", "a", &both).unwrap_err();
    assert!(
        err.contains("b.txt") && err.contains("已写入/登记 1 个"),
        "{}",
        err
    );
    assert_eq!(
        store.read_under(&sb, "a.txt").unwrap().as_deref(),
        Some(&b"a2"[..]),
        "a.txt 已经写进沙箱，基线也必须已经推进"
    );

    ws.work_commit_user("w", "a.txt", b"a3", 5, 0).unwrap();
    let rep = ws.work_pull("w", "a", &both).unwrap();
    assert!(
        rep.conflicts.is_empty(),
        "部分基线没保留时这里会报伪冲突：{:?}",
        rep.conflicts
    );
    assert!(rep.updated.contains(&"a.txt".to_string()));
    assert!(rep.updated.contains(&"b.txt".to_string()));
}

/// work_status 把"本地有修改 / 上游有更新 / 冲突"分开说，而不是笼统一句。
#[test]
pub(crate) fn work_status_separates_local_and_upstream_changes() {
    use crate::capabilities::workspace::domain::workstore::StatusState;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    let sb = sandbox_of(&ws, "w", "a");

    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(st.entries.len(), 1);
    assert_eq!(st.entries[0].state, StatusState::Clean);

    store.write_under(&sb, "note.md", b"v2").unwrap();
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(st.entries[0].state, StatusState::LocalModified);

    ws.work_commit_user("w", "note.md", b"v3", 2, 0).unwrap();
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(st.entries[0].state, StatusState::Conflict);
    assert!(st.head.is_some());
    assert!(st.tree.contains(&"note.md".to_string()));
}

/// 删除必须显式：path 从沙箱消失不等于删共享文件；deletes 会真的从主副本删掉。
#[test]
pub(crate) fn work_delete_is_explicit_in_the_commit_request() {
    let (ws, _dirs, _store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();

    let rep = ws
        .work_commit("w", "a", "w--a", &commit_req(&[], &["note.md"], "删掉", 2))
        .unwrap();
    assert_eq!(rep.changes.len(), 1);
    assert_eq!(
        rep.changes[0].kind,
        crate::capabilities::workspace::api::ChangeKind::Delete
    );
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert!(!st.tree.contains(&"note.md".to_string()));
}

/// 目的：work_* 的 paths 入口把“写错”和“上游没有”分开报；空提交说清真实原因；
///   沙箱里删掉已拉取的文件时状态是“本地已删除”，不是“本地有修改”。
#[test]
pub(crate) fn work_paths_and_empty_commit_report_the_real_reason() {
    use crate::capabilities::workspace::domain::workstore::StatusState;
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();

    // 非法路径整条拒绝，不再降级成“主副本没有”。
    let bad = ws
        .work_pull("w", "a", &["../x.txt".to_string()])
        .unwrap_err();
    assert!(bad.contains("不能包含 . 或 .."), "{}", bad);
    let bad = ws
        .work_commit("w", "a", "w--a", &commit_req(&["../x.txt"], &[], "x", 2))
        .unwrap_err();
    assert!(bad.contains("不能包含 . 或 .."), "{}", bad);
    let bad = ws
        .work_status("w", "a", &["../x.txt".to_string()])
        .unwrap_err();
    assert!(bad.contains("不能包含 . 或 .."), "{}", bad);

    // 合法但不存在：work_status 如实回报 missing，不静默丢。
    let st = ws.work_status("w", "a", &["nope.txt".to_string()]).unwrap();
    assert_eq!(st.missing, vec!["nope.txt"]);

    // 上游删除、本地未改：空提交说“上游已删除”，不再说“与主副本一致”。
    ws.work_pull("w", "b", &["note.md".to_string()]).unwrap();
    ws.work_commit("w", "b", "w--b", &commit_req(&[], &["note.md"], "删", 2))
        .unwrap();
    let empty = ws
        .work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "重加", 3))
        .unwrap_err();
    assert!(empty.contains("上游已删除"), "{}", empty);
    assert!(!empty.contains("与主副本一致"), "{}", empty);

    // 本地删除（上游没动）→ 本地已删除。
    ws.prepare("w2", &["a".to_string()]).unwrap();
    ws.work_commit_user("w2", "note.md", b"v1", 1, 0).unwrap();
    ws.work_pull("w2", "a", &["note.md".to_string()]).unwrap();
    let sb = sandbox_of(&ws, "w2", "a");
    store.remove_under(&sb, "note.md").unwrap();
    let st = ws.work_status("w2", "a", &[]).unwrap();
    assert_eq!(st.entries[0].state, StatusState::LocalDeleted);
}

/// 目的：work_versions 给文本预览、二进制只给指纹与大小；不把二进制冒充文本。
#[test]
pub(crate) fn work_versions_reports_text_and_binary_without_guessing() {
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
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
    ws.prepare("w", &["a".to_string()]).unwrap();
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
    use crate::capabilities::workspace::api::{ResolveAction, ResolveTake};
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
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
    let err = ws
        .work_commit("w", "b", "w--b", &commit_req(&["clip.mp4"], &[], "b", 3))
        .unwrap_err();
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
    use crate::capabilities::workspace::api::ResolveTake;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
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

/// 目的：两边都改时，人工合并后用 take=local 推进基线，提交成功且主副本拿到合并结果。
#[test]
pub(crate) fn work_resolve_local_marks_merged_result_and_commit_succeeds() {
    use crate::capabilities::workspace::api::ResolveAction;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    let sb_a = sandbox_of(&ws, "w", "a");
    let sb_b = sandbox_of(&ws, "w", "b");
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    ws.work_pull("w", "b", &["note.md".to_string()]).unwrap();
    store.write_under(&sb_a, "note.md", b"a-change").unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "a", 2))
        .unwrap();
    store.write_under(&sb_b, "note.md", b"merged").unwrap();
    assert!(ws
        .work_commit("w", "b", "w--b", &commit_req(&["note.md"], &[], "b", 3))
        .is_err());

    let rep = ws
        .work_resolve("w", "b", &resolve_local_req(&ws, "w", &["note.md"]))
        .unwrap();
    assert_eq!(rep.entries[0].action, ResolveAction::Marked);
    let commit = ws
        .work_commit("w", "b", "w--b", &commit_req(&["note.md"], &[], "合并", 4))
        .unwrap();
    assert_eq!(commit.changes[0].path, "note.md");
    assert_eq!(
        store
            .read_under(&shared_of(&ws, "w"), "note.md")
            .unwrap()
            .as_deref(),
        Some(&b"merged"[..])
    );
}

/// 目的：take=upstream 用主副本覆盖本地；覆盖后与主副本一致，不需要再提交。
#[test]
pub(crate) fn work_resolve_upstream_overwrites_local_and_then_is_clean() {
    use crate::capabilities::workspace::api::{ResolveAction, ResolveTake};
    use crate::capabilities::workspace::domain::workstore::StatusState;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    let sb_a = sandbox_of(&ws, "w", "a");
    let sb_b = sandbox_of(&ws, "w", "b");
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    ws.work_pull("w", "b", &["note.md".to_string()]).unwrap();
    store.write_under(&sb_a, "note.md", b"a-change").unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "a", 2))
        .unwrap();
    store.write_under(&sb_b, "note.md", b"b-change").unwrap();

    let rep = ws
        .work_resolve("w", "b", &resolve_req(&["note.md"], ResolveTake::Upstream))
        .unwrap();
    assert_eq!(rep.entries[0].action, ResolveAction::WroteUpstream);
    assert_eq!(
        store.read_under(&sb_b, "note.md").unwrap().as_deref(),
        Some(&b"a-change"[..])
    );
    let st = ws.work_status("w", "b", &[]).unwrap();
    assert_eq!(st.entries[0].state, StatusState::Clean);
    assert!(ws
        .work_commit("w", "b", "w--b", &commit_req(&["note.md"], &[], "noop", 3))
        .unwrap_err()
        .contains("没有可提交"));
}

/// 目的：上游删文件时，take=upstream 删本地并清基线；take=local 清基线、让本地成为新增。
#[test]
pub(crate) fn work_resolve_handles_upstream_deletion_by_side() {
    use crate::capabilities::workspace::api::{ResolveAction, ResolveTake};
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    ws.work_commit_user("w", "gone.txt", b"v1", 1, 0).unwrap();
    ws.work_commit_user("w", "keep.bin", b"bin1", 2, 0).unwrap();
    let sb_a = sandbox_of(&ws, "w", "a");
    let both = ["gone.txt".to_string(), "keep.bin".to_string()];
    ws.work_pull("w", "a", &both).unwrap();
    ws.work_pull("w", "b", &both).unwrap();
    store.write_under(&sb_a, "gone.txt", b"local").unwrap();
    store.write_under(&sb_a, "keep.bin", b"bin-local").unwrap();
    ws.work_commit(
        "w",
        "b",
        "w--b",
        &commit_req(&[], &["gone.txt", "keep.bin"], "上游删", 3),
    )
    .unwrap();
    let pulled = ws.work_pull("w", "a", &both).unwrap();
    assert_eq!(pulled.missing, both.to_vec());

    let gone = ws
        .work_resolve("w", "a", &resolve_req(&["gone.txt"], ResolveTake::Upstream))
        .unwrap();
    assert_eq!(gone.entries[0].action, ResolveAction::RemovedLocal);
    assert!(store.read_under(&sb_a, "gone.txt").unwrap().is_none());

    let keep = ws
        .work_resolve("w", "a", &resolve_local_req(&ws, "w", &["keep.bin"]))
        .unwrap();
    assert_eq!(keep.entries[0].action, ResolveAction::Cleared);
    assert_eq!(
        store.read_under(&sb_a, "keep.bin").unwrap().as_deref(),
        Some(&b"bin-local"[..]),
        "take=local 不能动本地文件"
    );
    let commit = ws
        .work_commit(
            "w",
            "a",
            "w--a",
            &commit_req(&["keep.bin"], &[], "保留为新增", 4),
        )
        .unwrap();
    assert_eq!(commit.changes[0].path, "keep.bin");
    assert_eq!(
        store
            .read_under(&shared_of(&ws, "w"), "keep.bin")
            .unwrap()
            .as_deref(),
        Some(&b"bin-local"[..])
    );
}

/// 目的：未拉取但沙箱有同名文件时状态直说，take=local 后可提交；未跟踪文件单列。
#[test]
pub(crate) fn work_status_exposes_both_present_and_local_only_then_resolve_clears_it() {
    use crate::capabilities::workspace::api::ResolveAction;
    use crate::capabilities::workspace::domain::workstore::StatusState;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    store.write_under(&sb, "note.md", b"local").unwrap();
    store.write_under(&sb, "extra.bin", &[0, 1, 2]).unwrap();

    let st = ws.work_status("w", "a", &[]).unwrap();
    let note = st
        .entries
        .iter()
        .find(|e| e.path == "note.md")
        .expect("主副本有的路径要出现");
    assert_eq!(note.state, StatusState::BothPresent);
    assert!(st.local_only.contains(&"extra.bin".to_string()));
    let only = ws
        .work_status("w", "a", &["extra.bin".to_string()])
        .unwrap();
    assert_eq!(only.entries[0].state, StatusState::LocalOnly);

    let rep = ws
        .work_resolve("w", "a", &resolve_local_req(&ws, "w", &["note.md"]))
        .unwrap();
    assert_eq!(rep.entries[0].action, ResolveAction::Marked);
    ws.work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "合并", 2))
        .unwrap();
    assert_eq!(
        store
            .read_under(&shared_of(&ws, "w"), "note.md")
            .unwrap()
            .as_deref(),
        Some(&b"local"[..])
    );
}

/// 目的：本地没改过时 take=local 必须拒绝；head token 过期也必须拒绝，且都不留改动。
#[test]
pub(crate) fn work_resolve_local_rejects_unmerged_and_stale_head() {
    use crate::capabilities::workspace::api::{ResolveAction, ResolveRequest, ResolveTake};
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    let sb_a = sandbox_of(&ws, "w", "a");
    let sb_b = sandbox_of(&ws, "w", "b");
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    ws.work_pull("w", "b", &["note.md".to_string()]).unwrap();
    store.write_under(&sb_a, "note.md", b"a-change").unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "a", 2))
        .unwrap();

    // b 本地仍是 v1：take=local 只会用旧内容覆盖上游，整次拒绝。
    let before = store.read_under(&sb_b, "note.md").unwrap();
    let err = ws
        .work_resolve("w", "b", &resolve_local_req(&ws, "w", &["note.md"]))
        .unwrap_err();
    assert!(err.contains("没有改过"), "{}", err);
    assert_eq!(store.read_under(&sb_b, "note.md").unwrap(), before);
    assert_eq!(ws.work_head("w").unwrap(), Some(2));

    // 带过期 head 的 resolve 同样拒绝：不能把基于上一版的合并登记到新版。
    let stale = ResolveRequest {
        paths: vec!["note.md".to_string()],
        take: ResolveTake::Local,
        expect_head: Some(1),
    };
    let err = ws.work_resolve("w", "b", &stale).unwrap_err();
    assert!(
        err.contains("主副本已从提交") && err.contains("2"),
        "{}",
        err
    );

    // 真正合并后再 resolve：成功，并记录本次基于的 head。
    store.write_under(&sb_b, "note.md", b"merged").unwrap();
    let rep = ws
        .work_resolve("w", "b", &resolve_local_req(&ws, "w", &["note.md"]))
        .unwrap();
    assert_eq!(rep.entries[0].action, ResolveAction::Marked);
    assert_eq!(rep.head, Some(2));
}

/// 目的：take=local 在“本地缺失 / 没有基线”时给出显式警告，但仍按既有契约执行。
#[test]
pub(crate) fn work_resolve_local_warns_when_local_missing_or_no_base() {
    use crate::capabilities::workspace::api::ResolveWarningKind;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    ws.work_commit_user("w", "other.md", b"o1", 2, 0).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    store.write_under(&sb, "other.md", b"local").unwrap();

    let rep = ws
        .work_resolve(
            "w",
            "a",
            &resolve_local_req(&ws, "w", &["note.md", "other.md"]),
        )
        .unwrap();
    assert_eq!(rep.entries.len(), 2);
    assert!(rep
        .warnings
        .iter()
        .any(|w| w.path == "note.md" && w.kind == ResolveWarningKind::LocalMissing));
    assert!(rep
        .warnings
        .iter()
        .any(|w| w.path == "other.md" && w.kind == ResolveWarningKind::NoBase));
}

/// 目的：take=upstream 覆盖前把本地版本备份到 lost/；work_lost 能列出并按当前 hash 恢复。
#[test]
pub(crate) fn work_resolve_upstream_backs_up_and_work_lost_restores() {
    use crate::capabilities::workspace::api::content_hash;
    use crate::capabilities::workspace::api::{ResolveAction, ResolveTake};
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    let sb_a = sandbox_of(&ws, "w", "a");
    let sb_b = sandbox_of(&ws, "w", "b");
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    ws.work_pull("w", "b", &["note.md".to_string()]).unwrap();
    store.write_under(&sb_a, "note.md", b"a-change").unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "a", 2))
        .unwrap();
    store.write_under(&sb_b, "note.md", b"b-change").unwrap();

    let rep = ws
        .work_resolve("w", "b", &resolve_req(&["note.md"], ResolveTake::Upstream))
        .unwrap();
    assert_eq!(rep.entries[0].action, ResolveAction::WroteUpstream);
    let backup = rep.entries[0].backup.as_deref().expect("要有备份路径");
    assert_eq!(backup, "lost/b/note.md");
    let backup_root = ws.roots("w", &[]).unwrap().store.join("lost").join("b");
    assert_eq!(
        store
            .read_under(&backup_root, "note.md")
            .unwrap()
            .as_deref(),
        Some(&b"b-change"[..])
    );
    assert_eq!(
        store.read_under(&sb_b, "note.md").unwrap().as_deref(),
        Some(&b"a-change"[..])
    );

    let lost = ws.work_lost("w", "b", None).unwrap();
    assert_eq!((lost.total, lost.entries.len()), (1, 1));
    let e = &lost.entries[0];
    assert_eq!(e.path, "note.md");
    assert_eq!(e.hash, content_hash(b"b-change"));
    let a_hash = content_hash(b"a-change");
    assert_eq!(e.current_hash.as_deref(), Some(a_hash.as_str()));

    // expect_current 不匹配：拒绝，沙箱不动。
    let err = ws
        .work_restore_lost("w", "b", "note.md", "not-the-current-hash")
        .unwrap_err();
    assert!(err.contains("expect_current"), "{}", err);
    assert_eq!(
        store.read_under(&sb_b, "note.md").unwrap().as_deref(),
        Some(&b"a-change"[..])
    );

    // 按当前 hash 恢复；备份仍保留。
    let current = e.current_hash.clone().unwrap();
    let restored = ws.work_restore_lost("w", "b", "note.md", &current).unwrap();
    assert_eq!(restored.previous.as_deref(), Some(current.as_str()));
    assert_eq!(
        store.read_under(&sb_b, "note.md").unwrap().as_deref(),
        Some(&b"b-change"[..])
    );
    assert_eq!(ws.work_lost("w", "b", None).unwrap().entries.len(), 1);

    // 当前文件不存在时，expect_current 传空串。
    store.remove_under(&sb_b, "note.md").unwrap();
    let restored = ws.work_restore_lost("w", "b", "note.md", "").unwrap();
    assert!(restored.previous.is_none());
    assert_eq!(
        store.read_under(&sb_b, "note.md").unwrap().as_deref(),
        Some(&b"b-change"[..])
    );
}

/// 目的：上游删除时 take=upstream 也要先备份本地版本，再删沙箱文件。
#[test]
pub(crate) fn work_resolve_upstream_backs_up_before_removing_local() {
    use crate::capabilities::workspace::api::{ResolveAction, ResolveTake};
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    ws.work_commit_user("w", "gone.txt", b"v1", 1, 0).unwrap();
    let sb_a = sandbox_of(&ws, "w", "a");
    ws.work_pull("w", "a", &["gone.txt".to_string()]).unwrap();
    ws.work_pull("w", "b", &["gone.txt".to_string()]).unwrap();
    store.write_under(&sb_a, "gone.txt", b"local").unwrap();
    ws.work_commit(
        "w",
        "b",
        "w--b",
        &commit_req(&[], &["gone.txt"], "上游删", 2),
    )
    .unwrap();

    let rep = ws
        .work_resolve("w", "a", &resolve_req(&["gone.txt"], ResolveTake::Upstream))
        .unwrap();
    assert_eq!(rep.entries[0].action, ResolveAction::RemovedLocal);
    let backup = rep.entries[0].backup.as_deref().expect("删除前也要备份");
    assert_eq!(backup, "lost/a/gone.txt");
    let backup_root = ws.roots("w", &[]).unwrap().store.join("lost").join("a");
    assert_eq!(
        store
            .read_under(&backup_root, "gone.txt")
            .unwrap()
            .as_deref(),
        Some(&b"local"[..])
    );
    assert!(store.read_under(&sb_a, "gone.txt").unwrap().is_none());
}

/// 目的：备份失败时整次 resolve 中止，不覆盖本地、不动基线。
#[test]
pub(crate) fn work_resolve_aborts_when_backup_fails() {
    use crate::capabilities::workspace::api::ResolveTake;
    use crate::capabilities::workspace::domain::workstore::StatusState;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    let sb_a = sandbox_of(&ws, "w", "a");
    let sb_b = sandbox_of(&ws, "w", "b");
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    ws.work_pull("w", "b", &["note.md".to_string()]).unwrap();
    store.write_under(&sb_a, "note.md", b"a-change").unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "a", 2))
        .unwrap();
    store.write_under(&sb_b, "note.md", b"b-change").unwrap();

    store.fail_times("copy_under_to_under", 1);
    let err = ws
        .work_resolve("w", "b", &resolve_req(&["note.md"], ResolveTake::Upstream))
        .unwrap_err();
    assert!(err.contains("copy_under_to_under"), "{}", err);
    assert_eq!(
        store.read_under(&sb_b, "note.md").unwrap().as_deref(),
        Some(&b"b-change"[..])
    );
    let st = ws.work_status("w", "b", &[]).unwrap();
    assert_eq!(st.entries[0].state, StatusState::Conflict);
}

/// 目的：空路径与三方都不存在的路径都被如实拒绝；部分存在时报告 missing。
#[test]
pub(crate) fn work_resolve_refuses_empty_and_reports_missing() {
    let (ws, _dirs, _store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    assert!(ws
        .work_resolve("w", "a", &resolve_local_req(&ws, "w", &[]))
        .is_err());
    assert!(ws
        .work_resolve("w", "a", &resolve_local_req(&ws, "w", &["nope"]))
        .is_err());
    let rep = ws
        .work_resolve("w", "a", &resolve_local_req(&ws, "w", &["note.md", "nope"]))
        .unwrap();
    assert_eq!(rep.entries.len(), 1);
    assert_eq!(rep.missing, vec!["nope"]);
}

/// 目的：用户投喂同样走事务；收尾失败后下一次操作恢复提交与主副本。
#[test]
pub(crate) fn abandoned_user_commit_pending_is_recovered() {
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    store.fail_times("write_commit", 2);
    assert!(ws.work_commit_user("w", "note.md", b"v1", 1, 0).is_err());
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(st.head.unwrap().id, 1);
    assert!(ws.work_has("w", "note.md"), "恢复后主副本要有文件");
}

/// 目的：提交号只增不复用；丢弃 / 恢复都不重置计数器。
#[test]
pub(crate) fn commit_ids_stay_monotonic_after_discard_and_restore() {
    let (ws, _dirs, _store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    let id1 = ws.work_commit_user("w", "a.txt", b"1", 1, 0).unwrap();
    let id2 = ws.work_commit_user("w", "b.txt", b"2", 2, 0).unwrap();
    let id3 = ws.work_commit_user("w", "c.txt", b"3", 3, 0).unwrap();
    assert_eq!((id1, id2, id3), (1, 2, 3));

    ws.work_discard_after("w", Some(id1)).unwrap();
    let id4 = ws.work_commit_user("w", "d.txt", b"4", 4, 0).unwrap();
    assert_eq!(id4, 4, "丢弃最高提交后不能复用 2/3");

    ws.work_restore_point("w", None).unwrap();
    let id5 = ws.work_commit_user("w", "e.txt", b"5", 5, 0).unwrap();
    assert_eq!(id5, 5, "清空共享区也不重置计数器");
}

/// 目的：提交收尾失败留下 pending；下一次 status 先恢复，再给出新 head。
#[test]
pub(crate) fn abandoned_commit_pending_is_recovered_by_next_status() {
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    store.write_under(&sb, "note.md", b"v2").unwrap();

    store.fail_times("write_commit", 2);
    let err = ws
        .work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "v2", 2))
        .unwrap_err();
    assert!(err.contains("恢复"), "{}", err);

    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(st.head.unwrap().id, 2, "status 要先恢复 pending 再读 head");
    assert_eq!(
        store
            .read_under(&shared_of(&ws, "w"), "note.md")
            .unwrap()
            .as_deref(),
        Some(&b"v2"[..])
    );
    assert_eq!(
        store.head(&ws.roots("w", &[]).unwrap().store).unwrap(),
        Some(2)
    );
}

/// 目的：一次失败后立即重放成功时，提交按成功返回。
#[test]
pub(crate) fn commit_pending_recovers_immediately_when_the_failure_clears() {
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    store.write_under(&sb, "note.md", b"v2").unwrap();

    store.fail_times("write_commit", 1);
    let rep = ws
        .work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "v2", 2))
        .expect("一次失败后应立即重放成功");
    assert_eq!(rep.id, 2);
    assert_eq!(
        store.head(&ws.roots("w", &[]).unwrap().store).unwrap(),
        Some(2)
    );
}

/// 目的：restore 的 pending 在崩溃后由下一次操作补完，head 与主副本一起到达目标点。
#[test]
pub(crate) fn abandoned_restore_pending_is_recovered_by_next_head_read() {
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    let id1 = ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    ws.work_commit_user("w", "note.md", b"v2", 2, 0).unwrap();

    store.fail_times("set_head", 2);
    assert!(ws.work_restore("w", id1).is_err());
    let head = ws.work_head("w").unwrap();
    assert_eq!(head, Some(id1), "恢复 pending 后 head 回到目标点");
    assert_eq!(
        store
            .read_under(&shared_of(&ws, "w"), "note.md")
            .unwrap()
            .as_deref(),
        Some(&b"v1"[..])
    );
}

/// 目的：discard 先物化再删提交；崩溃后不会留下 dangling head，下一次操作补完删除。
#[test]
pub(crate) fn abandoned_discard_pending_keeps_head_valid_and_is_recovered() {
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    let id1 = ws.work_commit_user("w", "a.txt", b"1", 1, 0).unwrap();
    let id2 = ws.work_commit_user("w", "b.txt", b"2", 2, 0).unwrap();
    let id3 = ws.work_commit_user("w", "c.txt", b"3", 3, 0).unwrap();

    store.fail_times("remove_commit", 2);
    assert!(ws.work_discard_after("w", Some(id1)).is_err());

    // 恢复前 head 已经先物化到 id1，不能指向待删的 id2/id3。
    assert_eq!(ws.work_head("w").unwrap(), Some(id1));
    let _ = ws.work_status("w", "a", &[]).unwrap();
    let root = ws.roots("w", &[]).unwrap().store;
    assert_eq!(store.list_commits(&root).unwrap(), vec![id1]);
    assert!(id2 > id1 && id3 > id2);
}

/// 目的：对象回收只删不可达对象；只被 agent 基线引用的对象、以及可达提交树的对象都要保留。
#[test]
pub(crate) fn work_gc_keeps_index_and_commit_objects_and_removes_the_rest() {
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    let root = ws.roots("w", &[]).unwrap().store;
    let id1 = ws.work_commit_user("w", "keep.txt", b"k1", 1, 0).unwrap();
    let id2 = ws.work_commit_user("w", "gone.txt", b"g1", 2, 0).unwrap();
    let id3 = ws.work_commit_user("w", "gone.txt", b"g2", 3, 0).unwrap();
    assert_eq!((id1, id2, id3), (1, 2, 3));
    // a 的基线引用 gone 的 g2；这个对象在丢弃提交后仍必须保留。
    ws.work_pull("w", "a", &["keep.txt".to_string(), "gone.txt".to_string()])
        .unwrap();
    // 一个非指纹杂项文件（模拟原子写崩溃留下的临时文件）。
    store.write_object(&root, ".tmp-stale", b"x").unwrap();
    ws.work_discard_after("w", Some(id1)).unwrap();

    let dry = ws.work_gc("w", false).unwrap();
    assert!(!dry.applied);
    assert_eq!(dry.objects, 3, "k1/g1/g2");
    assert_eq!(dry.reachable_objects, 2, "k1 + 基线引用的 g2");
    assert_eq!(dry.unreachable_objects, 1, "g1 只属于被丢弃的提交");
    assert_eq!(dry.removed_objects, 0);
    assert_eq!(dry.ignored_files, 1);
    assert!(store
        .read_object(
            &root,
            &crate::capabilities::workspace::api::content_hash(b"g1")
        )
        .is_ok());

    let applied = ws.work_gc("w", true).unwrap();
    assert!(applied.applied);
    assert_eq!(applied.removed_objects, 1);
    assert_eq!(applied.removed_ignored, 1);
    assert!(
        store
            .read_object(
                &root,
                &crate::capabilities::workspace::api::content_hash(b"g1")
            )
            .is_err(),
        "g1 应被回收"
    );
    assert!(
        store
            .read_object(
                &root,
                &crate::capabilities::workspace::api::content_hash(b"g2")
            )
            .is_ok(),
        "基线引用的 g2 必须保留"
    );
    // 基线仍能看三方；保留提交仍能物化。
    let v = ws.work_versions("w", "a", "gone.txt").unwrap();
    assert!(
        v.base.is_some(),
        "index-only 对象被删会让 work_versions 失效"
    );
    ws.work_restore("w", id1).unwrap();
}

// ---------- 主副本对 agent 只读：工具层与围栏层两处收口 ----------

/// 只读共享区：读得到、写不进；写自己的沙箱照旧；围栏也不含主副本。
/// 这是"agent 绕不过 pull/commit 直接写共享区"的工具层证据，围栏层的证据由三平台探针给。
#[test]
pub(crate) fn read_only_shared_refuses_builtin_writes_and_leaves_the_fence() {
    use crate::kernel::api::FenceSpec;
    let sb = test_sandbox_readonly("a", &[]);
    let io = InMemorySysIo::new();
    io.seed(&["demo", "work", "note.txt"], "内容");
    // 进 JSON / 补丁的路径一律用**书写形式**（/ 分隔；Windows 反斜杠在 JSON 里非法）。
    let work = s(&["demo", "work", "note.txt"]);

    // 读仍然可以（默认只读 = 可读不可写）。
    let rd = run_builtin(&sb, &io, "read", &format!("{{\"path\":\"{}\"}}", work));
    assert!(rd.ok, "只读不等于读不到：{}", rd.output);

    // 写 / 改 / 补丁一律被工具层拒绝。
    let wr = run_builtin(
        &sb,
        &io,
        "write",
        &format!("{{\"path\":\"{}\",\"content\":\"x\"}}", work),
    );
    assert!(!wr.ok && wr.output.contains("只读"), "{}", wr.output);
    let ed = run_builtin(
        &sb,
        &io,
        "edit",
        &format!(
            "{{\"path\":\"{}\",\"old_string\":\"内容\",\"new_string\":\"新\"}}",
            work
        ),
    );
    assert!(!ed.ok && ed.output.contains("只读"), "{}", ed.output);
    let patch = format!(
        "*** Update File: {}\n*** SEARCH\n内容\n*** REPLACE\n新\n*** End File\n",
        work
    );
    let pa = run_builtin(&sb, &io, "patch", &patch);
    assert!(!pa.ok && pa.output.contains("只读"), "{}", pa.output);

    // 自己的沙箱照旧可写。
    let own = s(&["demo", "a", "out.txt"]);
    let ok = run_builtin(
        &sb,
        &io,
        "write",
        &format!("{{\"path\":\"{}\",\"content\":\"x\"}}", own),
    );
    assert!(ok.ok, "{}", ok.output);

    // 围栏：主副本永远不进 rw（写共享只能走 work_commit）；私有沙箱进 rw。
    let spec = FenceSpec::for_session(&sb, &[], false);
    assert!(!spec.rw.contains(&sb.shared), "主副本不该进 rw");
    assert!(spec.rw.contains(&sb.private));
    // 会话租约从沙箱带出：同名 agent 的多个并发会话靠它区分围栏归属。
    assert_eq!(spec.lease, sb.session, "围栏租约 = 沙箱的会话 id");
    let writable = test_sandbox("a", &[]);
    let spec2 = FenceSpec::for_session(&writable, &[], false);
    assert!(!spec2.rw.contains(&writable.shared), "共享写不直接进 rw");
}

/// env.rs 装出来的 **agent 会话沙箱**默认就是只读共享区（生产接线，不是测试助手自己设的）。
#[test]
pub(crate) fn agent_sandboxes_from_a_work_are_read_only_for_the_shared_area() {
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec!["[]".into()]);
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let meta = core.history_open(&sid).unwrap().0;
    let roster = core.scan();
    let sandboxes = core.sandboxes(&meta, &roster).unwrap();
    let sb = sandboxes.list.first().expect("至少一个 agent 沙箱");
    let (place, path) = sb.resolve(&p(&["w", "work", "x.txt"])).unwrap();
    assert!(
        !sb.can_write(&place, &path),
        "生产里 agent 沙箱不能直接写共享区"
    );
}

/// 目的：版本库写闸按**工作**分：不同工作互不阻塞，同一个工作严格串行。
#[test]
pub(crate) fn version_gate_is_per_work() {
    let ws = crate::capabilities::workspace::service::WorkspaceService::new(
        Arc::new(InMemoryWorkspace::new()),
        Arc::new(InMemoryWorkStore::new()),
    );
    let a = ws.gate("a");
    let b = ws.gate("b");
    let _hold_a = a.lock().unwrap_or_else(|e| e.into_inner());
    assert!(b.try_lock().is_ok(), "不同工作互不阻塞");
    let a2 = ws.gate("a");
    assert!(a2.try_lock().is_err(), "同一个工作要串行");
}

/// 目的：work_status 也要过 per-work 写闸，不能与提交并发读到撕裂快照。
#[test]
pub(crate) fn work_status_waits_for_the_per_work_gate() {
    use crate::capabilities::workspace::api::Workspace;
    use std::sync::{mpsc, Arc};
    use std::time::Duration;
    let ws = Arc::new(
        crate::capabilities::workspace::service::WorkspaceService::new(
            Arc::new(InMemoryWorkspace::new()),
            Arc::new(InMemoryWorkStore::new()),
        ),
    );
    let gate = ws.gate("w");
    let guard = gate.lock().unwrap_or_else(|e| e.into_inner());
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = {
        let ws = Arc::clone(&ws);
        std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let ok = ws.work_status("w", "a", &[]).is_ok();
            done_tx.send(ok).unwrap();
        })
    };
    started_rx.recv().unwrap();
    assert!(
        done_rx.recv_timeout(Duration::from_millis(500)).is_err(),
        "status 不能绕过写闸"
    );
    drop(guard);
    assert!(
        done_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        "释放后 status 要完成"
    );
    worker.join().unwrap();
}

/// 目的：虚拟机档下模型给的 guest 写法要能寻址——译成宿主路径**再过同一道根校验**；越界仍拒。
#[test]
pub(crate) fn sandbox_resolves_guest_written_paths() {
    use crate::capabilities::vm::api::PathBridge;
    use crate::capabilities::vm::domain::paths::PathDialect;
    use crate::capabilities::workspace::domain::workspace::Place;
    // 跨方言才有翻译（同方言时桥是恒等，走的是宿主写法那条路）。
    let guest = if cfg!(windows) {
        PathDialect::Posix
    } else {
        PathDialect::Windows
    };
    let bridge = PathBridge::new(std::env::consts::OS, guest);
    let mut sb = test_sandbox("a1", &["data"]);
    sb.path_codec = Some(Arc::new(bridge));
    // 私有沙箱在 guest 里的写法要认，并落回宿主私有沙箱。
    let guest_note = format!("{}/note.txt", bridge.to_guest(&sb.private));
    let (place, path) = sb.resolve(&guest_note).expect("guest 写法该认");
    assert_eq!(place, Place::Private);
    assert_eq!(path, sb.private.join("note.txt"));
    // 虚拟机档只认 guest 写法：宿主写法要明确拒绝，并说清是写法不对。
    let host_note = sb.private.join("host.txt").to_string_lossy().into_owned();
    let why = sb.resolve(&host_note).expect_err("宿主写法不再直通");
    assert!(why.contains("不是本环境的写法"), "{}", why);
    // 镜像根下、但不在任何允许根里的路径：翻译得出来也要被根校验拒。
    let outside = bridge.to_guest(&abs(&["elsewhere", "x.txt"]));
    let why = sb.resolve(&outside).expect_err("越界要拒");
    assert!(why.contains("系统工具允许的根"), "{}", why);
}

/// 目的：提交按"根 + 增量"落盘，读树仍给出完整结果；work_status 的树由回溯重建。
/// 约束：只有根提交带整棵树，增量只记 changes。
#[test]
pub(crate) fn work_commits_store_roots_and_deltas_but_read_full_tree() {
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
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

/// 目的：GC 只从引用出发算活集合：有 live 钉子时旧提交保住，引用全删才真正回收。
/// 约束：留档/回档不删提交，删引用才省空间。
#[test]
pub(crate) fn work_gc_reclaims_commits_only_after_refs_drop() {
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "a.md", b"1", 1, 0).unwrap();
    ws.work_commit_user("w", "b.md", b"2", 2, 0).unwrap();
    let sd = ws.roots("w", &[]).unwrap().store;

    // 回档前把当前线钉成 live/2：旧提交仍可达。
    ws.work_restore("w", 1).unwrap();
    assert!(store.read_ref(&sd, "live/2").unwrap().is_some());
    assert_eq!(ws.work_gc("w", true).unwrap().removed_commits, 0);
    assert_eq!(store.list_commits(&sd).unwrap(), vec![1, 2]);

    // 删掉 live 与 main 两条引用后，两条提交都不可达，GC 才回收。
    store.remove_ref(&sd, "live/2").unwrap();
    store.remove_ref(&sd, "main").unwrap();
    let rep = ws.work_gc("w", true).unwrap();
    assert_eq!(rep.removed_commits, 2);
    assert!(store.list_commits(&sd).unwrap().is_empty());
}

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
