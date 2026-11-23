//! 目的：文件类适配器在真实磁盘上的边界测试。
//! 管：`FsWorkspace` 的布局与用量、`FsWorkStore` 的哈希与逃逸、`FsSysIo` 的符号链接与编码、`FsHistory` 的嵌套与删除。
//! 不管：内存替身与端口契约（见 docs/testing/doubles.md）。
//! 联动：隔离根 `tests::scratch`；会话元数据夹具 `meta` 在文件内。

use super::*;
use crate::capabilities::conversation::session::api::{MEETING, SINGLE, SessionSpec};
use crate::capabilities::workspace::api::{ChangeKind, CommitAnchor, WORKIGNORE, WORKIGNORE_SEED};
use crate::tests::doubles::test_workspace_store;

fn meta(name: &str) -> SessionMeta {
    SessionMeta {
        usage_totals: Default::default(),
        context_freshness: false,
        freshness_keep_percent: 80,
        name: name.to_string(),
        spec: SessionSpec::plain(SINGLE, "单机", "solo"),
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
        shared: Default::default(),
        parent: None,
        node: None,
        delegation: None,
        thinking: None,
        run: crate::capabilities::conversation::session::api::RunState::Active,
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
fn fs_meta_primitives_follow_platform_file_mode_rules() {
    let root = scratch("fs-meta");
    let file = root.join("tool");
    std::fs::write(&file, b"#!/bin/sh\n").expect("写文件");

    crate::kernel::api::fs_meta::make_executable_checked(&file).expect("加执行位");
    assert!(crate::kernel::detail::fs_meta::is_executable(&file));
    crate::kernel::api::fs_meta::ensure_executable(&file);
    crate::kernel::api::fs_meta::ensure_executable(&root.join("missing"));
    let dir = root.join("dir");
    std::fs::create_dir_all(&dir).expect("建目录");
    crate::kernel::api::fs_meta::ensure_executable(&dir);
    crate::kernel::api::fs_meta::make_executable(&file);
    crate::kernel::api::fs_meta::restrict_private_file(&file).expect("收紧私密权限");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&file)
                .expect("元数据")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let meta = std::fs::metadata(&file).expect("元数据");
    assert!(crate::kernel::api::fs_meta::file_footprint(&meta) > 0);
    crate::kernel::api::fs_meta::sync_dir(&root);

    let script = root.join("script");
    crate::kernel::api::fs_meta::write_executable(&script, "#!/bin/sh\n").expect("写可执行脚本");
    assert!(script.is_file());
    #[cfg(unix)]
    assert!(crate::kernel::detail::fs_meta::is_executable(&script));

    let link = root.join("link");
    let linked = crate::kernel::api::fs_meta::symlink("tool", &link);
    #[cfg(unix)]
    {
        linked.expect("建软链");
        assert!(crate::kernel::api::fs_meta::is_link(&link));
    }
    #[cfg(not(unix))]
    assert!(linked.is_err());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fs_sys_io_reads_utf8_flags_lossy_and_truncation_and_creates_parents() {
    let root = scratch("fs-sysio");
    let f = root.join("deep").join("note.txt");
    let io = FsSysIo::default();
    io.mutate(
        &f,
        &crate::capabilities::tools::ports::Expect::Absent,
        crate::capabilities::tools::ports::Mutation::Replace("你好，世界".to_string()),
    )
    .expect("写要自动建父目录");
    let r = io.read(&f).expect("读回");
    assert_eq!(r.text, "你好，世界");
    assert_eq!(r.bytes, "你好，世界".len(), "bytes 是文件原始字节数");
    assert!(!r.lossy && !r.cut);

    let bad = root.join("bad.bin");
    std::fs::write(&bad, [0x41, 0xff, 0xfe, 0x42]).expect("造非法 UTF-8");
    let r = io.read(&bad).expect("读回");
    assert!(r.lossy, "含非法字节必须如实标注 lossy（本程序不猜编码）");
    assert!(r.text.starts_with('A'), "{}", r.text);

    let small = FsSysIo {
        max_read_bytes: 3,
        ..FsSysIo::default()
    };
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

/// 目的：目录遍历按 .gitignore / .workignore 与隐藏、二进制剪枝，流式按行扫描从磁盘读，前 8 KiB 含 NUL 判二进制。
#[test]
fn fs_sys_io_walks_a_tree_and_scans_lines_from_disk() {
    use crate::capabilities::tools::ports::WalkOpts;
    let root = scratch("fs-sysio-walk");
    std::fs::create_dir_all(root.join("src")).expect("建目录");
    std::fs::create_dir_all(root.join("node_modules").join("pkg")).expect("建目录");
    std::fs::create_dir_all(root.join(".hidden")).expect("建目录");
    std::fs::write(root.join("src").join("a.rs"), "let a = 1;\nlet b = 2;\n").expect("写文件");
    std::fs::write(root.join("node_modules").join("pkg").join("i.js"), "x\n").expect("写文件");
    std::fs::write(root.join(".hidden").join("h.txt"), "h\n").expect("写文件");
    std::fs::write(root.join(".gitignore"), "node_modules/\n").expect("写规则");
    std::fs::write(root.join("bin.dat"), [b'a', 0, b'b']).expect("写二进制");
    let io = FsSysIo::default();

    let walked = io
        .walk(
            &root,
            None,
            WalkOpts {
                hidden: false,
                ignore: true,
            },
        )
        .expect("遍历成功");
    let rels: Vec<&str> = walked.iter().map(|e| e.rel.as_str()).collect();
    assert_eq!(
        rels,
        vec!["bin.dat", "src/a.rs"],
        "点号开头的文件与目录一律按隐藏处理：{:?}",
        rels
    );

    let walked = io
        .walk(
            &root,
            None,
            WalkOpts {
                hidden: true,
                ignore: true,
            },
        )
        .expect("遍历成功");
    let rels: Vec<&str> = walked.iter().map(|e| e.rel.as_str()).collect();
    assert_eq!(
        rels,
        vec![".gitignore", ".hidden/h.txt", "bin.dat", "src/a.rs"],
        "放行隐藏文件、.gitignore 仍生效：{:?}",
        rels
    );

    let walked = io
        .walk(
            &root,
            None,
            WalkOpts {
                hidden: false,
                ignore: false,
            },
        )
        .expect("遍历成功");
    let rels: Vec<&str> = walked.iter().map(|e| e.rel.as_str()).collect();
    assert!(
        rels.contains(&"node_modules/pkg/i.js"),
        "关掉忽略后依赖目录要能搜到：{:?}",
        rels
    );

    let mut seen: Vec<(u32, String)> = Vec::new();
    let summary = io
        .scan_lines(&root.join("src").join("a.rs"), &mut |n, line| {
            seen.push((n, line.to_string()));
        })
        .expect("扫描成功");
    assert_eq!(summary.lines, 2, "行号从 1 数到 2");
    assert!(!summary.binary && !summary.lossy);
    assert_eq!(
        seen,
        vec![(1, "let a = 1;".to_string()), (2, "let b = 2;".to_string())]
    );

    let mut called = 0;
    let summary = io
        .scan_lines(&root.join("bin.dat"), &mut |_, _| {
            called += 1;
        })
        .expect("扫描成功");
    assert!(summary.binary, "含 NUL 判二进制");
    assert_eq!(called, 0, "二进制不回调任何行");

    assert_eq!(
        io.read_small(&root.join(".gitignore"))
            .expect("读成功")
            .as_deref(),
        Some("node_modules/\n")
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：按行窗口读时只取请求区间，但行数与版本覆盖整份文件——超过 1MB 也能直接读后半段。
#[test]
fn fs_sys_io_read_page_reads_a_late_window_and_counts_the_whole_file() {
    let root = scratch("fs-sysio-page");
    let big = root.join("big.txt");
    let line = format!("{}\n", "a".repeat(100));
    let text = line.repeat(12_000);
    assert!(text.len() > 1_000_000, "样本要超过旧的 1MB 前缀上限");
    std::fs::write(&big, &text).expect("造大文件");

    let io = FsSysIo::default();
    let page = io.read_page(&big, 11_000, 2).expect("读后半段");
    assert_eq!(page.total_lines, 12_000, "行数按整份文件统计");
    assert_eq!(
        page.text,
        format!("{}\n{}", "a".repeat(100), "a".repeat(100))
    );
    assert_eq!(page.bytes, text.len());
    assert_eq!(
        page.hash,
        crate::capabilities::workspace::api::content_hash(text.as_bytes()),
        "版本仍是整份文件"
    );
    assert!(!page.lossy && !page.line_cut && !page.window_cut);

    let tail = io.read_page(&big, 12_000, 5).expect("读末行");
    assert_eq!(tail.text, "a".repeat(100));
    assert_eq!(tail.total_lines, 12_000);

    // 选中字节到单次上限就收手：window_cut 如实标注，行数仍然数完。
    let small = FsSysIo {
        max_read_bytes: 6,
        ..FsSysIo::default()
    };
    let w = root.join("window.txt");
    std::fs::write(&w, "aaa\nbbb\nccc\n").expect("造窗口文件");
    let page = small.read_page(&w, 1, 3).expect("窗口读");
    assert_eq!(page.text, "aaa\nbbb");
    assert!(page.window_cut && !page.line_cut, "{page:?}");
    assert_eq!(page.total_lines, 3);

    // 单行超过上限：只取前缀，line_cut 如实标注。
    let narrow = FsSysIo {
        max_read_bytes: 2,
        ..FsSysIo::default()
    };
    let page = narrow.read_page(&w, 1, 1).expect("窄窗口读");
    assert_eq!(page.text, "aa");
    assert!(page.line_cut, "{page:?}");
    assert_eq!(page.total_lines, 3);

    // 开头的空行也是选中行；CRLF 的 \r 去掉，与 str::lines() 同口径。
    let blanks = root.join("blanks.txt");
    std::fs::write(&blanks, "\n\nx\n").expect("造空行文件");
    let page = io.read_page(&blanks, 1, 2).expect("读开头的空行");
    assert_eq!(page.text, "\n");
    assert_eq!(page.total_lines, 3);

    let crlf = root.join("crlf.txt");
    std::fs::write(&crlf, "a\r\nb\r\n").expect("造 CRLF 文件");
    let page = io.read_page(&crlf, 1, 2).expect("读 CRLF");
    assert_eq!(page.text, "a\nb");
    assert_eq!(page.total_lines, 2);

    let _ = std::fs::remove_dir_all(&root);
}

/// 写盘是 CAS：版本对不上就不写；新建不覆盖；编辑在锁内重新找锚点。
#[test]
fn fs_sys_io_mutate_is_compare_and_swap_and_edit_rechecks_the_anchor() {
    use crate::capabilities::tools::api::LineRange;
    use crate::capabilities::tools::ports::{Expect, Mutation};

    let root = scratch("fs-sysio-mutate");
    let io = FsSysIo::default();
    let f = root.join("note.txt");
    let first = io
        .mutate(
            &f,
            &Expect::Absent,
            Mutation::Replace("一\n二\n".to_string()),
        )
        .expect("新建成功");
    assert_eq!(first.total_lines, 2);
    assert_eq!(first.changed, vec![LineRange::new(1, 2)]);

    // 版本已变时再写：拒绝，且不覆盖对方写入的内容。
    std::fs::write(&f, "外部改过\n").expect("模拟外部改动");
    let stale = io
        .mutate(
            &f,
            &Expect::Version(first.version.clone()),
            Mutation::Replace("覆盖".to_string()),
        )
        .unwrap_err();
    assert_eq!(stale.code, "FS_STALE_VERSION");
    assert_eq!(
        std::fs::read_to_string(&f).expect("读回"),
        "外部改过\n",
        "版本不符时一个字都不能写"
    );

    let exists = io
        .mutate(&f, &Expect::Absent, Mutation::Replace("覆盖".to_string()))
        .unwrap_err();
    assert_eq!(exists.code, "FS_ALREADY_EXISTS");

    let current = io.fingerprint(&f).expect("取版本").expect("文件在");
    let edited = io
        .mutate(
            &f,
            &Expect::Version(current),
            Mutation::Edit {
                old: "外部".to_string(),
                new: "改后".to_string(),
                all: false,
            },
        )
        .expect("编辑成功");
    assert_eq!(std::fs::read_to_string(&f).expect("读回"), "改后改过\n");
    assert_eq!(edited.changed, vec![LineRange::new(1, 1)]);
    assert_eq!(edited.replaced, 1);

    let missing = io
        .mutate(
            &f,
            &Expect::Version(edited.version.clone()),
            Mutation::Edit {
                old: "不存在的锚点".to_string(),
                new: "x".to_string(),
                all: false,
            },
        )
        .unwrap_err();
    assert_eq!(missing.code, "FS_ANCHOR_NOT_FOUND");
    assert_eq!(std::fs::read_to_string(&f).expect("读回"), "改后改过\n");

    // 落盘后不留临时文件。
    let names: Vec<String> = std::fs::read_dir(&root)
        .expect("列目录")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names.iter().all(|n| !n.contains(".solomni-")),
        "临时文件必须清理：{:?}",
        names
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 事务：先全部校验/暂存，再发布；后面失败时前面的文件不能留下。
#[test]
fn fs_sys_io_transact_commits_together_or_not_at_all() {
    use crate::capabilities::tools::ports::{Expect, Mutation, WriteOp};

    let root = scratch("fs-sysio-transact");
    let io = FsSysIo::default();
    let a = root.join("a.txt");
    let b = root.join("b.txt");
    let receipts = io
        .transact(vec![
            WriteOp {
                path: a.clone(),
                expect: Expect::Absent,
                mutation: Mutation::Replace("甲".to_string()),
            },
            WriteOp {
                path: b.clone(),
                expect: Expect::Absent,
                mutation: Mutation::Replace("乙".to_string()),
            },
        ])
        .expect("事务成功");
    assert_eq!(receipts.len(), 2);
    assert_eq!(std::fs::read_to_string(&a).expect("读回"), "甲");
    assert_eq!(std::fs::read_to_string(&b).expect("读回"), "乙");

    // 同一路径出现两次：整笔拒绝，不猜哪次算数。
    let dup = io
        .transact(vec![
            WriteOp {
                path: a.clone(),
                expect: Expect::Absent,
                mutation: Mutation::Replace("一".to_string()),
            },
            WriteOp {
                path: a.clone(),
                expect: Expect::Absent,
                mutation: Mutation::Replace("二".to_string()),
            },
        ])
        .unwrap_err();
    assert_eq!(dup.code, "FS_DUPLICATE_PATH");

    // 第 2 个路径的父级是个文件：校验阶段就失败，第 1 个不能先写出去。
    let blocker = root.join("blocker");
    std::fs::write(&blocker, "占位").expect("造父级文件");
    let c = root.join("c.txt");
    let d = blocker.join("d.txt");
    let err = io
        .transact(vec![
            WriteOp {
                path: c.clone(),
                expect: Expect::Absent,
                mutation: Mutation::Replace("丙".to_string()),
            },
            WriteOp {
                path: d,
                expect: Expect::Absent,
                mutation: Mutation::Replace("丁".to_string()),
            },
        ])
        .unwrap_err();
    assert_eq!(err.code, "FS_IO");
    assert!(!c.exists(), "事务失败不能留下第 1 个文件");

    // 回滚原语：有备份就写回，没有备份就删掉。
    crate::capabilities::tools::detail::sys_io::rollback(&a, Some("备份".as_bytes()))
        .expect("写回备份内容");
    assert_eq!(std::fs::read_to_string(&a).expect("读回"), "备份");
    std::fs::write(&c, "新").expect("造新文件");
    crate::capabilities::tools::detail::sys_io::rollback(&c, None).expect("删掉新文件");
    assert!(!c.exists());

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
    renamed.spec = SessionSpec::plain(MEETING, "会议", "discussant");
    h.save_meta(&renamed).expect("写回元信息");
    assert_eq!(
        h.load("w1").expect("读回").0.spec.kind,
        MEETING,
        "meta.yaml 是会话身份的唯一真相"
    );

    std::fs::create_dir_all(root.join("w2")).expect("造一个坏会话");
    std::fs::write(root.join("w2").join("meta.yaml"), "spec: [不是字符串").expect("造坏 meta");
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
    parent.spec = SessionSpec::plain(MEETING, "会议", "discussant");
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

/// 目的：写路径的跨进程锁由锁目录承载——同路径第二个实例拿不到锁就报 FS_LOCKED，放锁后可重试。
#[test]
fn fs_sys_io_write_lock_is_cross_instance_and_releases() {
    use crate::capabilities::workspace::api::content_hash;

    let root = scratch("fs-sysio-lock");
    let path = root.join("a.txt");
    std::fs::write(&path, "a").expect("造文件");
    let io = FsSysIo::with_lock_dir(root.join("locks"));
    let held = io
        .hold_lock_for_test(&path)
        .expect("拿锁成功")
        .expect("配置了锁目录就应有锁文件");

    let mut tight = FsSysIo::with_lock_dir(root.join("locks"));
    tight.lock_wait = std::time::Duration::from_millis(50);
    let version = content_hash(b"a");
    let err = tight
        .mutate(
            &path,
            &crate::capabilities::tools::ports::Expect::Version(version.clone()),
            crate::capabilities::tools::ports::Mutation::Replace("b".to_string()),
        )
        .expect_err("另一个实例还持锁时必须失败");
    assert_eq!(err.code, "FS_LOCKED", "{}", err.message);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "a",
        "被挡住时不能写"
    );

    drop(held);
    let receipt = tight
        .mutate(
            &path,
            &crate::capabilities::tools::ports::Expect::Version(version),
            crate::capabilities::tools::ports::Mutation::Replace("b".to_string()),
        )
        .expect("放锁后应能写");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "b");
    assert_eq!(receipt.total_lines, 1);
}

/// 目的：准备会话时播种 `.workignore`，且**不覆盖**用户已改的规则；规则落在版本库目录里、不进共享区。
#[test]
fn fs_workspace_prepare_seeds_workignore_once() {
    let root = scratch("fs-workignore-seed");
    let sessions = root.join("session");
    let ws = FsWorkspace::new(sessions.clone());
    ws.prepare("w", &["a".to_string()]).expect("建目录");

    let rules = sessions.join("w").join(".work").join(WORKIGNORE);
    assert_eq!(
        std::fs::read_to_string(&rules).expect("种子要落地"),
        WORKIGNORE_SEED,
        "初始规则就是常量这一份"
    );
    assert!(
        !sessions.join("w").join("work").join(WORKIGNORE).exists(),
        "规则不进共享区：它属于版本库目录"
    );

    std::fs::write(&rules, "*.log\n").expect("用户改规则");
    ws.prepare("w", &["a".to_string()]).expect("再次准备");
    assert_eq!(
        std::fs::read_to_string(&rules).expect("读回"),
        "*.log\n",
        "已存在的规则不被种子覆盖"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：`list_visible` 按规则剪枝，`list` 仍列全部——内容面只差"发现"这一步，语义只有一份。
#[test]
fn fs_work_store_lists_visible_files_by_rules() {
    let root = scratch("fs-list-visible");
    let shared = root.join("work");
    for rel in [
        "README.md",
        "src/main.rs",
        "node_modules/pkg/index.js",
        "build/out.o",
        ".git/config",
    ] {
        let p = shared.join(rel);
        std::fs::create_dir_all(p.parent().expect("父目录")).expect("造目录");
        std::fs::write(&p, rel).expect("造文件");
    }

    let store = FsWorkStore::new();
    assert_eq!(
        store.list(&shared).expect("列全部"),
        vec![
            ".git/config".to_string(),
            "README.md".to_string(),
            "build/out.o".to_string(),
            "node_modules/pkg/index.js".to_string(),
            "src/main.rs".to_string(),
        ],
        "不过滤时列全，排序稳定"
    );
    assert_eq!(
        store
            .list_visible(&shared, Some("node_modules\n.git\nbuild/\n"))
            .expect("按规则列"),
        vec!["README.md".to_string(), "src/main.rs".to_string()],
        "被忽略目录整棵剪掉"
    );
    assert_eq!(
        store.list_visible(&shared, None).expect("无规则"),
        store.list(&shared).expect("列全部"),
        "None 与不过滤等价"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：首次绑定外部工作区时只导入**算内容**的文件；显式点名的路径不受规则限制（同 `git add -f`）。
#[test]
fn work_import_root_respects_workignore_and_still_takes_explicit_paths() {
    let root = scratch("fs-import-workignore");
    let sessions = root.join("session");
    let external = root.join("project");
    for rel in ["README.md", "src/a.txt", "node_modules/x.js", ".git/config"] {
        let p = external.join(rel);
        std::fs::create_dir_all(p.parent().expect("父目录")).expect("造目录");
        std::fs::write(&p, rel).expect("造文件");
    }

    let ws = test_workspace_store(
        Arc::new(FsWorkspace::new(sessions.clone())),
        Arc::new(FsWorkStore::new()),
    );
    ws.bind("w", Some(&external)).expect("绑定外部工作区");
    ws.prepare("w", &["a".to_string()]).expect("准备");
    ws.work_import_root("w", 7).expect("初始导入");

    let mut tree = ws.work_status("w", "a", &[]).expect("读状态").tree;
    tree.sort();
    assert_eq!(
        tree,
        vec!["README.md".to_string(), "src/a.txt".to_string()],
        "被规则忽略的目录不进初始提交"
    );

    // 显式点名：规则之后仍能提交被忽略目录里的某个文件。
    let late = external.join("node_modules").join("late.js");
    std::fs::write(&late, "late").expect("造后加文件");
    let report = ws
        .work_sync(
            "w",
            &["node_modules/late.js".to_string()],
            8,
            CommitAnchor {
                agent: "a".to_string(),
                session: "w".to_string(),
                line: 1,
            },
        )
        .expect("显式点名同步");
    assert_eq!(
        report.changes.len(),
        1,
        "只有点名的那一条被采纳：{:?}",
        report.changes
    );
    assert_eq!(report.changes[0].path, "node_modules/late.js");
    assert!(
        report.changes[0].kind == ChangeKind::Add,
        "新文件按新增处理"
    );
    let _ = std::fs::remove_dir_all(&root);
}
