//! 目的：常驻服务的机制实现（只有组合根能构造）。
//! 管：协议适配器实现（当前：MCP stdio）。
//! 不管：端口定义（在 ports）与生命周期 / 租约（在 service）。
//! 联动：由 service 经 ServiceAdapter 端口消费。

pub mod mcp;

pub use mcp::McpAdapter;
