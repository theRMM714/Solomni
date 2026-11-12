//! 目的：空间盘点的真实实现——把产品根下的落点读成分类事实。
//! 管：四类可回收缓存与三类只读事实的扫描、qcow2 事实、卷元数据、宿主余量。
//! 不管：预算与候选排序（domain/service）、回收锁与删除（reclaim）。
//! 联动：service 经 SpaceSurvey 端口消费；路径约定见 detail/paths.rs。

use crate::capabilities::storage::api::{
    CAT_BUILD, CAT_DOWNLOAD, CAT_GUESTS, CAT_LOGS, CAT_OUTPUTS, CAT_OVERLAYS, CAT_SESSIONS,
    SpaceCategory, SpaceItem,
};
use crate::capabilities::storage::ports::{SpaceFacts, SpaceSurvey};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{guests, paths, qemu, scan, space};

/// 目的：按产品根盘点空间事实。
pub struct DiskSurvey {
    root: PathBuf,
}

impl DiskSurvey {
    /// 目的：组合根专用——取产品根。
    pub fn new(root: PathBuf) -> DiskSurvey {
        DiskSurvey { root }
    }
}

impl SpaceSurvey for DiskSurvey {
    fn survey(&self, full: bool) -> SpaceFacts {
        let root = &self.root;
        // 一次盘点只取一次时间：分类里的"距今多久"与回收摘要用同一个刻度。
        let now = crate::kernel::detail::procs::now_ts();
        let qemu = qemu::find(root);
        let mut categories = vec![
            build(root, qemu.as_deref()),
            download(root),
            overlays(root, qemu.as_deref(), now),
            logs(root),
        ];
        if full {
            categories.push(guest_bases(root));
            categories.push(sessions(root));
            categories.push(module_outputs(root));
        }
        let (total_bytes, free_bytes) = space::total_free(root).unwrap_or((0, 0));
        SpaceFacts {
            generated_at: now,
            total_bytes,
            free_bytes,
            categories,
        }
    }
}

/// 目的：构建缓存卷旁的元数据（builds 写；缺失时降级为卷级）。
#[derive(Debug, Deserialize, Default)]
struct VolumeMeta {
    #[serde(default)]
    last_used: Option<u64>,
    #[serde(default)]
    modules: BTreeMap<String, serde_json::Value>,
}

fn build(root: &Path, qemu: Option<&Path>) -> SpaceCategory {
    let dir = paths::build_dir(root);
    let mut items = Vec::new();
    for p in scan::files_with_ext(&dir, "qcow2") {
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()).map(str::to_string) else {
            continue;
        };
        let active = paths::build_locked(root, &stem);
        let meta = read_meta(&paths::build_meta(root, &stem));
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
        CAT_BUILD,
        "构建缓存",
        items,
        true,
        "每虚拟根一块；可按整卷或模块回收",
    )
}

fn download(root: &Path) -> SpaceCategory {
    let items = scan::children(&paths::download_dir(root))
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
        CAT_DOWNLOAD,
        "下载缓存",
        items,
        true,
        "依赖补齐的下载归档与中间产物",
    )
}

fn overlays(root: &Path, qemu: Option<&Path>, now: u64) -> SpaceCategory {
    let live = guests::live_in(&root.join(".home"));
    let items = scan::files_with_ext(&paths::overlay_dir(root), "qcow2")
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
        CAT_OVERLAYS,
        "过期可写层",
        items,
        true,
        "会话销毁时删除；崩溃残留由回收兜底",
    )
}

fn logs(root: &Path) -> SpaceCategory {
    let items = scan::files_with_ext(&paths::log_dir(root), "log")
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
    category(CAT_LOGS, "日志", items, true, "按保留天数与最近份数回收")
}

fn guest_bases(root: &Path) -> SpaceCategory {
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
        CAT_GUESTS,
        "虚拟根基础镜像",
        items,
        false,
        "环境组件；还原会丢弃 guest 内状态",
    )
}

fn sessions(root: &Path) -> SpaceCategory {
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
        CAT_SESSIONS,
        "会话与工作区",
        items,
        false,
        "用户数据；永不自动回收",
    )
}

fn module_outputs(root: &Path) -> SpaceCategory {
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
        CAT_OUTPUTS,
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
