//! 目的：登记处的出站端口——文件机制本身（只有 service.rs 持有，R12）。
//! 管：`HomeFiles` 的形状：按文件名读 / 写，私密标记是事实不是策略。
//! 不管：文档清单（在 domain）；原子的实现细节（在 detail/fs_home.rs）。
//! 联动：实现见 detail/fs_home.rs；由 service.rs 消费。

/// 目的：一块登记处目录的文件面（读 / 写一份文本）。
pub trait HomeFiles: Send + Sync {
    /// 目的：读一份文件；不存在 = `None`。
    fn read(&self, name: &str) -> Result<Option<String>, String>;

    /// 目的：整份写回一份文件（原子），`private` 为真时收紧权限。
    fn write(&self, name: &str, text: &str, private: bool) -> Result<(), String>;
}
