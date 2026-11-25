//! 目的：把调用方持有的停止位包成只读端口——置位的仍是持有原子的那一方。
//! 管：`CancelFlag` 与唯一入口 `cancel_flag`。
//! 不管：停止位的创建与置位（在 `session` 与 `turn`）；适配器的轮询（在 `detail/`）。
//! 联动：本能力的 `api.rs` 转出 `cancel_flag`，适配器只读 `Cancelled`。

/// 目的：一次调用的停止位包装；只读，置位由原子持有者做。
struct CancelFlag(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl crate::units::llm::api::Cancelled for CancelFlag {
    fn cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// 目的：包一个停止位；同一个原子可以包多次，读到的是同一事实。
/// 参数：`flag` —— 调用方持有的停止位（停止时由它置位）。
/// 返回：只读端口；适配器轮询它关闭在途请求。
pub fn cancel_flag(
    flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> std::sync::Arc<dyn crate::units::llm::api::Cancelled> {
    std::sync::Arc::new(CancelFlag(flag))
}

/// 目的：借来的停止位（不复制原子，只在本次调用期间读它）。
struct CancelBorrow<'a>(&'a std::sync::atomic::AtomicBool);

impl crate::units::llm::api::Cancelled for CancelBorrow<'_> {
    fn cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// 目的：包一个**借来的**停止位：调用方自己持有原子，包装的生命期随请求选项。
/// 参数：`flag` —— 调用方持有的停止位。
/// 返回：只读端口；存活期不长于那个原子。
pub fn cancel_flag_ref(
    flag: &std::sync::atomic::AtomicBool,
) -> std::sync::Arc<dyn crate::units::llm::api::Cancelled + '_> {
    std::sync::Arc::new(CancelBorrow(flag))
}
