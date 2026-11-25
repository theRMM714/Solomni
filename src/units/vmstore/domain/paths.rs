//! 目的：`data/.vm` 的布局——每个子目录与文件名的唯一拼法。
//! 管：构建缓存（卷 / 元数据 / 锁 / 待清理标记）、下载缓存、可写层、构建状态、虚拟根、便携引擎、安装台账、回收状态与回收锁。
//! 不管：这些落点上发生的事（构建、补齐、回收、起虚拟机）；`.vm` 这个介质名本身（在 `kernel` 的 `layout`）。
//! 联动：`builds` / `provision` / `storage` / `vm` 的 detail 经它取路径；口径见 docs/vmstore/README.md。

use std::path::{Path, PathBuf};

use crate::kernel::domain::layout;

/// 目的：`.vm` 下的落点集合——只持产品根，派生纯路径。
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    /// 目的：按产品根造一份布局。
    /// 参数：`root` = 产品安装根。
    /// 返回：只持根的布局值（可反复派生，不碰磁盘）。
    pub fn at(root: &Path) -> Paths {
        Paths {
            root: root.to_path_buf(),
        }
    }

    /// 目的：`.vm` 本身。
    fn vm(&self) -> PathBuf {
        layout::vm(&self.root)
    }

    /// 目的：构建缓存卷的目录。
    pub fn build_cache(&self) -> PathBuf {
        self.vm().join("build-cache")
    }

    /// 目的：一个虚拟根的构建缓存卷（`<build-cache>/<虚拟根>.qcow2`）。
    pub fn build_volume(&self, guest: &str) -> PathBuf {
        self.build_cache().join(format!("{}.qcow2", safe_segment(guest)))
    }

    /// 目的：构建缓存卷旁的元数据（`<build-cache>/<虚拟根>.json`；构建写，盘点读）。
    pub fn build_meta(&self, guest: &str) -> PathBuf {
        self.build_cache()
            .join(format!("{}.json", safe_segment(guest)))
    }

    /// 目的：同一虚拟根的构建锁（`<build-cache>/<虚拟根>.lock`；构建持，补齐与盘点看）。
    pub fn build_lock(&self, guest: &str) -> PathBuf {
        self.build_cache()
            .join(format!("{}.lock", safe_segment(guest)))
    }

    /// 目的：按模块清理的排队标记（`<build-cache>/<虚拟根>.purge`，下次构建前消费）。
    pub fn build_purge(&self, guest: &str) -> PathBuf {
        self.build_cache()
            .join(format!("{}.purge", safe_segment(guest)))
    }

    /// 目的：依赖补齐的下载缓存目录（cloud 镜像、deb 与 tar 的落地处）。
    pub fn downloads(&self) -> PathBuf {
        self.vm().join("cache")
    }

    /// 目的：会话可写层目录（每个租约一份 qcow2 覆盖层）。
    pub fn overlays(&self) -> PathBuf {
        self.vm().join("overlays")
    }

    /// 目的：构建状态目录（`<build-state>/`）。
    pub fn build_states(&self) -> PathBuf {
        self.vm().join("build-state")
    }

    /// 目的：一个模块的构建状态文件（`<build-state>/<模块>.json`）。
    pub fn build_state(&self, module: &str) -> PathBuf {
        self.build_states()
            .join(format!("{}.json", safe_segment(module)))
    }

    /// 目的：一个模块那次构建的私有家目录（`<build-state>/<模块>/home`）。
    pub fn build_home(&self, module: &str) -> PathBuf {
        self.build_states()
            .join(safe_segment(module))
            .join("home")
    }

    /// 目的：虚拟根目录（`<guests>/`）。
    pub fn guests(&self) -> PathBuf {
        self.vm().join("guest")
    }

    /// 目的：一个虚拟根的目录（`<guests>/<名>`）。
    pub fn guest(&self, name: &str) -> PathBuf {
        self.guests().join(safe_segment(name))
    }

    /// 目的：便携引擎目录（`<engines>/`，QEMU 这类工具落在这里）。
    pub fn engines(&self) -> PathBuf {
        self.vm().join("qemu")
    }

    /// 目的：安装台账文件（`state/deps.json`）。
    pub fn ledger(&self) -> PathBuf {
        self.vm().join("state").join("deps.json")
    }

    /// 目的：回收状态文件（`storage.json`，只记上次回收摘要）。
    pub fn storage_state(&self) -> PathBuf {
        self.vm().join("storage.json")
    }

    /// 目的：回收互斥锁（`storage.lock`）。
    pub fn storage_lock(&self) -> PathBuf {
        self.vm().join("storage.lock")
    }
}

/// 目的：把名字收成一段安全的路径段（虚拟根名与模块 id 都只该是一段，这里再兜一层）。
/// 约束：与构建缓存的卷名同源——同一个虚拟根在卷、锁、元数据上必须落到同一个 stem（R13：跨单元不许同形重复）。
fn safe_segment(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 目的：所有落点都在 `.vm` 下，且各自的形状与文档一致。
    #[test]
    fn every_landing_sits_under_vm() {
        let p = Paths::at(Path::new("/w"));
        let vm = layout::vm(Path::new("/w"));
        assert_eq!(p.build_cache(), vm.join("build-cache"));
        assert_eq!(p.build_volume("debian-13"), vm.join("build-cache").join("debian-13.qcow2"));
        assert_eq!(p.build_meta("debian-13"), vm.join("build-cache").join("debian-13.json"));
        assert_eq!(p.build_lock("debian-13"), vm.join("build-cache").join("debian-13.lock"));
        assert_eq!(p.build_purge("debian-13"), vm.join("build-cache").join("debian-13.purge"));
        assert_eq!(p.downloads(), vm.join("cache"));
        assert_eq!(p.overlays(), vm.join("overlays"));
        assert_eq!(p.build_state("demo"), vm.join("build-state").join("demo.json"));
        assert_eq!(p.build_home("demo"), vm.join("build-state").join("demo").join("home"));
        assert_eq!(p.guest("debian-13"), vm.join("guest").join("debian-13"));
        assert_eq!(p.engines(), vm.join("qemu"));
        assert_eq!(p.ledger(), vm.join("state").join("deps.json"));
        assert_eq!(p.storage_state(), vm.join("storage.json"));
        assert_eq!(p.storage_lock(), vm.join("storage.lock"));
    }

    /// 目的：名字里的分隔符不会把落点带出 `.vm`，同一个虚拟根的各落点 stem 一致。
    #[test]
    fn unsafe_segments_collapse_into_one_stem() {
        let p = Paths::at(Path::new("/w"));
        assert_eq!(p.build_volume("a/b"), p.build_volume("a_b"));
        assert!(p.guest("../x").starts_with(layout::vm(Path::new("/w")).join("guest")));
        assert!(!p.build_volume("a/b").to_string_lossy().contains("a/b"));
    }
}
