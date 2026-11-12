//! 目的：存储空间的契约——策略排序、盘点、回收幂等、锁与显式清理。
//! 管：storage 能力的 T1/T2 用例与三个端口的契约；不碰真实 .vm / session / .home。
//! 不管：构建怎么跑（builds.rs）、guest 生命周期（vm.rs）。
//! 联动：契约见 docs/storage/README.md；入口 node run-tests.js。

use crate::capabilities::storage::api::{
    CleanKind, CleanOutcome, CleanRequest, LastGc, SpaceCategory, SpaceItem, SpaceReport,
    StoragePolicy,
};
use crate::capabilities::storage::domain;
use crate::capabilities::storage::service::StorageService;

/// 目的：用真实适配器装一个存储能力（隔离根；假补齐面与无声日志）。
fn service(root: &std::path::Path) -> StorageService {
    use crate::capabilities::storage::detail::guests::ProcessGuests;
    use crate::capabilities::storage::detail::reclaim::DiskReclaim;
    use crate::capabilities::storage::detail::survey::DiskSurvey;
    StorageService::new(
        crate::tests::doubles::test_provision(),
        std::sync::Arc::new(DiskSurvey::new(root.to_path_buf())),
        std::sync::Arc::new(DiskReclaim::new(root.to_path_buf())),
        std::sync::Arc::new(ProcessGuests::new(root)),
        std::sync::Arc::new(crate::kernel::ports::NoopLog),
        StoragePolicy::default(),
    )
}

fn report(categories: Vec<SpaceCategory>) -> SpaceReport {
    let managed_bytes = categories
        .iter()
        .filter(|c| c.auto_evictable)
        .map(|c| c.bytes)
        .sum();
    SpaceReport {
        managed_bytes,
        cap_bytes: 50,
        categories,
        ..SpaceReport::default()
    }
}

fn item(id: &str, bytes: u64, last: u64, active: bool) -> SpaceItem {
    SpaceItem {
        id: id.to_string(),
        label: id.to_string(),
        bytes,
        last_used: Some(last),
        active,
        modules: Vec::new(),
        cleanable: true,
        ..SpaceItem::default()
    }
}

/// 目的：超上限时先删最久未用的构建卷；活跃卷不进入候选。
#[test]
fn plan_prefers_oldest_build_volume_and_skips_active() {
    let cat = SpaceCategory {
        id: "build-cache".to_string(),
        label: "构建缓存".to_string(),
        bytes: 30,
        items: vec![
            item("old", 10, 1, false),
            item("new", 20, 100, false),
            item("busy", 30, 0, true),
        ],
        auto_evictable: true,
        note: String::new(),
    };
    let r = report(vec![cat]);
    let policy = StoragePolicy {
        cache_cap_gib: 1,
        cap_bytes: 25,
        cache_keep_days: 30,
        ..StoragePolicy::default()
    };
    let plan = domain::plan(&r, policy, 200, None);
    assert_eq!(plan.first().map(|c| c.id.as_str()), Some("old"));
    assert!(!plan.iter().any(|c| c.id == "busy"));
}

/// 目的：按龄回收日志/下载缓存，不把未超龄的项带上。
#[test]
fn plan_reclaims_aged_download_cache() {
    let cat = SpaceCategory {
        id: "download-cache".to_string(),
        label: "下载缓存".to_string(),
        bytes: 10,
        items: vec![item("old.bin", 10, 1, false)],
        auto_evictable: true,
        note: String::new(),
    };
    let r = report(vec![cat]);
    let policy = StoragePolicy {
        cache_keep_days: 1,
        cap_bytes: 100,
        ..StoragePolicy::default()
    };
    let plan = domain::plan(&r, policy, 200_000, None);
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].category, "download-cache");
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
    let svc = service(&crate::tests::scratch("storage-confirm"));
    assert!(crate::capabilities::storage::api::Storage::clean(&svc, &req).is_err());
    let _ = CleanOutcome::Report(Default::default());
}

/// 目的：盘点能读出卷的实际大小与模块元数据；到上限回收删除最旧卷。
#[test]
fn report_and_gc_delete_the_oldest_volume_in_a_scratch_root() {
    use crate::capabilities::storage::api::Storage;
    let root = crate::tests::scratch("storage-gc");
    let cache = root.join(".vm").join("build-cache");
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
    use crate::capabilities::storage::api::Storage;
    let root = crate::tests::scratch("storage-active");
    let cache = root.join(".vm").join("build-cache");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("busy.qcow2"), b"busy").unwrap();
    std::fs::write(
        cache.join("busy.lock"),
        format!("pid {} at now", std::process::id()),
    )
    .unwrap();
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
    std::fs::remove_file(cache.join("busy.lock")).unwrap();
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

/// 目的：不是产品清单构造的虚拟根不能“还原”，避免把用户自备 guest 删掉却重建不出来。
#[test]
fn reset_refuses_a_guest_not_in_the_manifest() {
    use crate::capabilities::provision::api::Provision;
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

/// 目的：全局回收间隔至少 15 分钟；默认 15。
#[test]
fn gc_interval_is_at_least_fifteen_minutes() {
    let mut app = crate::capabilities::registry::api::AppSettings::default();
    assert_eq!(app.storage.gc_interval_minutes, 15);
    app.storage.gc_interval_minutes = 14;
    let err = crate::capabilities::registry::domain::providers::validate_storage_settings(&app)
        .expect_err("14 分钟要拒绝");
    assert!(err.contains("至少 15 分钟"), "{}", err);
    app.storage.gc_interval_minutes = 15;
    crate::capabilities::registry::domain::providers::validate_storage_settings(&app)
        .expect("15 分钟是合法下限");
}

/// 目的：盘点端口给时间、分类与宿主余量；非 full 不扫用户数据。
#[test]
fn survey_reports_time_categories_and_host_room() {
    use crate::capabilities::storage::detail::survey::DiskSurvey;
    use crate::capabilities::storage::ports::SpaceSurvey;
    let root = crate::tests::scratch("storage-survey");
    let cache = root.join(".vm").join("build-cache");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("a.qcow2"), b"x").unwrap();
    let facts = DiskSurvey::new(root.clone()).survey(false);
    assert!(facts.generated_at > 0);
    assert!(facts.total_bytes > 0, "宿主总量应可读");
    let build = facts
        .categories
        .iter()
        .find(|c| c.id == "build-cache")
        .unwrap();
    assert_eq!(build.items.len(), 1);
    assert!(
        facts.categories.iter().all(|c| c.id != "sessions"),
        "非 full 不扫用户数据"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：回收锁同一时刻只给一个人；凭据 drop 后放行。
#[test]
fn reclaim_lock_is_exclusive_and_released_on_drop() {
    use crate::capabilities::storage::detail::reclaim::DiskReclaim;
    use crate::capabilities::storage::ports::SpaceReclaim;
    let root = crate::tests::scratch("storage-lease");
    let held = DiskReclaim::new(root.clone())
        .lock()
        .unwrap()
        .expect("第一次应拿到锁");
    assert!(
        DiskReclaim::new(root.clone()).lock().unwrap().is_none(),
        "第二个回收应被挡"
    );
    drop(held);
    assert!(
        DiskReclaim::new(root.clone()).lock().unwrap().is_some(),
        "drop 后应能再拿"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：有构建锁的卷不许删；删失败的现场要留着。
#[test]
fn reclaim_refuses_a_volume_that_a_build_is_using() {
    use crate::capabilities::storage::detail::reclaim::DiskReclaim;
    use crate::capabilities::storage::ports::SpaceReclaim;
    let root = crate::tests::scratch("storage-reclaim-locked");
    let cache = root.join(".vm").join("build-cache");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("busy.qcow2"), b"busy").unwrap();
    std::fs::write(
        cache.join("busy.lock"),
        format!("pid {} at now", std::process::id()),
    )
    .unwrap();
    let err = DiskReclaim::new(root.clone())
        .remove("build-cache", "busy")
        .unwrap_err();
    assert!(err.contains("有构建在跑"), "{}", err);
    assert!(cache.join("busy.qcow2").exists());
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：按模块清理写成 虚拟根.purge 标记；名字不安全一律拒绝。
#[test]
fn reclaim_queues_a_module_purge_marker() {
    use crate::capabilities::storage::detail::reclaim::DiskReclaim;
    use crate::capabilities::storage::ports::SpaceReclaim;
    let root = crate::tests::scratch("storage-purge");
    std::fs::create_dir_all(root.join(".vm").join("build-cache")).unwrap();
    let r = DiskReclaim::new(root.clone());
    r.queue_module_purge("dev", "indexer").unwrap();
    let text =
        std::fs::read_to_string(root.join(".vm").join("build-cache").join("dev.purge")).unwrap();
    assert_eq!(text.trim(), "indexer");
    assert!(
        r.queue_module_purge("../x", "m").is_err(),
        "名字不安全要拒绝"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：上次回收的摘要能落盘并原样读回。
#[test]
fn reclaim_round_trips_the_last_gc_summary() {
    use crate::capabilities::storage::detail::reclaim::DiskReclaim;
    use crate::capabilities::storage::ports::SpaceReclaim;
    let root = crate::tests::scratch("storage-last-gc");
    let r = DiskReclaim::new(root.clone());
    assert!(r.last_gc().is_none());
    r.record_gc(&LastGc {
        at: 42,
        freed_bytes: 7,
        reason: "tests".to_string(),
        skipped: 1,
    })
    .unwrap();
    let back = r.last_gc().unwrap();
    assert_eq!((back.at, back.freed_bytes, back.skipped), (42, 7, 1));
    assert_eq!(back.reason, "tests");
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：没有进程台账时不算有 guest 在跑。
#[test]
fn guests_are_absent_without_a_process_ledger() {
    use crate::capabilities::storage::detail::guests::ProcessGuests;
    use crate::capabilities::storage::ports::GuestLiveness;
    let root = crate::tests::scratch("storage-guests");
    assert!(!ProcessGuests::new(&root).any_running());
    let _ = std::fs::remove_dir_all(&root);
}
