//! 目的：内容盘的落点派生——模块私有区与运行包。
//! 管：`data/modules` 根、模块 id → 私有区、私有区里的 `userdata/` 与 `build/`、`data/runtimes`。
//! 不管：这些落点上发生的事（模块清单在 `catalog`、构建产物在 `builds`、回收在 `storage`）。
//! 联动：`content::api::paths` 交给组合根与各 detail 适配器；见 docs/content/README.md。

use crate::kernel::domain::layout;
use std::path::{Path, PathBuf};

/// 目的：内容盘的落点派生器——只持产品安装根，派生结果每次重算（不缓存、不落盘）。
pub struct ContentPaths {
    install: PathBuf,
}

/// 目的：一个模块的私有区——源码在 `modules/<id>`，用户数据与构建产物在这里。
pub struct ModuleArea {
    root: PathBuf,
}

impl ContentPaths {
    /// 目的：按产品安装根造一份派生器。
    /// 参数：`install` 产品安装根（`data/` 在它下面）。
    /// 返回：派生器本身；不碰磁盘。
    pub fn at(install: &Path) -> ContentPaths {
        ContentPaths {
            install: install.to_path_buf(),
        }
    }

    /// 目的：模块私有区的根目录（`data/modules`）。
    /// 返回：`data/modules`。
    pub fn modules_data(&self) -> PathBuf {
        layout::modules_data(&self.install)
    }

    /// 目的：某个模块的私有区。
    /// 参数：`id` 模块 id（合法 id 已由 `catalog` 校验；这里再造一次路径段安全化，只影响落点不改 id）。
    /// 返回：私有区（`data/modules/<id>`）。
    pub fn area(&self, id: &str) -> ModuleArea {
        ModuleArea {
            root: self.modules_data().join(safe_segment(id)),
        }
    }

    /// 目的：运行包目录（用户投放口）。
    /// 返回：`data/runtimes`。
    pub fn runtimes(&self) -> PathBuf {
        layout::runtimes(&self.install)
    }
}

impl ModuleArea {
    /// 目的：模块的跨任务私有数据（模块进程的 HOME 与唯一可写落点）。
    /// 返回：`data/modules/<id>/userdata`。
    pub fn userdata(&self) -> PathBuf {
        self.root.join("userdata")
    }

    /// 目的：模块的构建产物区（模块自己的构建脚本往里写，不进源码树）。
    /// 返回：`data/modules/<id>/build`。
    pub fn build(&self) -> PathBuf {
        self.root.join("build")
    }
}

/// 目的：把模块 id 变成单层安全的路径段（防穿越、防跳目录）。
/// 参数：`s` 原始 id。
/// 返回：只含 ascii 字母数字与 `-_` 的单段；其余字符替 `_`。
/// 约束：合法 id 本来就只有安全字符，这里只兜底，不改变 `catalog` 的 id 契约。
fn safe_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else if c != '.' {
            out.push('_');
        }
    }
    if out.is_empty() { "_".to_string() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_landing_sits_under_data() {
        let p = ContentPaths::at(Path::new("product"));
        assert_eq!(
            p.modules_data(),
            Path::new("product").join("data").join("modules")
        );
        assert_eq!(
            p.runtimes(),
            Path::new("product").join("data").join("runtimes")
        );
        assert_eq!(
            p.area("harvest").userdata(),
            Path::new("product")
                .join("data")
                .join("modules")
                .join("harvest")
                .join("userdata")
        );
        assert_eq!(
            p.area("harvest").build(),
            Path::new("product")
                .join("data")
                .join("modules")
                .join("harvest")
                .join("build")
        );
    }

    #[test]
    fn unsafe_ids_stay_one_segment() {
        let p = ContentPaths::at(Path::new("product"));
        assert_eq!(p.area("a/b").userdata(), p.area("a_b").userdata());
        assert!(p.area("../x").userdata().starts_with(p.modules_data()));
        assert_eq!(
            p.area("..").userdata(),
            p.modules_data().join("_").join("userdata")
        );
    }
}
