//! 协调业务的**出站端口**：核心代理工具要做的外部动作（清单 / 建会话 / 转达 / 观察 / 生命周期）。
//!
//! 工具逻辑（`domain/proxy.rs`、`service/proxy.rs`）只依赖这一面，不认识会话机制；
//! 真实会话宿主尚未落地——当前只有测试替身实现它（见 src/capabilities/conductor/testgaps.yaml）。
#![allow(dead_code)] // 见 docs/testing/quality-isolation.md §三：契约已冻结，生产实现与调用点在下一步

use crate::capabilities::conductor::domain::proxy::{
    Catalog, CatalogScope, ControlAction, ControlState, Created, NewSession, ObserveView, Relayed,
    Snapshot,
};

/// 代理工具的宿主：把工具层的动作落到真实会话上。失败一律如实回报（不静默降级）。
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
    /// 控制一个会话的生命周期（pause / resume / stop / close），reason 进可回放记录。
    fn control(
        &self,
        session: &str,
        action: ControlAction,
        reason: &str,
    ) -> Result<ControlState, String>;
}
