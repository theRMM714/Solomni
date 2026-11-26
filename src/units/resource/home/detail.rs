//! 目的：登记处的机制实现（只有组合根能构造）。
//! 管：文件系统适配器 `FsHome` 的模块声明。
//! 不管：文档清单（在 domain）与用例（在 service）。
//! 联动：由 `src/entry/assembly.rs` 构造，经 `ports::HomeFiles` 交给 service。

pub mod fs_home;

pub use fs_home::FsHome;
