//! 目的：随包只读内容的适配器——把出站端口落到本机文件系统。
//! 管：`FsAssetFs` 的导出。
//! 不管：落点、文案与排序（在 `domain` / `service`）。
//! 联动：只由组合根构造，注入 `service::AssetsService`；契约见 docs/assets/README.md。

pub mod fs_assets;

pub use fs_assets::FsAssetFs;
