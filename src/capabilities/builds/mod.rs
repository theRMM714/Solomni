//! 目的：构建能力——按模块声明的 `build:` 在目标环境把产物编出来（本机档在宿主，虚拟机档在虚拟根里）。
//! 管：一次"确保产物可用"的用例（入口在场即跳过、缺失才编、编完复验）、构建期能力选包、源指纹缓存与 `.vm/build-state/` 落点。
//! 不管：模块清单与契约（`modules`）、包库与选型（`packages`）、虚拟机引擎与档位（`vm`）、补齐清单（`provision`）、CLI 文案（`diagnostics`）。
//! 联动：`conductor` 在创建会话前调它补编；`diagnostics --build` 是它的薄壳；契约见 docs/builds/README.md。

pub mod api;
pub mod detail;
pub mod service;
