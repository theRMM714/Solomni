//! 目的：存储空间的契约——策略排序、盘点、回收幂等、锁与显式清理。
//! 管：storage 能力的 T1/T2 用例；不碰真实 .vm / session / .home。
//! 不管：构建怎么跑（builds.rs）、guest 生命周期（vm.rs）。
//! 联动：契约见 docs/storage/README.md；入口 node run-tests.js。

use crate::capabilities::storage::api::{
    CleanKind, CleanOutcome, CleanRequest, SpaceCategory, SpaceItem, SpaceReport, StoragePolicy,
};
use crate::capabilities::storage::domain;

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
    let svc = crate::capabilities::storage::service::StorageService::new(
        crate::tests::scratch("storage-confirm"),
        crate::tests::doubles::test_provision(),
        std::sync::Arc::new(crate::kernel::ports::NoopLog),
        StoragePolicy::default(),
    );
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
    let svc = crate::capabilities::storage::service::StorageService::new(
        root.clone(),
        crate::tests::doubles::test_provision(),
        std::sync::Arc::new(crate::kernel::ports::NoopLog),
        StoragePolicy::default(),
    );
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
    assert!(build
        .items
        .iter()
        .any(|i| i.modules == vec!["m".to_string()]));
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
    let svc = crate::capabilities::storage::service::StorageService::new(
        root.clone(),
        crate::tests::doubles::test_provision(),
        std::sync::Arc::new(crate::kernel::ports::NoopLog),
        StoragePolicy::default(),
    );
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
    let svc = crate::capabilities::provision::ProvisionService::new(&root);
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
