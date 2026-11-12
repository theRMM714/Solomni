//! 目的：文件类适配器在真实磁盘上的边界测试。
//! 管：`FsWorkspace` 的布局与用量、`FsWorkStore` 的哈希与逃逸、`FsSysIo` 的符号链接与编码、`FsHistory` 的嵌套与删除。
//! 不管：内存替身与端口契约（见 docs/testing/doubles.md）。
//! 联动：隔离根 `tests::scratch`；会话元数据夹具 `meta` 在文件内。

use super::*;

fn meta(name: &str) -> SessionMeta {
    SessionMeta {
        context_freshness: false,
        name: name.to_string(),
        mode: "direct".to_string(),
        delegate: false,
        modules: vec!["a".to_string()],
        task: None,
        ts: 7,
        agents: vec![AgentMeta {
            name: "a".to_string(),
            transient: false,
            modules: vec!["a".to_string()],
            model: None,
            chara: String::new(),
        }],
        exec: ExecSpec::default(),
        sandbox: Default::default(),
        parent: None,
        node: None,
        delegation: None,
        thinking: None,
        run: crate::capabilities::session::api::RunState::Active,
        workspace: None,
    }
}

#[test]
fn fs_workspace_builds_the_real_layout_and_lists_only_files() {
    let root = scratch("fs-workspace");
    let sessions = root.join("session");
    let ws = FsWorkspace::new(sessions.clone());
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .expect("建目录成功");
    assert!(
        sessions.join("w").join("work").is_dir(),
        "共享区要真的建出来"
    );
    assert!(
        sessions.join("w").join("a").is_dir() && sessions.join("w").join("b").is_dir(),
        "每个 agent 一个私有沙箱"
    );

    assert!(!ws.work_has("w", "note.txt"));
    ws.write_work("w", "note.txt", "你好".as_bytes())
        .expect("写入成功");
    assert!(ws.work_has("w", "note.txt"));
    assert_eq!(
        std::fs::read_to_string(sessions.join("w").join("work").join("note.txt")).expect("读回"),
        "你好"
    );

    std::fs::create_dir_all(sessions.join("w").join("a").join("sub")).expect("造子目录");
    std::fs::write(
        sessions.join("w").join("a").join("sub").join("deep.txt"),
        "x",
    )
    .expect("造私有文件");
    let files = ws.list("w", &["a".to_string()]).expect("列文件");
    assert_eq!(
        files.work,
        vec!["note.txt".to_string()],
        "相对路径、/ 分隔、排序稳定"
    );
    assert_eq!(
        files.agents.get("a").map(|v| v.as_slice()),
        Some(&["sub/deep.txt".to_string()][..]),
        "递归列出，只列文件"
    );

    let roots = ws.roots("w", &["a".to_string()]).expect("取根");
    assert!(
        roots.shared.is_absolute() && roots.agents.contains_key("a"),
        "根一律绝对路径"
    );
    // 列清单是查询不是创建：会话不存在 = 空清单，不报错也不造目录。
    let none = ws.list("没这个会话", &[]).expect("列不存在的会话不报错");
    assert!(
        none.work.is_empty() && !sessions.join("没这个会话").exists(),
        "查询不该有副作用"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 用量按区统计：文件数与**真实字节**（删除前如实交代用），空工作区不是错误。
#[test]
fn fs_workspace_reports_usage_by_area() {
    let root = scratch("fs-workspace-usage");
    let sessions = root.join("session");
    let ws = FsWorkspace::new(sessions.clone());
    ws.prepare("w", &["a".to_string()]).expect("建目录");
    ws.write_work("w", "note.txt", "你好".as_bytes())
        .expect("投喂");
    std::fs::write(sessions.join("w").join("a").join("out.bin"), [0u8; 5]).expect("写产物");
    let u = ws.usage("w", &["a".to_string()]).expect("统计用量");
    assert_eq!(u.files, 2);
    assert_eq!(u.bytes, "你好".len() as u64 + 5);
    assert_eq!(u.work.files, 1);
    assert_eq!(u.work.bytes, "你好".len() as u64);
    assert_eq!(u.agents["a"].files, 1);
    assert_eq!(u.agents["a"].bytes, 5);
    let empty = ws.usage("没这个会话", &[]).expect("统计不存在的会话不报错");
    assert_eq!(empty.files, 0, "查询不该有副作用");
    let _ = std::fs::remove_dir_all(&root);
}

/// 真实版本库：文件原语 + 内容寻址对象 / 提交 / head / 基线往返；**符号链接逃逸必须被拒**。
#[test]
fn fs_work_store_round_trips_and_refuses_symlink_escapes() {
    use crate::capabilities::workspace::detail::fs_workstore::FsWorkStore;
    use crate::capabilities::workspace::domain::workstore::{
        Change, ChangeKind, Commit, Index, Ref, RefKind,
    };
    use crate::capabilities::workspace::ports::WorkStore;

    let root = scratch("fs-workstore");
    let shared = root.join("work");
    let sandbox = root.join("a");
    let store = root.join(".work");
    std::fs::create_dir_all(&shared).expect("建共享区");
    std::fs::create_dir_all(&sandbox).expect("建沙箱");
    let s = FsWorkStore::new();

    // 空仓库不是错误。
    assert_eq!(s.head(&store).unwrap(), None);
    // 旧布局：只有 head 文件时读到一次就升级成 refs/main，旧文件随之删除。
    std::fs::create_dir_all(&store).expect("建版本库");
    std::fs::write(store.join("head"), b"7").expect("写旧 head");
    assert_eq!(s.head(&store).unwrap(), Some(7));
    assert!(!store.join("head").exists(), "升级后旧 head 文件要删掉");
    assert_eq!(s.read_ref(&store, "main").unwrap().unwrap().target, Some(7));
    s.clear_head(&store).unwrap();
    assert!(s.read_ref(&store, "main").unwrap().is_none());
    // 命名引用：写 / 列 / 删都能跨调用往返。
    s.write_ref(
        &store,
        "archive/3",
        &Ref {
            kind: RefKind::Archive,
            target: Some(7),
            heads: Default::default(),
        },
    )
    .unwrap();
    assert!(s.list_refs(&store).unwrap().contains_key("archive/3"));
    s.remove_ref(&store, "archive/3").unwrap();
    assert!(s.read_ref(&store, "archive/3").unwrap().is_none());
    assert!(s.list(&shared).unwrap().is_empty());
    assert!(s.read_index(&store, "a").unwrap().is_empty());
    assert!(s.read_commit(&store, 1).unwrap().is_none());
    assert!(s.read_object(&store, "nope").is_err());

    // 文件原语：列目录相对 / 分隔，删除幂等。
    std::fs::create_dir_all(sandbox.join("sub")).expect("建子目录");
    std::fs::write(sandbox.join("sub").join("out.txt"), b"hi").expect("写文件");
    assert_eq!(s.list(&sandbox).unwrap(), vec!["sub/out.txt"]);
    s.remove_under(&sandbox, "sub/out.txt").unwrap();
    s.remove_under(&sandbox, "sub/out.txt").unwrap();
    assert!(!sandbox.join("sub").join("out.txt").exists());

    // 对象 / 提交 / head / 拉取基线。
    s.write_object(&store, "h1", b"obj").unwrap();
    assert_eq!(s.read_object(&store, "h1").unwrap(), b"obj");
    let mut tree = std::collections::BTreeMap::new();
    tree.insert("a/b.txt".to_string(), "h1".to_string());
    let c = Commit {
        id: 1,
        parent: None,
        author: "a".to_string(),
        session: "w--a".to_string(),
        time: 1,
        message: "首提交".to_string(),
        changes: vec![Change {
            path: "a/b.txt".to_string(),
            kind: ChangeKind::Add,
            hash: "h1".to_string(),
        }],
        tree: Some(tree),
        base: 0,
        depth: 0,
        source: Default::default(),
        anchor: None,
    };
    s.write_commit(&store, &c).unwrap();
    s.set_head(&store, 1).unwrap();
    assert_eq!(s.head(&store).unwrap(), Some(1));
    assert_eq!(s.list_commits(&store).unwrap(), vec![1]);
    assert_eq!(s.read_commit(&store, 1).unwrap().unwrap(), c);
    let mut idx = Index::new();
    idx.insert("a/b.txt".to_string(), "h1".to_string());
    s.write_index(&store, "a", &idx).unwrap();
    assert_eq!(s.read_index(&store, "a").unwrap(), idx);

    // 事务日志 / 提交号 / 文件锁：真实适配器都要能往返。
    use crate::capabilities::workspace::domain::workstore::Pending;
    let lock = s.lock(&store).expect("拿锁");
    assert!(lock.is_some(), "真实适配器有跨进程锁文件");
    let pending = Pending::Restore { target: Some(1) };
    s.write_pending(&store, &pending).unwrap();
    assert_eq!(s.read_pending(&store).unwrap(), Some(pending));
    s.clear_pending(&store).unwrap();
    assert!(s.read_pending(&store).unwrap().is_none());
    assert_eq!(s.read_next_id(&store).unwrap(), None);
    s.write_next_id(&store, 9).unwrap();
    assert_eq!(s.read_next_id(&store).unwrap(), Some(9));
    // 原子写不能留下临时文件。
    let leftovers: Vec<String> = std::fs::read_dir(&store)
        .unwrap()
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| n.starts_with(".tmp-"))
        .collect();
    assert!(leftovers.is_empty(), "临时文件要清掉：{:?}", leftovers);
    drop(lock);

    // 符号链接逃逸：根外的目录被链接进沙箱，写它必须被拒（绝不写进根外）。
    #[cfg(unix)]
    {
        let outside = root.join("outside");
        std::fs::create_dir_all(&outside).expect("建根外目录");
        std::os::unix::fs::symlink(&outside, sandbox.join("link")).expect("造链接");
        let err = s
            .copy_object_to_under(&store, "h1", &sandbox, "link/pwn.txt")
            .unwrap_err();
        assert!(err.contains("符号链接") || err.contains("越过"), "{}", err);
        assert!(!outside.join("pwn.txt").exists(), "绝不能写进根外");
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：真实适配器的资产流式原语：扫描不整份返回、对象写入校验指纹与上限、复制清临时文件。
#[test]
fn fs_work_store_streams_assets_and_rejects_oversize_or_mismatch() {
    use crate::capabilities::workspace::api::content_hash;
    use crate::capabilities::workspace::detail::fs_workstore::FsWorkStore;
    use crate::capabilities::workspace::ports::WorkStore;

    let root = scratch("fs-workstore-assets");
    let shared = root.join("work");
    let sandbox = root.join("a");
    let store = root.join(".work");
    std::fs::create_dir_all(&shared).expect("建共享区");
    std::fs::create_dir_all(&sandbox).expect("建沙箱");
    let s = FsWorkStore::new();

    let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(sandbox.join("clip.bin"), &data).expect("写大文件");
    assert_eq!(
        s.stat_under(&sandbox, "clip.bin").unwrap(),
        Some(data.len() as u64)
    );
    assert_eq!(s.scan_under(&sandbox, "nope.bin", None).unwrap(), None);

    let scanned = s.scan_under(&sandbox, "clip.bin", None).unwrap().unwrap();
    assert_eq!(scanned.hash, content_hash(&data));
    assert_eq!(scanned.size, data.len() as u64);
    assert!(scanned.text.is_none(), "含 NUL 的字节流不给文本");
    let hash = scanned.hash;

    let size = s
        .ingest_under(&store, &sandbox, "clip.bin", &hash, 1_000_000)
        .unwrap();
    assert_eq!(size, data.len() as u64);
    assert_eq!(s.stat_object(&store, &hash).unwrap(), data.len() as u64);
    let object = s.scan_object(&store, &hash, None).unwrap();
    assert_eq!(object.hash, hash);
    assert_eq!(object.size, data.len() as u64);

    s.copy_object_to_under(&store, &hash, &shared, "copy/clip.bin")
        .unwrap();
    assert_eq!(
        std::fs::read(shared.join("copy").join("clip.bin")).unwrap(),
        data
    );

    // 指纹不符：不落对象，临时文件清掉。
    let bad = content_hash(b"different");
    let err = s
        .ingest_under(&store, &sandbox, "clip.bin", &bad, 1_000_000)
        .unwrap_err();
    assert!(err.contains("改动"), "{}", err);
    assert!(s.stat_object(&store, &bad).is_err());

    // 上限：换一个新文件，确保不是“对象已存在”的短路。
    std::fs::write(sandbox.join("big.bin"), [0u8; 32]).expect("写超限文件");
    let big = content_hash(&[0u8; 32]);
    let err = s
        .ingest_under(&store, &sandbox, "big.bin", &big, 16)
        .unwrap_err();
    assert!(err.contains("上限"), "{}", err);
    assert!(s.stat_object(&store, &big).is_err());

    assert!(s.scan_object(&store, "nope", None).is_err());
    assert!(
        s.copy_object_to_under(&store, "nope", &shared, "x")
            .is_err()
    );

    // 临时文件不留在对象目录或目标目录。
    let leftovers = |dir: &std::path::Path| -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .filter(|n| n.starts_with(".tmp-"))
            .collect()
    };
    assert!(leftovers(&store.join("objects")).is_empty());
    assert!(leftovers(&shared.join("copy")).is_empty());

    #[cfg(unix)]
    {
        let outside = root.join("outside");
        std::fs::create_dir_all(&outside).expect("建根外目录");
        std::fs::write(outside.join("pwn.bin"), b"x").expect("写根外文件");
        std::os::unix::fs::symlink(&outside, sandbox.join("link")).expect("造链接");
        assert!(
            s.scan_under(&sandbox, "link/pwn.bin", None).is_err(),
            "扫描也不能越过符号链接"
        );
        assert!(
            s.ingest_under(&store, &sandbox, "link/pwn.bin", &content_hash(b"x"), 999)
                .is_err(),
            "写对象也不能越过符号链接"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：跨根复制保持流式与指纹校验；目标根可新建，失败不留临时文件，符号链接逃逸被拒。
#[test]
fn fs_work_store_copies_between_roots_with_hash_check() {
    use crate::capabilities::workspace::api::content_hash;
    use crate::capabilities::workspace::detail::fs_workstore::FsWorkStore;
    use crate::capabilities::workspace::ports::WorkStore;

    let root = scratch("fs-workstore-cross-copy");
    let sandbox = root.join("a");
    let lost = root.join(".work").join("lost").join("a");
    std::fs::create_dir_all(&sandbox).expect("建沙箱");
    let s = FsWorkStore::new();
    let data = b"lost bytes".to_vec();
    std::fs::write(sandbox.join("note.bin"), &data).expect("写本地版本");
    let hash = content_hash(&data);

    // 目标根不存在：复制要建出来。
    let size = s
        .copy_under_to_under(&sandbox, "note.bin", &lost, "note.bin", &hash)
        .unwrap();
    assert_eq!(size, data.len() as u64);
    assert_eq!(std::fs::read(lost.join("note.bin")).unwrap(), data);

    // 指纹不符：拒绝，目标不出现，临时文件清掉。
    let bad = content_hash(b"different");
    let err = s
        .copy_under_to_under(&sandbox, "note.bin", &lost, "other.bin", &bad)
        .unwrap_err();
    assert!(err.contains("改动"), "{}", err);
    assert!(!lost.join("other.bin").exists());
    let leftovers: Vec<String> = std::fs::read_dir(&lost)
        .unwrap()
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| n.starts_with(".tmp-"))
        .collect();
    assert!(leftovers.is_empty(), "临时文件要清掉：{:?}", leftovers);

    // 源缺失。
    assert!(
        s.copy_under_to_under(&sandbox, "nope", &lost, "x", &hash)
            .is_err()
    );

    #[cfg(unix)]
    {
        let outside = root.join("outside");
        std::fs::create_dir_all(&outside).expect("建根外目录");
        std::fs::write(outside.join("pwn.bin"), b"x").expect("写根外文件");
        std::os::unix::fs::symlink(&outside, sandbox.join("link")).expect("造链接");
        assert!(
            s.copy_under_to_under(
                &sandbox,
                "link/pwn.bin",
                &lost,
                "pwn.bin",
                &content_hash(b"x")
            )
            .is_err(),
            "源路径不能越过符号链接"
        );
        std::os::unix::fs::symlink(&outside, lost.join("out")).expect("造目标链接");
        assert!(
            s.copy_under_to_under(&sandbox, "note.bin", &lost, "out/pwn.bin", &hash)
                .is_err(),
            "目标路径不能越过符号链接"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：词法上在根内的路径，真实落点可能是符号链接 / junction 指的根外——必须拒绝。
#[test]
fn fs_sys_io_rejects_symlink_or_junction_escape() {
    let base = scratch("fs-sysio-link");
    let root = base.join("root");
    let outside = base.join("outside");
    std::fs::create_dir_all(&root).expect("建根");
    std::fs::create_dir_all(&outside).expect("建外部");
    std::fs::write(outside.join("secret.txt"), "SECRET").expect("造外部文件");

    if !make_dir_link(&outside, &root.join("out")) {
        return; // 平台不允许造链接：跳过（Windows 的 junction 探针在真机上另验）
    }
    let io = FsSysIo::default();
    assert!(
        io.within(&root, &root.join("out").join("secret.txt"))
            .is_err(),
        "junction 指向根外必须拒绝"
    );
    assert!(
        io.within(&root, &root.join("ok.txt")).is_ok(),
        "不存在的普通文件按最近的已存在祖先判，仍在根内"
    );
    if make_file_link(&outside.join("missing.txt"), &root.join("dangling")) {
        assert!(
            io.within(&root, &root.join("dangling")).is_err(),
            "悬空符号链接不能被“看父目录”绕过"
        );
    }
}

#[cfg(unix)]
fn make_dir_link(target: &std::path::Path, link: &std::path::Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn make_dir_link(target: &std::path::Path, link: &std::path::Path) -> bool {
    std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(unix)]
fn make_file_link(target: &std::path::Path, link: &std::path::Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn make_file_link(target: &std::path::Path, link: &std::path::Path) -> bool {
    std::os::windows::fs::symlink_file(target, link).is_ok()
}

#[test]
fn fs_sys_io_reads_utf8_flags_lossy_and_truncation_and_creates_parents() {
    let root = scratch("fs-sysio");
    let f = root.join("deep").join("note.txt");
    let io = FsSysIo::default();
    io.write(&f, "你好，世界").expect("写要自动建父目录");
    let r = io.read(&f).expect("读回");
    assert_eq!(r.text, "你好，世界");
    assert_eq!(r.bytes, "你好，世界".len(), "bytes 是文件原始字节数");
    assert!(!r.lossy && !r.cut);

    let bad = root.join("bad.bin");
    std::fs::write(&bad, [0x41, 0xff, 0xfe, 0x42]).expect("造非法 UTF-8");
    let r = io.read(&bad).expect("读回");
    assert!(r.lossy, "含非法字节必须如实标注 lossy（本程序不猜编码）");
    assert!(r.text.starts_with('A'), "{}", r.text);

    let small = FsSysIo { max_read_bytes: 3 };
    let cut = small.read(&f).expect("截断读取成功");
    assert!(cut.cut, "超出上限要如实标注 cut");
    assert_eq!(cut.bytes, "你好，世界".len(), "bytes 仍是整个文件的字节数");
    assert!(cut.text.chars().count() < 6, "只读了开头：{}", cut.text);

    // 大文件也只读开头：
    let big = root.join("big.bin");
    std::fs::write(&big, vec![b'a'; 100_000]).expect("造大文件");
    let r = small.read(&big).expect("大文件截断读取成功");
    assert!(
        r.cut && r.bytes == 100_000 && r.text.len() == 3,
        "{:?}",
        r.text
    );

    assert!(
        io.read(&root.join("nope.txt"))
            .err()
            .expect("应当失败")
            .contains("读取失败"),
        "缺文件要如实报错"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fs_history_roundtrips_lists_deletes_and_rejects_broken_meta() {
    let root = scratch("fs-history");
    let h = FsHistory::new(root.clone());
    // 目录还不存在：列会话 = 空清单，不是错误。
    assert!(h.list().expect("列会话").is_empty());

    h.create(&meta("w1")).expect("创建成功");
    assert!(
        root.join("w1").join("meta.yaml").is_file(),
        "meta.yaml 落盘"
    );
    h.append("w1", &[serde_json::json!({"type": "say", "text": "hi"})])
        .expect("追加成功");
    h.append("w1", &[serde_json::json!({"type": "ended"})])
        .expect("追加成功");
    let (m, events) = h.load("w1").expect("读回");
    assert_eq!(m.name, "w1");
    assert_eq!(events.len(), 2, "jsonl 一行一条，只追加不丢");
    let listed = h.list().expect("列会话");
    assert_eq!(listed.len(), 1);
    assert!(listed[0].done, "ended 之后 done = true");

    let mut renamed = meta("w1");
    renamed.mode = "collab".to_string();
    h.save_meta(&renamed).expect("写回元信息");
    assert_eq!(
        h.load("w1").expect("读回").0.mode,
        "collab",
        "meta.yaml 是会话身份的唯一真相"
    );

    std::fs::create_dir_all(root.join("w2")).expect("造一个坏会话");
    std::fs::write(root.join("w2").join("meta.yaml"), "mode: [不是字符串").expect("造坏 meta");
    assert!(
        h.load("w2").unwrap_err().contains("非法"),
        "坏 meta 要如实报错，不静默跳过"
    );
    assert_eq!(h.list().expect("列会话").len(), 1, "坏会话不进清单");

    assert!(h.delete("w1").expect("删除成功"), "删除已存在的会话 = true");
    assert!(!h.delete("w1").expect("再删"), "重复删除 = false，不是错误");
    assert!(h.load("w1").unwrap_err().contains("无此会话"));
    let _ = std::fs::remove_dir_all(&root);
}

/// 子会话落在**父会话目录内部**（`session/<父>/children/<子>`）：位置与 meta.parent 同源。
/// 嵌套可以任意深（代理的子会话里还能有它自己的子会话）：整棵 children/ 树都要能找到、列出、删净。
#[test]
fn fs_history_nests_children_inside_their_parent_dir() {
    let root = scratch("fs-history-children");
    let h = FsHistory::new(root.clone());
    let mut parent = meta("p");
    parent.mode = "collab".to_string();
    h.create(&parent).expect("建父会话");
    let mut child = meta("p--甲");
    child.parent = Some("p".to_string());
    h.create(&child).expect("建子会话");
    // 第二层：子会话自己的子会话（代理建的协作子工作会派生节点子会话）。
    let mut grand = meta("p--甲--乙");
    grand.parent = Some("p--甲".to_string());
    h.create(&grand).expect("建孙会话");
    h.append("p--甲", &[serde_json::json!({"type": "say", "text": "hi"})])
        .expect("子会话流水");
    h.append(
        "p--甲--乙",
        &[serde_json::json!({"type": "say", "text": "yo"})],
    )
    .expect("孙会话流水（按名字在任意深度定位）");
    assert!(
        root.join("p")
            .join("children")
            .join("p--甲")
            .join("meta.yaml")
            .is_file(),
        "子会话落在父会话目录内部"
    );
    assert!(
        root.join("p")
            .join("children")
            .join("p--甲")
            .join("children")
            .join("p--甲--乙")
            .join("meta.yaml")
            .is_file(),
        "子会话可以再嵌套：children/ 树任意深"
    );
    assert!(!root.join("p--甲").exists(), "不再与父会话并列");
    let listed = h.list().expect("列会话");
    assert_eq!(listed.len(), 3, "祖父子都在清单里（深层也要收进来）");
    assert_eq!(
        h.load("p--甲--乙")
            .expect("按名字读孙会话")
            .0
            .parent
            .as_deref(),
        Some("p--甲")
    );
    assert!(h.delete("p").expect("删父会话"), "删父 = 删一个目录");
    assert!(!root.join("p").exists());
    assert!(h.load("p--甲--乙").is_err(), "整棵子树随父一起消失");
    let _ = std::fs::remove_dir_all(&root);
}
