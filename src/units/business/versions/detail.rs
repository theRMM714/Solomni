//! 目的：版本库单元的机制实现（本单元自己的适配器，只能由入口层的组合根构造）。
//! 管：`fs_workstore`（共享区版本库与内容寻址存储）与 `walk`（目录遍历）。
//! 不管：端口定义（在 `ports.rs`）；纯规则（在 `domain/`）；装配（在入口层组合根）。
//! 联动：由组合根构造并作为 `WorkStore` 端口注入 `service.rs`。

pub mod fs_workstore;
pub mod walk;

pub use fs_workstore::FsWorkStore;
