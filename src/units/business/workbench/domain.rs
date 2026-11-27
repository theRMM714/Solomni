//! 目的：工作台契约的纯形状——写入门禁的口径。
//! 管：`gate`（版本身份、需求、拒绝原因与门禁端口）、`readfact`（一次读的读取事实）。
//! 不管：实现与 IO；布局与版本库（在 `workspace` 与 `versions`）。
//! 联动：由本单元的 `api` 重导出。

pub mod gate;
pub mod readfact;
