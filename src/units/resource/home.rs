//! 目的：**登记处资源**——`data/.home` 这块盘的唯一主人：文档名、读、原子写、私密权限。
//! 管：六份登记处文档（providers / models / settings / agents / secrets / modules）的读写；缺文件 = 无；写原子（临时文件 + rename）。
//! 不管：yaml 形态与校验（在 `catalog` 的 domain）；哪些业务在用哪份文档（在 catalog 的 service）；
//!   目录路径怎么拼（在 `kernel::domain::layout`，组合根给了绝对目录）。
//! 联动：catalog 的三个 yaml 适配器经 `api::Home` 读写；装配见 `src/entry/assembly.rs`。
//!   判据与资源清单见 docs/architecture/state.md「两个根」与 docs/home/README.md。

pub mod api;
pub mod detail;
pub mod domain;
pub mod ports;
pub mod service;
