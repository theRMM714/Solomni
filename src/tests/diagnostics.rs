//! 目的：入口层的可观察契约——隐藏模式（自检 / 探针）与启动报告的围栏判据。
//! 管：构建的产品路径（查模块 / 查构建期能力 / 缓存 / 跑命令 / 入口复验 / 状态落盘）、
//!   `--doctor` 的事实本体、启动报告与 `--doctor` 共用同一份本机能力与本次实际。
//! 不管：CLI 参数解析（在 main.rs）、模块清单校验（在 `modules.rs`）、围栏机制本身（在 kernel）。
//! 联动：模块契约见 MODULE_SPEC 的「构建」；状态落 `.vm/build-state/<模块>.json`。

/// 目的：--work-gc 默认 dry-run、--yes 才删；只删不可达对象。
#[test]
fn work_gc_cli_dry_run_then_applies() {
    use crate::capabilities::workspace::api::{content_hash, Workspace};
    use crate::capabilities::workspace::detail::fs_workspace::FsWorkspace;
    use crate::capabilities::workspace::detail::fs_workstore::FsWorkStore;
    use crate::capabilities::workspace::ports::WorkStore;
    use crate::capabilities::workspace::service::WorkspaceService;
    use std::sync::Arc;

    let root = crate::tests::scratch("work-gc-cli");
    let dirs = Arc::new(FsWorkspace::new(root.join("session")));
    let store = Arc::new(FsWorkStore::new());
    let ws = WorkspaceService::new(dirs, store.clone());
    ws.prepare("w", &[]).unwrap();
    ws.work_commit_user("w", "keep.txt", b"keep", 1, 0).unwrap();
    let store_path = root.join("session").join("w").join(".work");
    let orphan = content_hash(b"orphan");
    store.write_object(&store_path, &orphan, b"orphan").unwrap();
    let keep = content_hash(b"keep");
    assert!(store_path.join("objects").join(&orphan).exists());

    let dry = crate::diagnostics::work_gc(&root, &["--work-gc".to_string(), "w".to_string()], 0);
    assert_eq!(dry, 0);
    assert!(
        store_path.join("objects").join(&orphan).exists(),
        "dry-run 不能删"
    );

    let applied = crate::diagnostics::work_gc(
        &root,
        &[
            "--work-gc".to_string(),
            "w".to_string(),
            "--yes".to_string(),
        ],
        0,
    );
    assert_eq!(applied, 0);
    assert!(
        !store_path.join("objects").join(&orphan).exists(),
        "apply 要删不可达对象"
    );
    assert!(
        store_path.join("objects").join(&keep).exists(),
        "可达对象必须保留"
    );
}

/// 目的：`--doctor` 的事实里要能看到"现在在跑什么"：账上记着、进程已经没了的条目照样如实报出来（`alive:false`）。
#[test]
pub(crate) fn doctor_reports_the_recorded_processes() {
    let root = crate::tests::scratch("doctor-procs");
    std::fs::create_dir_all(root.join(".home")).expect("建 .home");
    let book = crate::kernel::detail::procs::ProcessBook::open(&root.join(".home"));
    book.record(crate::kernel::detail::procs::Entry {
        pid: 999_999,
        since: 12,
        purpose: "guest".to_string(),
        owner: "w-doctor".to_string(),
        marker: "solomni:w-doctor".to_string(),
        created: None,
        guard_note: Some("把子进程绑进作业对象失败".to_string()),
    });
    let doc = crate::diagnostics::doctor_json(&root);
    let procs = doc
        .get("processes")
        .and_then(|p| p.as_array())
        .expect("要有 processes");
    assert_eq!(procs.len(), 1, "账上那条要报出来");
    assert_eq!(
        procs[0].get("purpose").and_then(|v| v.as_str()),
        Some("guest")
    );
    assert_eq!(
        procs[0].get("owner").and_then(|v| v.as_str()),
        Some("w-doctor")
    );
    assert_eq!(
        procs[0].get("alive").and_then(|v| v.as_bool()),
        Some(false),
        "进程早没了就要如实报 false"
    );
    assert_eq!(
        procs[0].get("orphanRisk").and_then(|v| v.as_bool()),
        Some(true),
        "父死子亡没挂上的条目要标出来（这条会留成孤儿）"
    );
    assert_eq!(
        procs[0].get("guardNote").and_then(|v| v.as_str()),
        Some("把子进程绑进作业对象失败"),
        "原因要原样报出来"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：路径级围栏的「本次实际」按平台判据：只有 Windows 受 `sandbox.policy.windows.write_acl` 授权影响。
///   用合成能力值，不依赖本机机制；Windows 分支由 Windows CI 真跑。
#[test]
fn effective_path_fence_only_depends_on_authorization_on_windows() {
    use crate::kernel::detail::confine::Capability;
    let cap = Capability {
        fs: true,
        net: true,
        tree: true,
        note: "测试".to_string(),
    };
    #[cfg(windows)]
    {
        assert_eq!(
            cap.effective_paths(false),
            (false, false),
            "Windows 未授权就不写 ACL"
        );
        assert_eq!(cap.effective_paths(true), (true, true), "授权后按本机能力");
    }
    #[cfg(not(windows))]
    {
        assert_eq!(
            cap.effective_paths(false),
            (true, true),
            "Unix 的围栏在守门进程里自足，授权与否都不影响"
        );
        assert_eq!(cap.effective_paths(true), (true, true));
    }
}

/// 目的：启动报告与 `--doctor` 共用同一份本机能力；Unix 的「本次实际」与它一致，
///   Windows 未授权时 fs/net 为假（容器围栏要写 ACL）——钉住启动报告与 `--doctor` 的口径。
#[test]
fn startup_fence_report_and_doctor_share_the_same_capability() {
    let cap = crate::kernel::detail::confine::capability();
    let report = crate::FenceReport::new(
        cap.clone(),
        false,
        std::sync::Arc::new(crate::kernel::api::SandboxMode::permissive()),
    );
    let root = crate::tests::scratch("fence-report");
    let doc = crate::diagnostics::doctor_json_with(&root, cap.clone());
    let fence = doc.get("fence").expect("--doctor 要报围栏事实");
    assert_eq!(
        fence.get("fs").and_then(|v| v.as_bool()),
        Some(cap.fs),
        "--doctor 报的是本机能力"
    );
    assert_eq!(
        fence.get("net").and_then(|v| v.as_bool()),
        Some(cap.net),
        "--doctor 报的是本机能力"
    );
    assert_eq!(fence.get("tree").and_then(|v| v.as_bool()), Some(cap.tree));
    assert_eq!(report.cap.tree, cap.tree, "进程树围栏与授权无关");
    #[cfg(windows)]
    {
        assert!(
            !report.fs && !report.net,
            "Windows 未授权时不写 ACL，容器围栏不生效"
        );
        assert!(!report.write_allowed);
    }
    #[cfg(not(windows))]
    {
        assert_eq!(
            (report.fs, report.net),
            (cap.fs, cap.net),
            "Unix 的本次实际就是本机能力，要与 --doctor 一致"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}
