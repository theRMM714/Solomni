//! 目的：机器可读的自检与探针——测试与 CI 的接口，不是用户功能；退出码按各模式的约定（多数恒 0：**判定归调用方**，这里只报事实）。
//! 管：隐藏模式的入口与事实输出（用户经文档知道它们，不列进面向用户的帮助）。
//! 不管：能力内部机制与装配（在各能力 detail 与 entry::assembly）。
//! 联动：由 `src/main.rs` 在对应分支调用；能力面装配统一在 `entry::assembly`。

pub mod build;
pub mod doctor;
pub mod fence;
pub mod gc;
pub mod https;
pub mod provision;
pub mod routes;
pub mod storage;
pub mod unpack;
pub mod work_gc;

pub use build::build;
pub use doctor::doctor;
pub use fence::{fence_verify, print_fence_env};
pub use gc::gc;
pub use https::https_check;
pub use provision::provision;
pub use routes::print_routes;
pub use storage::storage;
pub use unpack::unpack_cli;
pub use work_gc::work_gc;

/// 目的：打印一次回收报告（CLI 人类可读；不打印机器路径）。
pub(crate) fn print_gc(r: &crate::capabilities::storage::api::GcReport) {
    if r.actions.is_empty() {
        println!(
            "[存储] {}；当前 {} 字节",
            if r.applied {
                "没有可回收项"
            } else {
                "没有候选"
            },
            r.before_bytes
        );
        return;
    }
    for a in &r.actions {
        println!(
            "[存储] {} {}：{}（{}）{}",
            if a.applied { "回收" } else { "可回收" },
            a.category,
            a.label,
            a.bytes,
            a.error.clone().unwrap_or_else(|| a.reason.clone())
        );
    }
    println!("[存储] 释放 {} 字节；仍超限={}", r.freed_bytes, r.over_cap);
}
