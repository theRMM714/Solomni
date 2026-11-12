//! 目的：空间回收的真实实现——回收锁、删除、排队、压缩与上次回收的落盘。
//! 管：.vm/storage.lock、各类落点的删除、构建卷保护、qemu-img 压缩、.vm/storage.json。
//! 不管：哪些该删（domain）、什么时候删（service）、路径约定（paths）。
//! 联动：service 经 SpaceReclaim 端口消费；删除用的路径与 survey 共用 detail/paths.rs。

use crate::capabilities::storage::api::{CAT_BUILD, CAT_DOWNLOAD, CAT_LOGS, CAT_OVERLAYS, LastGc};
use crate::capabilities::storage::domain::safe_id;
use crate::capabilities::storage::ports::{ReclaimLease, SpaceReclaim};
use std::path::{Path, PathBuf};

use super::{lock, paths, qemu};

/// 目的：按产品根执行删除、排队与压缩。
pub struct DiskReclaim {
    root: PathBuf,
}

impl DiskReclaim {
    /// 目的：组合根专用——取产品根。
    pub fn new(root: PathBuf) -> DiskReclaim {
        DiskReclaim { root }
    }
}

impl ReclaimLease for lock::GcLock {}

impl SpaceReclaim for DiskReclaim {
    fn lock(&self) -> Result<Option<Box<dyn ReclaimLease>>, String> {
        let lease = lock::try_acquire(&self.root)?;
        Ok(lease.map(|l| Box::new(l) as Box<dyn ReclaimLease>))
    }

    fn remove(&self, category: &str, id: &str) -> Result<(), String> {
        let root = &self.root;
        match category {
            CAT_BUILD => remove_build_volume(root, id),
            CAT_DOWNLOAD => remove_entry(&paths::download_dir(root).join(id)),
            CAT_OVERLAYS => remove_entry(&paths::overlay_dir(root).join(format!("{}.qcow2", id))),
            CAT_LOGS => remove_entry(&paths::log_dir(root).join(id)),
            _ => Err(format!("不支持自动回收的类别：{}", category)),
        }
    }

    fn queue_module_purge(&self, guest: &str, module: &str) -> Result<(), String> {
        if !safe_id(guest) {
            return Err(format!("虚拟根名不合法：{}", guest));
        }
        if paths::build_locked(&self.root, guest) {
            return Err(format!("虚拟根 {} 有构建在跑，不能标记清理", guest));
        }
        std::fs::write(
            paths::build_purge(&self.root, guest),
            format!("{}\n", module),
        )
        .map_err(|e| format!("写清理标记失败：{}", e))
    }

    fn compact(&self, guest: &str) -> Result<(u64, u64), String> {
        if !safe_id(guest) {
            return Err(format!("虚拟根名不合法：{}", guest));
        }
        if paths::build_locked(&self.root, guest) {
            return Err(format!("虚拟根 {} 有构建在跑，不能压缩", guest));
        }
        let Some(q) = qemu::find(&self.root) else {
            return Err("项目内没有 qemu-img，不能压缩（先补齐虚拟机依赖）".to_string());
        };
        let vol = paths::build_volume(&self.root, guest);
        if !vol.is_file() {
            return Err(format!("虚拟根 {} 没有构建缓存卷", guest));
        }
        let before = qemu::info(&q, &vol).map(|i| i.actual).unwrap_or(0);
        let after = qemu::convert(&q, &vol)?;
        Ok((before, after))
    }

    fn last_gc(&self) -> Option<LastGc> {
        super::state::read(&self.root)
    }

    fn record_gc(&self, last: &LastGc) -> Result<(), String> {
        super::state::write(&self.root, last)
    }

    fn build_locked(&self, guest: &str) -> bool {
        paths::build_locked(&self.root, guest)
    }
}

/// 目的：删掉一个虚拟根的构建缓存（卷、元数据、锁、待清理标记）。
fn remove_build_volume(root: &Path, guest: &str) -> Result<(), String> {
    if !safe_id(guest) {
        return Err(format!("虚拟根名不合法：{}", guest));
    }
    if paths::build_locked(root, guest) {
        return Err(format!("虚拟根 {} 有构建在跑，不能删缓存", guest));
    }
    let dir = paths::build_dir(root);
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

/// 目的：删一个文件或目录；不存在视为已删。
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
