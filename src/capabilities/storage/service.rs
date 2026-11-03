//! 目的：存储空间的用例实现——盘点、预算、自动回收、显式清理与虚拟根还原作业。
//! 管：策略与落盘事实的编排、回收锁、构建卷/下载缓存/可写层/日志的删除、还原作业的进度。
//! 不管：构建怎么跑（builds）、guest 怎么起（vm）、下载怎么装（provision）、工作对象可达性（workspace）。
//! 联动：组合根构造并注入 Ops；builds 在起 guest 前问 ensure_headroom；口径见 docs/storage/README.md。

use crate::capabilities::provision::api::Provision;
use crate::capabilities::storage::api::{
    CleanKind, CleanOutcome, CleanRequest, GcAction, GcReport, JobState, LastGc, SpaceCategory,
    SpaceItem, SpaceReport, Storage, StoragePolicy,
};
use crate::capabilities::storage::detail::{self, qemu, scan, space};
use crate::kernel::ports::Log;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const GIB: u64 = 1024 * 1024 * 1024;
const BUILD: &str = "build-cache";
const DOWNLOAD: &str = "download-cache";
const OVERLAYS: &str = "overlays";
const LOGS: &str = "logs";

/// 目的：存储能力实现；策略与作业状态由它持有。
pub struct StorageService {
    root: PathBuf,
    provision: Arc<dyn Provision + Send + Sync>,
    log: Arc<dyn Log + Send + Sync>,
    policy: Mutex<StoragePolicy>,
    job: Arc<Mutex<JobState>>,
}

/// 目的：构建缓存卷旁的元数据（builds 写；缺失时降级为卷级）。
#[derive(Debug, Deserialize, Default)]
struct VolumeMeta {
    #[serde(default)]
    last_used: Option<u64>,
    #[serde(default)]
    modules: BTreeMap<String, serde_json::Value>,
}

impl StorageService {
    /// 目的：组合根专用——注入产品根、补齐面与日志。
    pub fn new(
        root: PathBuf,
        provision: Arc<dyn Provision + Send + Sync>,
        log: Arc<dyn Log + Send + Sync>,
        policy: StoragePolicy,
    ) -> StorageService {
        StorageService {
            root,
            provision,
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
                let mut first = true;
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(if first {
                        5
                    } else {
                        15 * 60
                    }));
                    first = false;
                    if me.policy().auto_gc {
                        let _ = me.gc(true);
                    }
                }
            })
            .ok();
    }

    fn scan(&self, full: bool) -> Result<SpaceReport, String> {
        let root = &self.root;
        let qemu = qemu::find(root);
        let mut categories = vec![
            scan_build(root, qemu.as_deref()),
            scan_download(root),
            scan_overlays(root, qemu.as_deref()),
            scan_logs(root),
        ];
        if full {
            categories.push(scan_guests(root));
            categories.push(scan_sessions(root));
            categories.push(scan_module_outputs(root));
        }
        let managed_bytes = categories
            .iter()
            .filter(|c| c.auto_evictable)
            .map(|c| c.bytes)
            .sum();
        let (total_bytes, free_bytes) = space::total_free(root).unwrap_or((0, 0));
        let mut policy = self.policy();
        if policy.cache_cap_gib == 0 {
            let ten = total_bytes / 10;
            policy.cap_bytes = ten.clamp(10 * GIB, 50 * GIB);
        } else {
            policy.cap_bytes = policy.cache_cap_gib.saturating_mul(GIB);
        }
        Ok(SpaceReport {
            generated_at: crate::kernel::detail::procs::now_ts(),
            managed_bytes,
            cap_bytes: policy.cap_bytes,
            free_bytes,
            total_bytes,
            over_cap: managed_bytes > policy.cap_bytes,
            categories,
            policy,
            last_gc: detail::state::read(root),
            job: self.job(),
        })
    }

    fn gc_with(
        &self,
        apply: bool,
        protect: Option<&str>,
        reason: &str,
    ) -> Result<GcReport, String> {
        let _lock = if apply {
            match detail::lock::try_acquire(&self.root)? {
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
        let before = self.scan(false)?;
        let planned = crate::capabilities::storage::domain::plan(
            &before,
            before.policy,
            crate::kernel::detail::procs::now_ts(),
            protect,
        );
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
                match self.remove_one(&c.category, &c.id) {
                    Ok(()) => action.applied = true,
                    Err(e) => action.error = Some(e),
                }
            }
            report.actions.push(action);
        }
        let after = self.scan(false)?;
        report.after_bytes = after.managed_bytes;
        report.freed_bytes = before.managed_bytes.saturating_sub(after.managed_bytes);
        report.over_cap = after.over_cap;
        if apply && !report.actions.is_empty() {
            let last = LastGc {
                at: crate::kernel::detail::procs::now_ts(),
                freed_bytes: report.freed_bytes,
                reason: reason.to_string(),
                skipped: report.skipped.len(),
            };
            if let Err(e) = detail::state::write(&self.root, &last) {
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

    fn remove_one(&self, category: &str, id: &str) -> Result<(), String> {
        let root = &self.root;
        match category {
            BUILD => {
                if root_lock_active(root, id) {
                    return Err(format!("虚拟根 {} 有构建在跑，跳过", id));
                }
                remove_build_volume(root, id)
            }
            DOWNLOAD => remove_entry(&root.join(".vm").join("cache").join(id)),
            OVERLAYS => remove_entry(
                &root
                    .join(".vm")
                    .join("overlays")
                    .join(format!("{}.qcow2", id)),
            ),
            LOGS => remove_entry(&root.join("logs").join(id)),
            _ => Err(format!("不支持自动回收的类别：{}", category)),
        }
    }

    fn clean_report(&self, actions: Vec<GcAction>, applied: bool) -> Result<GcReport, String> {
        let report = self.scan(false)?;
        let mut out = GcReport {
            applied,
            before_bytes: report.managed_bytes,
            after_bytes: report.managed_bytes,
            over_cap: report.over_cap,
            actions,
            ..GcReport::default()
        };
        if applied {
            let after = self.scan(false)?;
            out.after_bytes = after.managed_bytes;
            out.freed_bytes = report.managed_bytes.saturating_sub(after.managed_bytes);
            out.over_cap = after.over_cap;
        }
        Ok(out)
    }

    fn start_reset(&self, guest: &str) -> Result<JobState, String> {
        if !safe_id(guest) {
            return Err(format!("虚拟根名不合法：{}", guest));
        }
        if any_live_guest(&self.root) {
            return Err("还有 guest 在跑：先停止所有虚拟机会话，再还原虚拟根".to_string());
        }
        if root_lock_active(&self.root, guest) {
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
        let root = self.root.clone();
        let provision = Arc::clone(&self.provision);
        let job = Arc::clone(&self.job);
        let log = Arc::clone(&self.log);
        let name = guest.to_string();
        std::thread::spawn(move || {
            let sink = Arc::clone(&job);
            let mut line = move |text: &str| push_line(&sink, text);
            let r = provision.reset_guest(&name, &mut line);
            if r.is_ok() {
                match remove_build_volume(&root, &name) {
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
        self.scan(true)
    }

    fn gc(&self, apply: bool) -> Result<GcReport, String> {
        self.gc_with(apply, None, if apply { "手动回收" } else { "预览" })
    }

    fn ensure_headroom(&self, protect: Option<&str>) -> Result<GcReport, String> {
        let policy = self.policy();
        if !policy.auto_gc {
            return Ok(GcReport::default());
        }
        let report = self.scan(false)?;
        let min_free = policy.min_free_gib.saturating_mul(GIB);
        let over = report.managed_bytes > report.cap_bytes;
        let low = report.free_bytes < min_free;
        if !over && !low {
            return Ok(GcReport::default());
        }
        let out = self.gc_with(true, protect, "构建前自动回收")?;
        if low {
            let after = self.scan(false)?;
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
        let _lock = match detail::lock::try_acquire(&self.root)? {
            Some(l) => Some(l),
            None => return Err("另一个回收正在跑，稍后重试".to_string()),
        };
        let root = &self.root;
        let mut actions: Vec<GcAction> = Vec::new();
        match req.kind {
            CleanKind::BuildCache => {
                if let Some(module) = req.module.as_deref().filter(|s| !s.trim().is_empty()) {
                    let Some(guest) = req.id.as_deref() else {
                        return Err("按模块清理要指定虚拟根".to_string());
                    };
                    if !safe_id(guest) {
                        return Err(format!("虚拟根名不合法：{}", guest));
                    }
                    if root_lock_active(root, guest) {
                        return Err(format!("虚拟根 {} 有构建在跑，不能标记清理", guest));
                    }
                    let p = root
                        .join(".vm")
                        .join("build-cache")
                        .join(format!("{}.purge", guest));
                    std::fs::write(&p, format!("{}\n", module))
                        .map_err(|e| format!("写清理标记失败：{}", e))?;
                    actions.push(GcAction {
                        category: BUILD.to_string(),
                        id: guest.to_string(),
                        label: format!("{} · 模块 {}", guest, module),
                        reason: "已排队：下次构建前回收该模块工作区".to_string(),
                        applied: true,
                        ..GcAction::default()
                    });
                } else if let Some(guest) = req.id.as_deref().filter(|s| !s.trim().is_empty()) {
                    if root_lock_active(root, guest) {
                        return Err(format!("虚拟根 {} 有构建在跑，不能清理", guest));
                    }
                    let bytes = scan::bytes(
                        &root
                            .join(".vm")
                            .join("build-cache")
                            .join(format!("{}.qcow2", guest)),
                    );
                    remove_build_volume(root, guest)?;
                    actions.push(GcAction {
                        category: BUILD.to_string(),
                        id: guest.to_string(),
                        label: format!("构建缓存 · {}", guest),
                        bytes,
                        reason: "手动清理整卷".to_string(),
                        applied: true,
                        ..GcAction::default()
                    });
                } else {
                    for p in scan::files_with_ext(&root.join(".vm").join("build-cache"), "qcow2") {
                        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
                            continue;
                        };
                        if root_lock_active(root, stem) {
                            continue;
                        }
                        let bytes = scan::bytes(&p);
                        remove_build_volume(root, stem)?;
                        actions.push(GcAction {
                            category: BUILD.to_string(),
                            id: stem.to_string(),
                            label: format!("构建缓存 · {}", stem),
                            bytes,
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
                for p in scan::children(&root.join(".vm").join("cache")) {
                    let bytes = scan::bytes(&p);
                    remove_entry(&p)?;
                    actions.push(GcAction {
                        category: DOWNLOAD.to_string(),
                        id: scan::name_of(&p),
                        label: format!("下载缓存 · {}", scan::name_of(&p)),
                        bytes,
                        reason: "手动清空下载缓存（重装需重新下载）".to_string(),
                        applied: true,
                        ..GcAction::default()
                    });
                }
            }
            CleanKind::Overlays => {
                let dir = root.join(".vm").join("overlays");
                for p in scan::files_with_ext(&dir, "qcow2") {
                    let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
                        continue;
                    };
                    if let Some(want) = req.id.as_deref() {
                        if want != stem {
                            continue;
                        }
                    }
                    if any_live_guest(root) {
                        continue;
                    }
                    let bytes = scan::bytes(&p);
                    remove_entry(&p)?;
                    actions.push(GcAction {
                        category: OVERLAYS.to_string(),
                        id: stem.to_string(),
                        label: format!("过期可写层 · {}", stem),
                        bytes,
                        reason: "手动清理过期可写层".to_string(),
                        applied: true,
                        ..GcAction::default()
                    });
                }
            }
            CleanKind::Logs => {
                let dir = root.join("logs");
                let keep = self.policy().log_keep_days;
                for p in scan::files_with_ext(&dir, "log") {
                    let old = scan::modified(&p)
                        .map(|t| {
                            crate::kernel::detail::procs::now_ts().saturating_sub(t)
                                > keep.saturating_mul(86_400)
                        })
                        .unwrap_or(false);
                    if keep > 0 && !old {
                        continue;
                    }
                    let bytes = scan::bytes(&p);
                    remove_entry(&p)?;
                    actions.push(GcAction {
                        category: LOGS.to_string(),
                        id: scan::name_of(&p),
                        label: format!("日志 · {}", scan::name_of(&p)),
                        bytes,
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
                if root_lock_active(root, guest) {
                    return Err(format!("虚拟根 {} 有构建在跑，不能压缩", guest));
                }
                let Some(q) = qemu::find(root) else {
                    return Err("项目内没有 qemu-img，不能压缩（先补齐虚拟机依赖）".to_string());
                };
                let vol = root
                    .join(".vm")
                    .join("build-cache")
                    .join(format!("{}.qcow2", guest));
                if !vol.is_file() {
                    return Err(format!("虚拟根 {} 没有构建缓存卷", guest));
                }
                let before = qemu::info(&q, &vol).map(|i| i.actual).unwrap_or(0);
                let after = qemu::convert(&q, &vol)?;
                actions.push(GcAction {
                    category: BUILD.to_string(),
                    id: guest.to_string(),
                    label: format!("构建缓存 · {}", guest),
                    bytes: before.saturating_sub(after),
                    reason: format!("压缩完成：{} -> {} 字节", before, after),
                    applied: true,
                    ..GcAction::default()
                });
            }
            CleanKind::GuestReset => unreachable!(),
        }
        self.clean_report(actions, true).map(CleanOutcome::Report)
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

fn scan_build(root: &Path, qemu: Option<&Path>) -> SpaceCategory {
    let dir = root.join(".vm").join("build-cache");
    let mut items = Vec::new();
    for p in scan::files_with_ext(&dir, "qcow2") {
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()).map(str::to_string) else {
            continue;
        };
        let lock = dir.join(format!("{}.lock", stem));
        let active = lock_active(&lock);
        let meta = read_meta(&dir.join(format!("{}.json", stem)));
        let info = qemu.and_then(|q| qemu::info(q, &p));
        let bytes = info.map(|i| i.actual).unwrap_or_else(|| scan::bytes(&p));
        let virtual_bytes = info.map(|i| i.virtual_size);
        let last_used = meta
            .as_ref()
            .and_then(|m| m.last_used)
            .or_else(|| scan::modified(&p));
        let mods = meta.as_ref().map(|m| m.modules.len()).unwrap_or(0);
        items.push(SpaceItem {
            id: stem.clone(),
            label: format!("构建缓存 · {}", stem),
            bytes,
            virtual_bytes,
            last_used,
            active,
            detail: if active {
                format!(
                    "构建占用中{}",
                    if mods > 0 {
                        format!("；{} 个模块", mods)
                    } else {
                        String::new()
                    }
                )
            } else if mods > 0 {
                format!("{} 个模块", mods)
            } else {
                "卷级（模块统计缺失）".to_string()
            },
            modules: meta
                .as_ref()
                .map(|m| m.modules.keys().cloned().collect())
                .unwrap_or_default(),
            cleanable: true,
        });
    }
    category(
        BUILD,
        "构建缓存",
        items,
        true,
        "每虚拟根一块；可按整卷或模块回收",
    )
}

fn scan_download(root: &Path) -> SpaceCategory {
    let dir = root.join(".vm").join("cache");
    let items = scan::children(&dir)
        .into_iter()
        .map(|p| SpaceItem {
            id: scan::name_of(&p),
            label: format!("下载缓存 · {}", scan::name_of(&p)),
            bytes: scan::bytes(&p),
            virtual_bytes: None,
            last_used: scan::modified(&p),
            active: false,
            detail: "重装需要重新下载".to_string(),
            modules: Vec::new(),
            cleanable: true,
        })
        .collect();
    category(
        DOWNLOAD,
        "下载缓存",
        items,
        true,
        "依赖补齐的下载归档与中间产物",
    )
}

fn scan_overlays(root: &Path, qemu: Option<&Path>) -> SpaceCategory {
    let dir = root.join(".vm").join("overlays");
    let live = any_live_guest(root);
    let now = crate::kernel::detail::procs::now_ts();
    let items = scan::files_with_ext(&dir, "qcow2")
        .into_iter()
        .map(|p| {
            let stem = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("overlay")
                .to_string();
            let last = scan::modified(&p);
            let info = qemu.and_then(|q| qemu::info(q, &p));
            SpaceItem {
                id: stem.clone(),
                label: format!("可写层 · {}", stem),
                bytes: info.map(|i| i.actual).unwrap_or_else(|| scan::bytes(&p)),
                virtual_bytes: info.map(|i| i.virtual_size),
                last_used: last,
                active: live || last.map(|t| now.saturating_sub(t) < 600).unwrap_or(false),
                detail: if live {
                    "有 guest 在跑，受保护"
                } else {
                    "会话销毁后应被删除；崩溃残留可回收"
                }
                .to_string(),
                modules: Vec::new(),
                cleanable: true,
            }
        })
        .collect();
    category(
        OVERLAYS,
        "过期可写层",
        items,
        true,
        "会话销毁时删除；崩溃残留由回收兜底",
    )
}

fn scan_logs(root: &Path) -> SpaceCategory {
    let items = scan::files_with_ext(&root.join("logs"), "log")
        .into_iter()
        .map(|p| SpaceItem {
            id: scan::name_of(&p),
            label: format!("日志 · {}", scan::name_of(&p)),
            bytes: scan::bytes(&p),
            virtual_bytes: None,
            last_used: scan::modified(&p),
            active: false,
            detail: "运行日志".to_string(),
            modules: Vec::new(),
            cleanable: true,
        })
        .collect();
    category(LOGS, "日志", items, true, "按保留天数与最近份数回收")
}

fn scan_guests(root: &Path) -> SpaceCategory {
    let items = scan::children(&root.join(".vm").join("guest"))
        .into_iter()
        .filter(|p| p.is_dir())
        .map(|p| {
            let name = scan::name_of(&p);
            SpaceItem {
                id: name.clone(),
                label: format!("虚拟根 · {}", name),
                bytes: scan::bytes(&p),
                virtual_bytes: None,
                last_used: scan::modified(&p),
                active: false,
                detail: "环境组件；只可还原到初始状态，不自动回收".to_string(),
                modules: Vec::new(),
                cleanable: true,
            }
        })
        .collect();
    category(
        "guest-bases",
        "虚拟根基础镜像",
        items,
        false,
        "环境组件；还原会丢弃 guest 内状态",
    )
}

fn scan_sessions(root: &Path) -> SpaceCategory {
    let items = scan::children(&root.join("session"))
        .into_iter()
        .filter(|p| p.is_dir())
        .map(|p| SpaceItem {
            id: scan::name_of(&p),
            label: format!("工作 · {}", scan::name_of(&p)),
            bytes: scan::bytes(&p),
            virtual_bytes: None,
            last_used: scan::modified(&p),
            active: false,
            detail: "用户数据；删除会话才会回收".to_string(),
            modules: Vec::new(),
            cleanable: false,
        })
        .collect();
    category(
        "sessions",
        "会话与工作区",
        items,
        false,
        "用户数据；永不自动回收",
    )
}

fn scan_module_outputs(root: &Path) -> SpaceCategory {
    let items = scan::children(&root.join("modules"))
        .into_iter()
        .filter(|p| p.is_dir())
        .filter_map(|p| {
            let build = p.join("build");
            if !build.is_dir() {
                return None;
            }
            let name = scan::name_of(&p);
            Some(SpaceItem {
                id: name.clone(),
                label: format!("模块产物 · {}", name),
                bytes: scan::bytes(&build),
                virtual_bytes: None,
                last_used: scan::modified(&build),
                active: false,
                detail: "可重建；不自动回收".to_string(),
                modules: Vec::new(),
                cleanable: false,
            })
        })
        .collect();
    category(
        "module-outputs",
        "模块产物",
        items,
        false,
        "宿主侧构建产物；不自动回收",
    )
}

fn category(
    id: &str,
    label: &str,
    items: Vec<SpaceItem>,
    auto_evictable: bool,
    note: &str,
) -> SpaceCategory {
    let bytes = items.iter().map(|i| i.bytes).sum();
    SpaceCategory {
        id: id.to_string(),
        label: label.to_string(),
        bytes,
        items,
        auto_evictable,
        note: note.to_string(),
    }
}

fn read_meta(path: &Path) -> Option<VolumeMeta> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn any_live_guest(root: &Path) -> bool {
    crate::kernel::detail::procs::ProcessBook::open(&root.join(".home"))
        .list()
        .iter()
        .any(|e| e.purpose == "guest" && crate::kernel::detail::procs::alive(e.pid))
}

fn lock_active(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let text = std::fs::read_to_string(path).unwrap_or_default();
    if let Some(pid) = text
        .split_whitespace()
        .skip_while(|w| *w != "pid")
        .nth(1)
        .and_then(|s| s.parse::<u32>().ok())
    {
        if crate::kernel::detail::procs::alive(pid) {
            return true;
        }
    }
    meta.modified()
        .ok()
        .and_then(|t| std::time::SystemTime::now().duration_since(t).ok())
        .map(|d| d < std::time::Duration::from_secs(3 * 3600))
        .unwrap_or(false)
}

fn root_lock_active(root: &Path, guest: &str) -> bool {
    lock_active(
        &root
            .join(".vm")
            .join("build-cache")
            .join(format!("{}.lock", guest)),
    )
}

fn remove_build_volume(root: &Path, guest: &str) -> Result<(), String> {
    if !safe_id(guest) {
        return Err(format!("虚拟根名不合法：{}", guest));
    }
    if root_lock_active(root, guest) {
        return Err(format!("虚拟根 {} 有构建在跑，不能删缓存", guest));
    }
    let dir = root.join(".vm").join("build-cache");
    for name in [
        format!("{}.qcow2", guest),
        format!("{}.json", guest),
        format!("{}.lock", guest),
        format!("{}.purge", guest),
    ] {
        remove_entry(&dir.join(name))?;
    }
    Ok(())
}

fn remove_entry(p: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => {
            std::fs::remove_dir_all(p).map_err(|e| format!("删 {} 失败：{}", p.display(), e))
        }
        Ok(_) => std::fs::remove_file(p).map_err(|e| format!("删 {} 失败：{}", p.display(), e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("看 {} 失败：{}", p.display(), e)),
    }
}

fn safe_id(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}
