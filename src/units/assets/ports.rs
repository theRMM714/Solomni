//! 目的：随包只读内容的出站端口——只由本单元的 `service` 持有（R12）。
//! 管：按路径读全文、按目录递归收 `.yaml`。
//! 不管：介质名、排序与文案（在 `domain` / `service`）。
//! 联动：实现者是 `detail::fs_assets::FsAssetFs`；契约见 docs/assets/README.md。

use std::path::{Path, PathBuf};

/// 目的：随包只读内容的文件系统端口。
pub trait AssetFiles: Send + Sync {
    /// 目的：读一份文件的全文。
    /// 参数：`path` = 要读的文件。
    /// 返回：全文。
    /// 错误：读不了时给出原因（原样上抛，由 `service` 补文案）。
    fn read(&self, path: &Path) -> Result<String, String>;

    /// 目的：递归收集目录下的 `.yaml`（不跟符号链接）。
    /// 参数：`dir` = 起点目录。
    /// 返回：收到的文件路径（顺序不保证，排序由 `service` 做）。
    /// 错误：目录读不了时给出原因。
    fn yaml_tree(&self, dir: &Path) -> Result<Vec<PathBuf>, String>;
}
