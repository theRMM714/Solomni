//! 目的：工作台契约的纯形状——一席的工作面、写入门禁的口径与工作区过滤名单。
//! 管：`area`（寻址落点）、`bench`（一席的寻址语义）、`benches`（一次工作的全部席位）、
//!   `roots`（寻址根 / 文件清单 / 用量）、`filter`（`.workignore` 规则）、`gate`（版本身份与门禁端口）、
//!   `readfact`（一次读的读取事实）。
//! 不管：实现与 IO；布局与版本库（在 `workspace` 与 `versions`）。
//! 联动：由本单元的 `api` 重导出。

pub mod area;
pub mod bench;
pub mod benches;
pub mod filter;
pub mod gate;
pub mod readfact;
pub mod roots;
