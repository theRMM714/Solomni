//! 目的：模块能力——模块公地的发现、`module.yaml` 契约与启停状态。
//! 管：扫描 `modules/`、清单契约与校验、模块清单（`Roster`）、清单文本、模块与服务的启停状态。
//! 不管：工具面与参数渲染（`tools` / `session`）、常驻服务运行态（`residents`）、隐秘字段的值（`secrets`）、运行包库与执行计划（`workspace`）、身份块渲染（`session`）。
//! 联动：清单来源端口见 `ports.rs`，实现见 `detail/fs_modules.rs`；消费方是 `workspace` / `residents` / `secrets` / `registry` / `conductor` 等（见 docs/modules/README.md）。
pub mod api;
pub mod detail;
pub mod domain;
pub mod ports;
pub mod service;
