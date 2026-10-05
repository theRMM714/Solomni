//! 纯逻辑：清单契约与校验、执行计划派生、沙箱寻址与越界判定。没有 IO，也不加 trait。

pub mod exec;
pub mod hash;
pub mod module;
pub mod packages;
pub mod workspace;
pub mod workstore;
