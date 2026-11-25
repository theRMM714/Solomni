//! 目的：随包只读内容的落点派生——介质名只在 `kernel` 的 `layout` 里出现，这里只按安装根拼子目录。
//! 管：`prompts/`、`systools/`、`locales/` 三个落点。
//! 不管：这些目录里有什么（`systools/` 的文档在 `doc`）；读盘（在 `detail`）。
//! 联动：消费方只经 `Assets` 拿文本，不拿绝对路径；契约见 docs/assets/README.md。

use crate::kernel::domain::layout;
use std::path::{Path, PathBuf};

/// 目的：随包只读内容的落点（产品安装根下的子目录）。
pub struct AssetPaths {
    install: PathBuf,
}

impl AssetPaths {
    /// 目的：按产品安装根建落点派生器。
    /// 参数：`install` = 产品安装根（随包只读内容所在的那一层）。
    /// 返回：只持安装根的派生器，不读盘。
    pub fn at(install: &Path) -> AssetPaths {
        AssetPaths {
            install: install.to_path_buf(),
        }
    }

    /// 目的：提示词册目录（`prompts/`）：递归收集的全部 `.yaml` 按路径排序合并成一册。
    /// 返回：`<安装根>/prompts`。
    pub fn prompts(&self) -> PathBuf {
        layout::prompts(&self.install)
    }

    /// 目的：机制与命令声明目录（`systools/`）。
    /// 返回：`<安装根>/systools`。
    pub fn systools(&self) -> PathBuf {
        layout::systools(&self.install)
    }

    /// 目的：语言目录（`locales/`）。
    /// 约束：运行期不读它——文案在编译期由 `kernel` 的 `include_str!` 编进来；这里只给落点，免得别处另拼。
    /// 返回：`<安装根>/locales`。
    #[allow(dead_code)]
    pub fn locales(&self) -> PathBuf {
        layout::locales(&self.install)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_landing_sits_under_install_root() {
        let p = AssetPaths::at(Path::new("product"));
        assert_eq!(p.prompts(), Path::new("product").join("prompts"));
        assert_eq!(p.systools(), Path::new("product").join("systools"));
        assert_eq!(p.locales(), Path::new("product").join("locales"));
    }
}
