//! 目的：存储空间的用例实现——盘点、预算、自动回收、显式清理与虚拟根还原作业。
//! 管：策略与落盘事实的编排、回收锁的持有、按策略挑候选、还原作业的进度。
//! 不管：目录怎么扫（survey）、文件怎么删（reclaim）、guest 怎么起（vm）、下载怎么装（provision）。
//! 联动：组合根构造并注入三个端口；builds 在起 guest 前问 ensure_headroom；口径见 docs/storage/README.md。

use crate::capabilities::provision::api::Provision;
use crate::capabilities::storage::api::{
    CleanKind, CleanOutcome, CleanRequest, GcAction, GcReport, JobState, LastGc, SpaceCategory,
    SpaceItem, SpaceReport, Storage, StoragePolicy, CAT_BUILD, CAT_DOWNLOAD, CAT_LOGS,
    CAT_OVERLAYS,
};
use crate::capabilities::storage::domain::{self, safe_id};
use crate::capabilities::storage::ports::{GuestLiveness, SpaceReclaim, SpaceSurvey};
use crate::kernel::ports::Log;
use std::sync::{Arc, Mutex};

const GIB: u64 = 1024 * 1024 * 1024;

/// 目的：存储能力实现；策略与作业状态由它持有，磁盘事实与删除经三个端口。
pub struct StorageService {
    provision: Arc<dyn Provision + Send + Sync>,
    survey: Arc<dyn SpaceSurvey + Send + Sync>,
    reclaim: Arc<dyn SpaceReclaim + Send + Sync>,
    guests: Arc<dyn GuestLiveness + Send + Sync>,
    log: Arc<dyn Log + Send + Sync>,
    policy: Mutex<StoragePolicy>,
    job: Arc<Mutex<JobState>>,
}

impl StorageService {
    /// 目的：组合根专用——注入补齐面、盘点、回收、存活与日志。
    pub fn new(
        provision: Arc<dyn Provision + Send + Sync>,
        survey: Arc<dyn SpaceSurvey + Send + Sync>,
        reclaim: Arc<dyn SpaceReclaim + Send + Sync>,
        guests: Arc<dyn GuestLiveness + Send + Sync>,
        log: Arc<dyn Log + Send + Sync>,
        policy: StoragePolicy,
    ) -> StorageService {
        StorageService {
            provision,
            survey,
            reclaim,
            guests,
            log,
            policy: Mutex::new(policy),
            job: Arc::new(Mutex::new(JobState::default())),
        }
    }

    /// 目的：启动后台自动回收（主进程用；测试不调）。
    pub fn start_auto(self: &Arc<Self>) {
        let me = Arc::clone(self);
        std::thread::Builder::new()
            .name("solomni-storage".to_string())
            .spawn(move || {
                let mut last = std::time::Instant::now();
                loop {
                    // 30 秒一醒：策略改了（关掉 / 调间隔）最多半分钟就生效，不必重启。
                    std::thread::sleep(std::time::Duration::from_secs(30));
                    let policy = me.policy();
                    if !policy.periodic_gc {
                        last = std::time::Instant::now();
                        continue;
                    }
                    let minutes = policy.gc_interval_minutes.max(15);
                    if last.elapsed() >= std::time::Duration::from_secs(minutes * 60) {
                        let _ = me.gc(true);
                        last = std::time::Instant::now();
                    }
                }
            })
            .ok();
    }

    fn scan(&self, full: bool) -> SpaceReport {
        let facts = self.survey.survey(full);
        let managed_bytes = facts
            .categories
            .iter()
            .filter(|c| c.auto_evictable)
            .map(|c| c.bytes)
            .sum();
        let mut policy = self.policy();
        if policy.cache_cap_gib == 0 {
            let ten = facts.total_bytes / 10;
            policy.cap_bytes = ten.clamp(10 * GIB, 50 * GIB);
        } else {
            policy.cap_bytes = policy.cache_cap_gib.saturating_mul(GIB);
        }
        SpaceReport {
            generated_at: facts.generated_at,
            managed_bytes,
            cap_bytes: policy.cap_bytes,
            free_bytes: facts.free_bytes,
            total_bytes: facts.total_bytes,
            over_cap: managed_bytes > policy.cap_bytes,
            categories: facts.categories,
            policy,
            last_gc: self.reclaim.last_gc(),
            job: self.job(),
        }
    }

    fn gc_with(
        &self,
        apply: bool,
        protect: Option<&str>,
        reason: &str,
    ) -> Result<GcReport, String> {
        let _lease = if apply {
            match self.reclaim.lock()? {
                Some(l) => Some(l),
                None => {
                    return Ok(GcReport {
                        applied: false,
                        skipped: vec![GcAction {
                            category: "storage".to_string(),
                            id: "lock".to_string(),
                            label: "另一个回收正在跑".to_string(),
                            reason: "稍后重试".to_string(),
                            ..GcAction::default()
                        }],
                        ..GcReport::default()
                    });
                }
            }
        } else {
            None
        };
        let before = self.scan(false);
        let planned = domain::plan(&before, before.policy, before.generated_at, protect);
        let mut report = GcReport {
            applied: apply,
            before_bytes: before.managed_bytes,
            over_cap: before.over_cap,
            ..GcReport::default()
        };
        for c in planned {
            let mut action = GcAction {
                category: c.category.clone(),
                id: c.id.clone(),
                label: c.label.clone(),
                bytes: c.bytes,
                reason: c.reason,
                ..GcAction::default()
            };
            if apply {
                match self.reclaim.remove(&c.category, &c.id) {
                    Ok(()) => action.applied = true,
                    Err(e) => action.error = Some(e),
                }
            }
            report.actions.push(action);
        }
        let after = self.scan(false);
        report.after_bytes = after.managed_bytes;
        report.freed_bytes = before.managed_bytes.saturating_sub(after.managed_bytes);
        report.over_cap = after.over_cap;
        if apply && !report.actions.is_empty() {
            let last = LastGc {
                at: after.generated_at,
                freed_bytes: report.freed_bytes,
                reason: reason.to_string(),
                skipped: report.skipped.len(),
            };
            if let Err(e) = self.reclaim.record_gc(&last) {
                self.log.warn("storage::state", &e);
            }
            self.log.info(
                "storage::gc",
                &format!(
                    "{}：{} 项，释放 {} 字节，仍超限={}",
                    reason,
                    report.actions.len(),
                    report.freed_bytes,
                    report.over_cap
                ),
            );
        }
        Ok(report)
    }

    fn clean_report(&self, before_bytes: u64, actions: Vec<GcAction>, applied: bool) -> GcReport {
        let after = self.scan(false);
        GcReport {
            applied,
            before_bytes,
            after_bytes: after.managed_bytes,
            freed_bytes: before_bytes.saturating_sub(after.managed_bytes),
            over_cap: after.over_cap,
            actions,
            ..GcReport::default()
        }
    }

    fn start_reset(&self, guest: &str) -> Result<JobState, String> {
        if !safe_id(guest) {
            return Err(format!("虚拟根名不合法：{}", guest));
        }
        if self.guests.any_running() {
            return Err("还有 guest 在跑：先停止所有虚拟机会话，再还原虚拟根".to_string());
        }
        if self.reclaim.build_locked(guest) {
            return Err(format!("虚拟根 {} 有构建在跑：等构建结束再还原", guest));
        }
        {
            let mut j = self.job.lock().map_err(|_| "作业状态锁坏了".to_string())?;
            if j.running {
                return Err("已有空间作业在跑".to_string());
            }
            *j = JobState {
                running: true,
                seq: 0,
                lines: Vec::new(),
                error: None,
            };
        }
        let provision = Arc::clone(&self.provision);
        let reclaim = Arc::clone(&self.reclaim);
        let job = Arc::clone(&self.job);
        let log = Arc::clone(&self.log);
        let name = guest.to_string();
        std::thread::spawn(move || {
            let sink = Arc::clone(&job);
            let mut line = move |text: &str| push_line(&sink, text);
            let r = provision.reset_guest(&name, &mut line);
            if r.is_ok() {
                match reclaim.remove(CAT_BUILD, &name) {
                    Ok(()) => push_line(&job, &format!("已清空虚拟根 {} 的构建缓存", name)),
                    Err(e) => push_line(&job, &format!("还原完成，但清构建缓存失败：{}", e)),
                }
            }
            if let Ok(mut j) = job.lock() {
                j.running = false;
                if let Err(e) = r {
                    log.error(
                        "storage::reset",
                        &format!("虚拟根 {} 还原失败：{}", name, e),
                    );
                    j.error = Some(e);
                } else {
                    log.info(
                        "storage::reset",
                        &format!("虚拟根 {} 已还原到初始状态", name),
                    );
                }
            }
        });
        Ok(self.job())
    }
}

impl Storage for StorageService {
    fn report(&self) -> Result<SpaceReport, String> {
        Ok(self.scan(true))
    }

    fn gc(&self, apply: bool) -> Result<GcReport, String> {
        self.gc_with(apply, None, if apply { "手动回收" } else { "预览" })
    }

    fn ensure_headroom(&self, protect: Option<&str>) -> Result<GcReport, String> {
        // 构建 / 起 guest 前的 headroom 是安全闸，不受“周期回收”开关控制。
        let policy = self.policy();
        let report = self.scan(false);
        let min_free = policy.min_free_gib.saturating_mul(GIB);
        let over = report.managed_bytes > report.cap_bytes;
        let low = report.free_bytes < min_free;
        if !over && !low {
            return Ok(GcReport::default());
        }
        let out = self.gc_with(true, protect, "构建前自动回收")?;
        if low {
            let after = self.scan(false);
            if after.free_bytes < min_free {
                return Err(format!(
                    "宿主剩余空间仍不足（{} MiB，低于 storage.min_free_gib）；先清理缓存或换磁盘",
                    after.free_bytes >> 20
                ));
            }
        }
        Ok(out)
    }

    fn clean(&self, req: &CleanRequest) -> Result<CleanOutcome, String> {
        if !req.confirm {
            return Err("需要二次确认（confirm=true）才会清理".to_string());
        }
        if req.kind == CleanKind::GuestReset {
            let Some(guest) = req.id.as_deref().filter(|s| !s.trim().is_empty()) else {
                return Err("还原虚拟根要指定虚拟根名".to_string());
            };
            return self.start_reset(guest).map(CleanOutcome::Started);
        }
        let _lease = match self.reclaim.lock()? {
            Some(l) => Some(l),
            None => return Err("另一个回收正在跑，稍后重试".to_string()),
        };
        let before = self.scan(false);
        let now = before.generated_at;
        let mut actions: Vec<GcAction> = Vec::new();
        match req.kind {
            CleanKind::BuildCache => {
                let builds = items_of(&before.categories, CAT_BUILD);
                if let Some(module) = req.module.as_deref().filter(|s| !s.trim().is_empty()) {
                    let Some(guest) = req.id.as_deref() else {
                        return Err("按模块清理要指定虚拟根".to_string());
                    };
                    self.reclaim.queue_module_purge(guest, module)?;
                    actions.push(GcAction {
                        category: CAT_BUILD.to_string(),
                        id: guest.to_string(),
                        label: format!("{} · 模块 {}", guest, module),
                        reason: "已排队：下次构建前回收该模块工作区".to_string(),
                        applied: true,
                        ..GcAction::default()
                    });
                } else if let Some(guest) = req.id.as_deref().filter(|s| !s.trim().is_empty()) {
                    let bytes = builds
                        .iter()
                        .find(|i| i.id == guest)
                        .map(|i| i.bytes)
                        .unwrap_or(0);
                    self.reclaim.remove(CAT_BUILD, guest)?;
                    actions.push(GcAction {
                        category: CAT_BUILD.to_string(),
                        id: guest.to_string(),
                        label: format!("构建缓存 · {}", guest),
                        bytes,
                        reason: "手动清理整卷".to_string(),
                        applied: true,
                        ..GcAction::default()
                    });
                } else {
                    for item in &builds {
                        if self.reclaim.build_locked(&item.id) {
                            continue;
                        }
                        self.reclaim.remove(CAT_BUILD, &item.id)?;
                        actions.push(GcAction {
                            category: CAT_BUILD.to_string(),
                            id: item.id.clone(),
                            label: item.label.clone(),
                            bytes: item.bytes,
                            reason: "手动清空全部构建缓存".to_string(),
                            applied: true,
                            ..GcAction::default()
                        });
                    }
                }
            }
            CleanKind::DownloadCache => {
                if self.provision.job().running {
                    return Err("依赖补齐作业在跑，等它结束再清下载缓存".to_string());
                }
                for item in items_of(&before.categories, CAT_DOWNLOAD) {
                    self.reclaim.remove(CAT_DOWNLOAD, &item.id)?;
                    actions.push(GcAction {
                        category: CAT_DOWNLOAD.to_string(),
                        id: item.id.clone(),
                        label: item.label,
                        bytes: item.bytes,
                        reason: "手动清空下载缓存（重装需重新下载）".to_string(),
                        applied: true,
                        ..GcAction::default()
                    });
                }
            }
            CleanKind::Overlays => {
                let live = self.guests.any_running();
                for item in items_of(&before.categories, CAT_OVERLAYS) {
                    if let Some(want) = req.id.as_deref() {
                        if want != item.id {
                            continue;
                        }
                    }
                    if live {
                        continue;
                    }
                    self.reclaim.remove(CAT_OVERLAYS, &item.id)?;
                    actions.push(GcAction {
                        category: CAT_OVERLAYS.to_string(),
                        id: item.id.clone(),
                        label: item.label,
                        bytes: item.bytes,
                        reason: "手动清理过期可写层".to_string(),
                        applied: true,
                        ..GcAction::default()
                    });
                }
            }
            CleanKind::Logs => {
                let keep = self.policy().log_keep_days;
                for item in items_of(&before.categories, CAT_LOGS) {
                    let old = item
                        .last_used
                        .map(|t| now.saturating_sub(t) > keep.saturating_mul(86_400))
                        .unwrap_or(false);
                    if keep > 0 && !old {
                        continue;
                    }
                    self.reclaim.remove(CAT_LOGS, &item.id)?;
                    actions.push(GcAction {
                        category: CAT_LOGS.to_string(),
                        id: item.id.clone(),
                        label: item.label,
                        bytes: item.bytes,
                        reason: "手动清理日志".to_string(),
                        applied: true,
                        ..GcAction::default()
                    });
                }
            }
            CleanKind::Compact => {
                let Some(guest) = req.id.as_deref().filter(|s| !s.trim().is_empty()) else {
                    return Err("压缩要指定虚拟根".to_string());
                };
                let (was, now_bytes) = self.reclaim.compact(guest)?;
                actions.push(GcAction {
                    category: CAT_BUILD.to_string(),
                    id: guest.to_string(),
                    label: format!("构建缓存 · {}", guest),
                    bytes: was.saturating_sub(now_bytes),
                    reason: format!("压缩完成：{} -> {} 字节", was, now_bytes),
                    applied: true,
                    ..GcAction::default()
                });
            }
            CleanKind::GuestReset => unreachable!(),
        }
        Ok(CleanOutcome::Report(self.clean_report(
            before.managed_bytes,
            actions,
            true,
        )))
    }

    fn job(&self) -> JobState {
        self.job.lock().map(|j| j.clone()).unwrap_or_default()
    }

    fn set_policy(&self, policy: StoragePolicy) {
        if let Ok(mut p) = self.policy.lock() {
            *p = policy;
        }
    }

    fn policy(&self) -> StoragePolicy {
        self.policy
            .lock()
            .map(|p| *p)
            .unwrap_or_else(|e| *e.into_inner())
    }
}

fn push_line(job: &Arc<Mutex<JobState>>, text: &str) {
    if let Ok(mut j) = job.lock() {
        j.seq += 1;
        j.lines.push(text.to_string());
        if j.lines.len() > 200 {
            j.lines.remove(0);
        }
    }
}

fn items_of(categories: &[SpaceCategory], id: &str) -> Vec<SpaceItem> {
    categories
        .iter()
        .find(|c| c.id == id)
        .map(|c| c.items.clone())
        .unwrap_or_default()
}
