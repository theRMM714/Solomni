//! 目的：空间回收的真实实现——回收锁、删除、排队、压缩与上次回收的落盘。
//! 管：回收锁、各类落点的删除、构建卷保护（抢构建锁）、qemu-img 压缩、回收记要。
//! 不管：哪些该删（domain）、什么时候删（service）、路径约定（paths）。
//! 联动：service 经 SpaceReclaim 端口消费；删除用的路径与 survey 共用 detail/paths.rs。

use crate::kernel::api::fs;
use crate::units::business::maintenance::storage::api::{
    CAT_BUILD, CAT_DOWNLOAD, CAT_OVERLAYS, LastGc,
};
use crate::units::business::maintenance::storage::domain::safe_id;
use crate::units::business::maintenance::storage::ports::{ReclaimLease, SpaceReclaim};
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

/// 目的：回收锁的持有凭据就是内核锁句柄——Drop 即释放，崩溃时由操作系统释放。
impl ReclaimLease for std::fs::File {}

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
            _ => Err(format!("不支持自动回收的类别：{}", category)),
        }
    }

    fn queue_module_purge(&self, guest: &str, module: &str) -> Result<(), String> {
        if !safe_id(guest) {
            return Err(format!("虚拟根名不合法：{}", guest));
        }
        let _lease =
            take_build_lock(&self.root, guest).map_err(|e| format!("{}，不能标记清理", e))?;
        fs::write(
            None,
            &paths::build_purge(&self.root, guest),
            format!("{}\n", module).as_bytes(),
            fs::Expect::Any,
        )
        .map(|_| ())
        .map_err(|fault| format!("写清理标记失败：{}", fault.message))
    }

    fn compact(&self, guest: &str) -> Result<(u64, u64), String> {
        if !safe_id(guest) {
            return Err(format!("虚拟根名不合法：{}", guest));
        }
        let _lease = take_build_lock(&self.root, guest).map_err(|e| format!("{}，不能压缩", e))?;
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

/// 目的：删掉一个虚拟根的构建缓存（卷、元数据、待清理标记）；锁文件留着，身份由它承担。
fn remove_build_volume(root: &Path, guest: &str) -> Result<(), String> {
    if !safe_id(guest) {
        return Err(format!("虚拟根名不合法：{}", guest));
    }
    let _lease = take_build_lock(root, guest).map_err(|e| format!("{}，不能删缓存", e))?;
    let dir = paths::build_dir(root);
    for name in [
        format!("{}.qcow2", guest),
        format!("{}.json", guest),
        format!("{}.purge", guest),
    ] {
        remove_entry(&dir.join(name))?;
    }
    Ok(())
}

/// 目的：非阻塞抢同一虚拟根的构建锁并持到作用域结束——抢不到就是有构建在跑。
/// 约束：与删卷 / 排队 / 压缩同作用域，判断与动手之间没有空档。
fn take_build_lock(root: &Path, guest: &str) -> Result<std::fs::File, String> {
    let path = paths::build_lock(root, guest);
    if let Some(dir) = path.parent() {
        fs::ensure_dir(dir).map_err(|fault| {
            format!("建构建缓存目录失败（{}）：{}", dir.display(), fault.message)
        })?;
    }
    crate::kernel::api::lock::try_lock(&path, "构建缓存卷")?
        .ok_or_else(|| format!("虚拟根 {} 有构建在跑", guest))
}

/// 目的：删一个文件或目录；不存在视为已删。
fn remove_entry(p: &Path) -> Result<(), String> {
    fs::remove_tree(p).map_err(|fault| format!("删 {} 失败：{}", p.display(), fault.message))
}
