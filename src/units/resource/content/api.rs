//! 目的：内容盘的入站面——落点派生与私有区的确保存在。
//! 管：`ContentPaths` / `ModuleArea` 的落点；`Content` 的私有区与运行包根；安装/卸载的形状（本期留缺口）。
//! 不管：模块清单、校验、回收策略与安装实现（见各单元）。
//! 联动：组合根与小业务经它取路径；实现只在 `service.rs`。

pub use crate::units::resource::content::domain::paths::{ContentPaths, ModuleArea};
use std::path::{Path, PathBuf};

/// 目的：按产品安装根造一份落点派生器（纯函数，不碰磁盘）。
/// 参数：`install` 产品安装根。
/// 返回：派生器；`modules_data()`、`area(id)`、`runtimes()`。
pub fn paths(install: &Path) -> ContentPaths {
    ContentPaths::at(install)
}

/// 目的：一个已安装运行包的事实（安装/卸载期的清单条目）。
/// 约束：本期没有构造点——安装/卸载尚未实现，形状先定（缺口 `content-install` / `content-remove`）。
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// 目的：包 id（`package.yaml` 里的 id）。
    pub id: String,
    /// 目的：展示名。
    pub name: String,
    /// 目的：版本；清单没写时为 None。
    pub version: Option<String>,
    /// 目的：占用字节数。
    pub bytes: u64,
    /// 目的：文件数。
    pub files: u32,
}

/// 目的：安装前的检查结果（要被谁替换、有哪些阻塞问题）。
/// 约束：同 `Installed`——形状先定，实现留给缺口。
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPlan {
    /// 目的：这次要装的包 id。
    pub id: String,
    /// 目的：同 id 的既有版本（有则是覆盖安装）。
    pub replaces: Option<String>,
    /// 目的：阻塞安装的问题（空 = 可以装）。
    pub problems: Vec<String>,
}

/// 目的：内容盘的入站面：落点 + 私有区 + 运行包 + 安装/卸载形状。
/// 约束：不把"该不该删 / 能不能装"的判断收进来——只如实报能不能与为什么不能。
pub trait Content: Send + Sync {
    /// 目的：某个模块的私有区落点。
    /// 参数：`id` 模块 id。
    fn area(&self, id: &str) -> ModuleArea;

    /// 目的：确保某个模块的私有区与 `userdata/` 存在（幂等；产品里唯一的载入即写盘）。
    /// 参数：`id` 模块 id。
    /// 返回：`true` = 私有区可用（建成了或本来就在）；`false` = 建不出来（只读挂载 / 权限不足）。
    fn ensure_userdata(&self, id: &str) -> bool;

    /// 目的：运行包目录（用户投放口）。
    fn runtimes(&self) -> PathBuf;
}

/// 目的：运行包的安装 / 卸载面——**本期只有形状**，实现是缺口（`content-install` / `content-remove`）。
/// 约束：调用方拿到的必须是如实报错的 `Err`，不是空清单或静默成功。
#[allow(dead_code)]
pub trait ContentInstall: Send + Sync {
    /// 目的：列出已安装的运行包。
    /// 返回：本期未实现（缺口 `content-install`），如实报缺口而不是给空清单。
    fn installed(&self) -> Result<Vec<Installed>, String>;

    /// 目的：检查一个运行包来源能不能装。
    /// 参数：`source` 来源（本地目录或归档；形状本期不约束）。
    fn plan_install(&self, source: &str) -> Result<InstallPlan, String>;

    /// 目的：卸载一个运行包。
    /// 参数：`id` 包 id。
    fn remove(&self, id: &str) -> Result<(), String>;
}
