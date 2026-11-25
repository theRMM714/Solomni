//! 目的：内容盘资源单元——`data/modules`（模块私有区）与 `data/runtimes`（运行包）。
//! 管：模块 id → 私有区（`userdata/` / `build/`）的落点、私有区的确保存在、运行包根的落点。
//! 不管：模块清单与校验（在 `catalog`）；盘点的回收策略（在 `storage`）；安装/卸载的运行包行为（本期留缺口）。
//! 联动：入站面 `api`；出站端口在 `ports.rs`，只由 `service.rs` 持有；适配器在 `detail/`，只由组合根构造。

pub mod api;
pub mod domain;
pub mod ports;
pub mod service;

pub mod detail;
