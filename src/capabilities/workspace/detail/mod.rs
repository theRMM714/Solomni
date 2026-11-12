//! 目的：工作区能力的机制实现（本能力自己的适配器，只能由入口层的组合根构造）。
//! 管：`fs_workspace`（目录布局与沙箱寻址的落盘）与 `fs_workstore`（共享区版本库与内容寻址存储）。
//! 不管：端口定义（在 `ports.rs`）；纯规则（在 `domain/`）；装配（在入口层组合根）。
//! 联动：由 `main.rs` 构造并作为 `Workdirs` / `WorkStore` 端口注入 `service.rs`。

pub mod fs_workspace;
pub mod fs_workstore;

pub use fs_workspace::FsWorkspace;
pub use fs_workstore::FsWorkStore;
