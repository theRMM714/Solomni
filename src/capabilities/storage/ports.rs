//! 目的：storage 能力的出站端口。
//! 管：SpaceSurvey（磁盘事实）、SpaceReclaim（锁、删除、排队、压缩、回收摘要）、GuestLiveness（guest 存活）。
//! 不管：什么该删、按什么顺序删（在 `domain`）；目录怎么扫、文件怎么删（在 `detail/`）。
//! 联动：由 `service.rs` 持有（R12）；实现是 `detail/survey.rs`、`detail/reclaim.rs`、`detail/guests.rs`。

use crate::capabilities::storage::api::{LastGc, SpaceCategory};

/// 目的：一次盘点的原始事实——不含预算、作业与策略（那是 service 的口径）。
pub struct SpaceFacts {
    /// 目的：这一趟盘点取的 unix 秒；按龄判定与回收摘要都用它。
    pub generated_at: u64,
    /// 目的：宿主磁盘总量（取不到为 0）。
    pub total_bytes: u64,
    /// 目的：宿主磁盘可用量（取不到为 0）。
    pub free_bytes: u64,
    pub categories: Vec<SpaceCategory>,
}

/// 目的：空间盘点端口——把产品根下的落点读成分类事实。
/// 约束：只读；读不到的项按空或 0 计，不阻断报告；只由 `service.rs` 持有（R12）。
pub trait SpaceSurvey: Send + Sync {
    /// 目的：盘点当前空间。
    /// 参数：`full` = 是否连环境组件、会话与模块产物一起扫。
    fn survey(&self, full: bool) -> SpaceFacts;
}

/// 目的：一次回收的独占凭据——持有期间别的回收进不来，drop 时释放。
pub trait ReclaimLease: Send {}

/// 目的：空间回收端口——回收锁、删除、排队清理、压缩与上次回收的落盘。
/// 约束：删构建卷前自己判构建锁；只由 `service.rs` 持有（R12）。
pub trait SpaceReclaim: Send + Sync {
    /// 目的：独占这一趟回收。
    /// 返回：drop 即释放的凭据；已有另一个回收在跑给 None。
    /// 错误：建目录或写锁失败。
    fn lock(&self) -> Result<Option<Box<dyn ReclaimLease>>, String>;

    /// 目的：删掉一项可回收数据。
    /// 参数：`category` = 分类 id（CAT_*）；`id` = 项 id。
    /// 错误：构建在跑、名字不安全、删除失败。
    fn remove(&self, category: &str, id: &str) -> Result<(), String>;

    /// 目的：排队在下次构建前清掉某个模块的工作区。
    /// 参数：`guest` = 虚拟根名；`module` = 模块 id。
    /// 错误：构建在跑、名字不安全、写标记失败。
    fn queue_module_purge(&self, guest: &str, module: &str) -> Result<(), String>;

    /// 目的：压缩一块构建缓存卷。
    /// 参数：`guest` = 虚拟根名。
    /// 返回：压缩前与压缩后的实际占用。
    /// 错误：构建在跑、没有 qemu-img、卷不存在、压缩失败。
    fn compact(&self, guest: &str) -> Result<(u64, u64), String>;

    /// 目的：上次回收的摘要；没有给 None。
    fn last_gc(&self) -> Option<LastGc>;

    /// 目的：记下这次回收的摘要。
    /// 错误：写不进状态文件。
    fn record_gc(&self, last: &LastGc) -> Result<(), String>;

    /// 目的：这个虚拟根是不是有构建在跑。
    /// 参数：`guest` = 虚拟根名。
    fn build_locked(&self, guest: &str) -> bool;
}

/// 目的：guest 存活端口——回收与还原前判"有没有 guest 在跑"。
pub trait GuestLiveness: Send + Sync {
    /// 目的：当前有没有活着的 guest 进程。
    fn any_running(&self) -> bool;
}
