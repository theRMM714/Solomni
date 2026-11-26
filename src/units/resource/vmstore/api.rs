//! 目的：虚拟机盘的入站面——别的单元只准从这里取 `.vm` 下的落点。
//! 管：`paths` 一个入口与 `Paths` 的类型再导出。
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
