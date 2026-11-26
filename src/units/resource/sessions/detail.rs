//! 目的：会话盘的机制实现（只有组合根能构造）。
//! 管：目录适配器 `FsSessionDirs` 的模块声明。
//! 不管：会话根的拼法（在 service）；会话目录内部布局（在消费方 detail）。
//! 联动：由 `src/entry/assembly.rs` 构造，经 `ports::SessionDirs` 交给 service。

pub mod fs_dirs;

pub use fs_dirs::FsSessionDirs;
