//! 目的：入口层共用的机制——只给程序入口用（`src/main.rs` / `src/diagnostics/` / `src/guard/`）。
//! 管：产品根规范化（`root`）这一件事。
//! 不管：任何业务语义；组合根的装配（那在各入口自己那里）。
//! 联动：消费者是 `src/main.rs`（解析产品根）。

pub mod root;
