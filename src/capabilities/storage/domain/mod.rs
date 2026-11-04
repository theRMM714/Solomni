//! 目的：存储空间的纯策略——哪些缓存项先回收、按龄还是按上限。
//! 管：plan 的排序与选择；不碰文件系统。
//! 不管：扫描、删除、锁（在 detail/service）。
//! 联动：service 调 plan 得到候选；测试直接钉策略。

use crate::capabilities::storage::api::{SpaceCategory, SpaceReport, StoragePolicy};
use std::collections::BTreeSet;

/// 目的：一条待回收候选；category 是 SpaceCategory.id。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub category: String,
    pub id: String,
    pub label: String,
    pub bytes: u64,
    pub reason: String,
}

const LOGS: &str = "logs";
const OVERLAYS: &str = "overlays";
const DOWNLOAD: &str = "download-cache";
const BUILD: &str = "build-cache";

/// 目的：按策略排一次回收候选；不删任何东西。
/// 参数：report = 当前盘点；policy = 预算；now = unix 秒；protect = 本次要用的虚拟根。
/// 返回：按“最容易再生/最久未用”排序的候选；active 项不进入。
pub fn plan(
    report: &SpaceReport,
    policy: StoragePolicy,
    now: u64,
    protect: Option<&str>,
) -> Vec<Candidate> {
    let keep_secs = policy.cache_keep_days.saturating_mul(86_400);
    let cutoff = now.saturating_sub(keep_secs);
    let log_cutoff = now.saturating_sub(policy.log_keep_days.saturating_mul(86_400));
    let mut out: Vec<Candidate> = Vec::new();
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();

    // 按龄：日志、过期可写层、下载缓存、构建卷。
    for cat in &report.categories {
        if !cat.auto_evictable {
            continue;
        }
        match cat.id.as_str() {
            LOGS => {
                let mut logs: Vec<_> = cat.items.iter().collect();
                logs.sort_by_key(|i| i.last_used.unwrap_or(0));
                let keep_recent = 50usize;
                for (idx, item) in logs.iter().enumerate() {
                    let aged = item.last_used.map(|t| t < log_cutoff).unwrap_or(false);
                    if (aged && policy.log_keep_days > 0) || idx + keep_recent < logs.len() {
                        add(&mut out, &mut seen, cat, item, "超过日志保留期限");
                    }
                }
            }
            OVERLAYS => {
                for item in &cat.items {
                    if !item.active
                        && item
                            .last_used
                            .map(|t| now.saturating_sub(t) > 600)
                            .unwrap_or(true)
                    {
                        add(&mut out, &mut seen, cat, item, "过期可写层");
                    }
                }
            }
            DOWNLOAD if keep_secs > 0 => {
                for item in &cat.items {
                    if item.last_used.map(|t| t < cutoff).unwrap_or(false) {
                        add(&mut out, &mut seen, cat, item, "超过缓存保留期限");
                    }
                }
            }
            BUILD if keep_secs > 0 => {
                for item in &cat.items {
                    if item.active || protect == Some(item.id.as_str()) {
                        continue;
                    }
                    if item.last_used.map(|t| t < cutoff).unwrap_or(false) {
                        add(&mut out, &mut seen, cat, item, "超过缓存保留期限");
                    }
                }
            }
            _ => {}
        }
    }

    // 超上限：把还没选中的可回收项按最近使用升序补到够为止。
    let mut over = report.managed_bytes.saturating_sub(policy.cap_bytes);
    if over > 0 {
        let mut rest: Vec<(
            String,
            &SpaceCategory,
            &crate::capabilities::storage::api::SpaceItem,
        )> = Vec::new();
        for cat in &report.categories {
            if !cat.auto_evictable {
                continue;
            }
            for item in &cat.items {
                if item.active || !item.cleanable {
                    continue;
                }
                if cat.id == BUILD && protect == Some(item.id.as_str()) {
                    continue;
                }
                if seen.contains(&(cat.id.clone(), item.id.clone())) {
                    continue;
                }
                rest.push((cat.id.clone(), cat, item));
            }
        }
        rest.sort_by_key(|(_, _, item)| item.last_used.unwrap_or(0));
        for (_, cat, item) in rest {
            if over == 0 {
                break;
            }
            if add(
                &mut out,
                &mut seen,
                cat,
                item,
                "超出缓存上限，按最近使用回收",
            ) {
                over = over.saturating_sub(item.bytes);
            }
        }
    }
    out
}

fn add(
    out: &mut Vec<Candidate>,
    seen: &mut BTreeSet<(String, String)>,
    cat: &SpaceCategory,
    item: &crate::capabilities::storage::api::SpaceItem,
    reason: &str,
) -> bool {
    if !item.cleanable || item.active {
        return false;
    }
    if !seen.insert((cat.id.clone(), item.id.clone())) {
        return false;
    }
    out.push(Candidate {
        category: cat.id.clone(),
        id: item.id.clone(),
        label: item.label.clone(),
        bytes: item.bytes,
        reason: reason.to_string(),
    });
    true
}
