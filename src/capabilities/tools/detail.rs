//! 目的：工具能力的机制实现（本能力自己的适配器，只能由入口层的组合根构造）。
//! 管：`sys_io`（内置文件工具的读写机制）与 `yaml_systools`（`systools/` 三份 yaml 的装配输入）。
//! 不管：端口定义（在 `ports.rs`）；纯规则（在 `domain/`）；装配（在入口层组合根）。
//! 联动：由 `main.rs` 构造并作为 `SysIo` / `SystoolsSource` 端口注入 `service/`。

pub mod sys_io;
pub mod yaml_systools;

pub use sys_io::FsSysIo;
