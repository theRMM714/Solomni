//! 目的：产品根的布局名字与派生——`.home`、`.vm`、`runtimes` 与 `guest.yaml` 只在这里拼。
//! 管：这几个目录名 / 文件名本身，以及从产品根、从 guest 目录派生出绝对路径。
//! 不管：名字下面的子目录归谁（`.home` 里是设置与台账、`.vm` 里是镜像与缓存，各有归属方）；谁在什么时机建它们。
//! 联动：组合根 `src/entry/assembly.rs`、存储回收、出包、围栏与进程簿都从这里取路径；改布局只改本文件与 `docs/storage/README.md`。

use std::path::{Path, PathBuf};

/// 目的：产品私有状态（设置、登记处、进程簿、锁）的目录名。
pub const HOME: &str = ".home";

/// 目的：虚拟机镜像、构建缓存与回收状态的目录名。
pub const VM: &str = ".vm";

/// 目的：运行包的目录名——产品根下唯一对用户可见的目录。
pub const RUNTIMES: &str = "runtimes";

/// 目的：一台 guest 的规格文件名，落在它自己的镜像目录里。
pub const GUEST_SPEC: &str = "guest.yaml";

/// 目的：产品私有状态目录的绝对路径。
/// 参数：`root` 产品根（安装目录与工作目录都从这里派生）。
/// 返回：`root/.home`。
pub fn home(root: &Path) -> PathBuf {
    root.join(HOME)
}

/// 目的：虚拟机与构建缓存目录的绝对路径。
/// 参数：`root` 产品根。
/// 返回：`root/.vm`。
pub fn vm(root: &Path) -> PathBuf {
    root.join(VM)
}

/// 目的：运行包目录的绝对路径。
/// 参数：`root` 产品根。
/// 返回：`root/runtimes`。
pub fn runtimes(root: &Path) -> PathBuf {
    root.join(RUNTIMES)
}

/// 目的：一台 guest 的规格文件路径。
/// 参数：`dir` 该 guest 的镜像目录。
/// 返回：`dir/guest.yaml`。
pub fn guest_spec(dir: &Path) -> PathBuf {
    dir.join(GUEST_SPEC)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_derive_under_the_product_root() {
        let root = Path::new("product");
        assert_eq!(home(root), Path::new("product").join(".home"));
        assert_eq!(vm(root), Path::new("product").join(".vm"));
        assert_eq!(runtimes(root), Path::new("product").join("runtimes"));
        assert_eq!(
            guest_spec(&home(root)),
            Path::new("product").join(".home").join("guest.yaml")
        );
        assert_ne!(home(root), vm(root), "私有状态与镜像不共用一个目录");
    }
}
