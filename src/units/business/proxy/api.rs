//! 目的：proxy 的唯一入口——值类型、宿主契约（`ProxyHost`）与成员侧执行面（`ProxyHandler` / `ProxyTools`）。
//! 管：domain 的值类型与纯规则经这里转出；`ProxyHost` 的方法契约。
//! 不管：真实会话动作（宿主动手）；工具声明与席位面（`systools/` 与 `session::domain::mechanism`）。
//! 联动：runtime 只经这里取用；宿主实现在 runtime 的 `ProxyBridge`。

pub use crate::units::business::proxy::domain::proxy::*;
/// 目的：成员侧执行面——`ProxyHandler` 挂进代理会话那一回合的工具表，`ProxyTools` 由测试直接驱动（不在生产路径上按名取）。
#[allow(unused_imports)]
pub(crate) use crate::units::business::proxy::service::proxy::{ProxyHandler, ProxyTools};

/// 目的：代理工具的宿主——把工具层的动作落到真实会话上。失败一律如实回报（不静默降级）。
pub trait ProxyHost: Send + Sync {
    /// 只读清单事实（agent / 模块 / 模型）：只含公开信息，不含密钥与真实私有路径。
    fn catalog(&self, scope: CatalogScope) -> Result<Catalog, String>;
    /// 建一个代理会话；失败**不留半成品**（原子性由实现保证）。
    fn create_session(&self, spec: &NewSession) -> Result<Created, String>;
    /// 向一个目标会话转达一条消息；目标不存在 / 会话已关闭等如实报错。
    fn send(&self, target: &str, msg: &Relayed) -> Result<(), String>;
    /// 只读观察一个会话（不推进、不隐式启动下一轮）。
    fn observe(
        &self,
        session: &str,
        view: ObserveView,
        since: Option<&str>,
    ) -> Result<Snapshot, String>;
    /// 控制一个会话的生命周期（stop / continue / archive / restore），reason 进可回放记录。
    fn control(
        &self,
        session: &str,
        action: ControlAction,
        reason: &str,
    ) -> Result<ControlState, String>;
    /// 倒查一个会话的消息：`from` = 距最新多少条（0 = 最新一条），按新→旧取 `count` 条。**只读**。
    fn messages(&self, session: &str, from: usize, count: usize) -> Result<MessagesPage, String>;
}
