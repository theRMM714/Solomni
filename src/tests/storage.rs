//! 目的：存储空间的契约——策略排序、盘点、回收幂等、锁与显式清理。
//! 管：storage 子业务的 T1/T2 用例与三个端口的契约；不碰真实 .vm / session / .home。
//! 不管：构建怎么跑（builds.rs）、guest 生命周期（vm.rs）。
//! 联动：契约见 docs/storage/README.md；入口 node run-tests.js。

use crate::units::business::maintenance::storage::api::{
    CleanKind, CleanOutcome, CleanRequest, StoragePolicy,
};

/// 目的：用组合根装一个存储能力（隔离根 + 假补齐面 + 可指定的运行日志）。用例只经入站面断言可观察结果。
fn service_with_active(
    root: &std::path::Path,
    active_log: Option<std::path::PathBuf>,
) -> impl crate::units::business::maintenance::storage::api::Storage {
    crate::entry::assembly::storage(
        root,
        crate::tests::doubles::test_provision(),
        StoragePolicy::default(),
        std::sync::Arc::new(crate::kernel::ports::NoopLog),
        active_log,
    )
}

/// 目的：不需要模拟运行日志时的简写。
fn service(
    root: &std::path::Path,
) -> impl crate::units::business::maintenance::storage::api::Storage {
    service_with_active(root, None)
}

/// 目的：未接入时显式清理如实拒绝；确认标志缺失也拒绝。
#[test]
fn clean_requires_confirmation_and_real_service() {
    let req = CleanRequest {
        kind: CleanKind::BuildCache,
        id: None,
        module: None,
        confirm: false,
    };
    let root = crate::tests::scratch("storage-confirm");
    let svc = service(&root);
    assert!(crate::units::business::maintenance::storage::api::Storage::clean(&svc, &req).is_err());
    let _ = CleanOutcome::Report(Default::default());
}

/// 目的：盘点能读出卷的实际大小与模块元数据；到上限回收删除最旧卷。
#[test]
fn report_and_gc_delete_the_oldest_volume_in_a_scratch_root() {
    use crate::units::business::maintenance::storage::api::Storage;
    let root = crate::tests::scratch("storage-gc");
    let cache = crate::kernel::domain::layout::vm(&root).join("build-cache");
    std::fs::create_dir_all(&cache).unwrap();
    for (id, used) in [("old", 1), ("new", 999)] {
        let f = cache.join(format!("{}.qcow2", id));
        let file = std::fs::File::create(&f).unwrap();
        file.set_len(700 * 1024 * 1024).unwrap();
        std::fs::write(
            cache.join(format!("{}.json", id)),
            format!(r#"{{"last_used":{},"modules":{{"m":{{}}}}}}"#, used),
        )
        .unwrap();
    }
    let svc = service(&root);
    let policy = StoragePolicy {
        cache_cap_gib: 1,
        cap_bytes: 1,
        cache_keep_days: 0,
        ..StoragePolicy::default()
    };
    svc.set_policy(policy);
    let r = svc.report().unwrap();
    let build = r.categories.iter().find(|c| c.id == "build-cache").unwrap();
    assert_eq!(build.items.len(), 2);
    assert!(
        build
            .items
            .iter()
            .any(|i| i.modules == vec!["m".to_string()])
    );
    let plan = svc.gc(false).unwrap();
    assert!(!plan.applied);
    assert!(plan.actions.iter().any(|a| a.id == "old"));
    let applied = svc.gc(true).unwrap();
    assert!(applied.applied);
    assert!(!cache.join("old.qcow2").exists(), "最旧卷应被删");
    assert!(cache.join("new.qcow2").exists(), "较新的卷应保留");
    let again = svc.gc(true).unwrap();
    assert_eq!(again.freed_bytes, 0, "幂等：第二次没有可回收项");
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：有构建锁的卷不进候选；按模块清理只写排队标记。
#[test]
fn active_volume_is_protected_and_module_cleanup_is_queued() {
    use crate::units::business::maintenance::storage::api::Storage;
    let root = crate::tests::scratch("storage-active");
    let cache = crate::kernel::domain::layout::vm(&root).join("build-cache");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("busy.qcow2"), b"busy").unwrap();
    let lease = crate::kernel::api::lock::try_lock(&cache.join("busy.lock"), "构建缓存卷")
        .unwrap()
        .expect("要拿得到构建锁");
    let svc = service(&root);
    let policy = StoragePolicy {
        cap_bytes: 1,
        cache_keep_days: 30,
        ..StoragePolicy::default()
    };
    svc.set_policy(policy);
    let plan = svc.gc(true).unwrap();
    assert!(plan.actions.iter().all(|a| a.id != "busy"));
    assert!(cache.join("busy.qcow2").exists());
    drop(lease);
    let req = CleanRequest {
        kind: CleanKind::BuildCache,
        id: Some("busy".to_string()),
        module: Some("indexer".to_string()),
        confirm: true,
    };
    let out = svc.clean(&req).unwrap();
    match out {
        CleanOutcome::Report(r) => assert!(r.actions[0].reason.contains("下次构建前")),
        _ => panic!("按模块清理应是同步排队"),
    }
    let text = std::fs::read_to_string(cache.join("busy.purge")).unwrap();
    assert_eq!(text.trim(), "indexer");
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：手动“清理整类”日志是全清——历史日志删完，只留当前进程正在写的那一份。
#[test]
fn clean_logs_deletes_all_but_the_active_file() {
    use crate::units::business::maintenance::storage::api::Storage;
    let root = crate::tests::scratch("storage-logs-clean");
    let logs = crate::kernel::domain::layout::logs(&root);
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(logs.join("old.log"), b"old").unwrap();
    std::fs::write(logs.join("mid.log"), b"mid").unwrap();
    let log = std::sync::Arc::new(
        crate::kernel::detail::file_log::FileLog::new(&root, "storage-logs-clean")
            .expect("建运行日志"),
    );
    let active = log.path().to_path_buf();
    let svc = crate::entry::assembly::storage(
        &root,
        crate::tests::doubles::test_provision(),
        StoragePolicy::default(),
        log,
        Some(active.clone()),
    );
    let active_name = active
        .file_name()
        .expect("文件名")
        .to_string_lossy()
        .to_string();

    let r = svc.report().unwrap();
    let cat = r.categories.iter().find(|c| c.id == "logs").unwrap();
    assert_eq!(cat.items.len(), 3);
    let running = cat.items.iter().find(|i| i.id == active_name).unwrap();
    assert!(running.active, "运行中的日志要标为 active");
    assert!(!running.cleanable, "运行中的日志不可清理");

    let req = CleanRequest {
        kind: CleanKind::Logs,
        id: None,
        module: None,
        confirm: true,
    };
    let CleanOutcome::Report(report) = svc.clean(&req).unwrap() else {
        panic!("手动清理是同步回执")
    };
    assert_eq!(report.actions.len(), 2, "两份历史日志都要删");
    assert_eq!(report.freed_bytes, 6);
    assert!(active.exists(), "运行中的日志要留下");
    assert!(!logs.join("old.log").exists());
    assert!(!logs.join("mid.log").exists());

    // 只剩运行中的那一份：再清一次没有候选，如实回 0，不假装成功。
    let CleanOutcome::Report(again) = svc.clean(&req).unwrap() else {
        panic!("手动清理是同步回执")
    };
    assert!(again.actions.is_empty());
    assert_eq!(again.freed_bytes, 0);
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：没有运行中的日志时，手动清理把 logs 下全部日志删光。
#[test]
fn clean_logs_without_a_running_file_deletes_everything() {
    use crate::units::business::maintenance::storage::api::Storage;
    let root = crate::tests::scratch("storage-logs-all");
    let logs = crate::kernel::domain::layout::logs(&root);
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(logs.join("a.log"), b"1234567890").unwrap();
    let log = std::sync::Arc::new(PurgingLog { dir: logs.clone() });
    let svc = crate::entry::assembly::storage(
        &root,
        crate::tests::doubles::test_provision(),
        StoragePolicy::default(),
        log,
        None,
    );
    let req = CleanRequest {
        kind: CleanKind::Logs,
        id: None,
        module: None,
        confirm: true,
    };
    let CleanOutcome::Report(report) = svc.clean(&req).unwrap() else {
        panic!("手动清理是同步回执")
    };
    assert_eq!(report.actions.len(), 1);
    assert_eq!(report.freed_bytes, 10);
    assert!(!logs.join("a.log").exists());
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：把清理打到指定日志目录的替身——没有“本次运行那份”，用来验证清空整类。
struct PurgingLog {
    dir: std::path::PathBuf,
}

impl crate::kernel::ports::Log for PurgingLog {
    fn info(&self, _at: &str, _msg: &str) {}
    fn warn(&self, _at: &str, _msg: &str) {}
    fn error(&self, _at: &str, _msg: &str) {}
    fn purge(&self, name: &str) -> bool {
        std::fs::remove_file(self.dir.join(name)).is_ok()
    }
}

/// 目的：不是产品清单构造的虚拟根不能“还原”，避免把用户自备 guest 删掉却重建不出来。
#[test]
fn reset_refuses_a_guest_not_in_the_manifest() {
    use crate::units::business::provision::api::Provision;
    let root = crate::tests::scratch("storage-reset-unknown");
    let svc = crate::entry::assembly::provision(&root);
    let mut lines = Vec::new();
    let err = svc
        .reset_guest("nope", &mut |line| lines.push(line.to_string()))
        .expect_err("未知虚拟根要拒绝");
    assert!(err.contains("不是产品清单构造"), "{}", err);
    assert!(lines.is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：模块产物盘点读安装根的 `modules/<id>/build`；数据根里的同名目录不算（构建产物回写模块目录）。
#[test]
fn module_outputs_are_read_from_the_module_source_tree() {
    use crate::units::business::maintenance::storage::api::Storage;
    let root = crate::tests::scratch("storage-outputs");
    let src = crate::kernel::domain::layout::modules_src(&root)
        .join("demo")
        .join("build");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("artifact.bin"), vec![0u8; 4096]).unwrap();
    let decoy = crate::kernel::domain::layout::modules_data(&root)
        .join("demo")
        .join("build");
    std::fs::create_dir_all(&decoy).unwrap();
    std::fs::write(decoy.join("artifact.bin"), vec![0u8; 8192]).unwrap();
    let r = service(&root).report().unwrap();
    let cat = r
        .categories
        .iter()
        .find(|c| c.id == "module-outputs")
        .unwrap();
    assert_eq!(cat.items.len(), 1);
    assert_eq!(cat.items[0].id, "demo");
    assert_eq!(cat.items[0].bytes, 4096);
}
