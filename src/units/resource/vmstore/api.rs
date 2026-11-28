//! 目的：虚拟机盘的入站面——别的单元只准从这里取 `.vm` 下的落点。
//! 管：`paths` / `resolve_guest` 两个入口与 `Paths` 的类型再导出。
//! 不管：落点的实际读写（各单元 detail）；根名常量（`kernel` 的 `layout`）。
//! 联动：`builds` / `provision` / `storage` / `vm` 的 detail 与组合根都用它。

use std::path::Path;

pub use crate::units::resource::vmstore::domain::paths::Paths;

/// 目的：产品根 → `data/.vm` 的布局。
/// 参数：`root` = 产品安装根。
/// 返回：可反复派生的 `Paths`（只持一个根，不碰磁盘）。
pub fn paths(root: &Path) -> Paths {
    Paths::at(root)
}

/// 目的：虚拟根名 → 那一格目录的绝对路径（`data/.vm/guest/<名>`）。
/// 参数：`root` = 产品安装根；`guest` = 清单里的一个名字。
/// 返回：那一格目录的路径（不碰磁盘，在不在场由调用方查）。
/// 错误：名字空或带路径成分时如实拒收——它来自清单，也是会话里存的值。
pub fn resolve_guest(root: &Path, guest: &str) -> Result<std::path::PathBuf, String> {
    let name = guest.trim();
    if name.is_empty() {
        return Err("没有指定虚拟根".to_string());
    }
    if name.contains('/') || name.contains('\\') || name == "." || name == ".." {
        return Err(format!(
            "虚拟根名不合法：{}（应是清单里的一个名字，不是路径）",
            guest
        ));
    }
    Ok(paths(root).guest(name))
}
