//! 目的：内容盘的适配器集合（只由组合根构造）。
//! 管：把端口接到真实文件系统。
//! 不管：端口契约（在 `ports.rs`）、路径派生（在 `domain/paths.rs`）。
//! 联动：`fs_content.rs`。

pub mod fs_content;

pub use fs_content::FsContentFs;
