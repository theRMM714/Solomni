//! 目的：把调用方持有的停止位包成只读端口——置位的仍是持有原子的那一方。
//! 管：`CancelFlag` 与唯一入口 `cancel_flag`。
//! 不管：停止位的创建与置位（在 `session` 与 `turn`）；适配器的轮询（在 `detail/`）。
//! 联动：本能力的 `api.rs` 转出 `cancel_flag`，适配器只读 `Cancelled`。

/// 目的：一次调用的停止位包装；只读，置位由原子持有者做。
struct CancelFlag(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl crate::capabilities::llm::api::Cancelled for CancelFlag {
    fn cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// 目的：包一个停止位；同一个原子可以包多次，读到的是同一事实。
/// 参数：`flag` —— 调用方持有的停止位（停止时由它置位）。
/// 返回：只读端口；适配器轮询它关闭在途请求。
pub fn cancel_flag(
    flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> std::sync::Arc<dyn crate::capabilities::llm::api::Cancelled> {
    std::sync::Arc::new(CancelFlag(flag))
}
