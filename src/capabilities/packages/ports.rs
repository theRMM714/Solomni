//! 目的：packages 能力的出站端口。
//! 管：PackageSource（扫描 runtimes/ 包清单）。
//! 不管：清单校验与冲突预检（domain/）、目录遍历（detail/）。
//! 联动：由 service.rs 持有（R12）；实现是 detail/fs_packages.rs。

use crate::capabilities::packages::domain::manifest::Library;

/// 目的：运行包库来源端口——扫描依赖文件夹（runtimes/）里的包清单。
/// 约束：「清单即事实」，每次调用重扫，放入即出现；清单校验、去重与冲突预检在本能力（Library::build），
///   目录遍历与 yaml 解析在适配层。只有 service.rs 持有它（R12）。
pub trait PackageSource {
    fn scan(&self) -> Library;
    /// 包库所在目录（配置界面要把"把包放哪儿"如实告诉用户）。
    fn dir(&self) -> std::path::PathBuf;
}
