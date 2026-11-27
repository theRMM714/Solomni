//! 目的：工作区单元的机制实现（本单元自己的适配器，只能由入口层的组合根构造）。
//! 管：`fs_workspace`（目录布局、沙箱寻址与文件落盘的落盘实现）与 `walk`（目录遍历）。
//! 不管：端口定义（在 `ports.rs`）；纯规则（在 `domain/`）；装配（在入口层组合根）。
//! 联动：由 `main.rs` 构造并作为 `Workdirs` 端口注入 `service.rs`。

pub mod fs_workspace;
pub mod walk;

pub use fs_workspace::FsWorkspace;
