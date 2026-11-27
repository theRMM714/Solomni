//! 目的：内容指纹的对外面——SHA-256 增量摘要与十六进制内容指纹。
//! 管：`Sha256`（分块喂入、`finish` / `finish_hex`）、`content_hash`（一次算完整份）、`is_content_hash`（名字判定）。
//! 不管：内容扫描、媒体类型与预览（在 `versions` 的 `asset`）；对象落盘（在 `versions/detail`）。
//! 联动：能力侧（版本化、工具的文件读写、运行包与素材指纹）只经本面取指纹；内核内部直接取 `domain::hash`。

pub use crate::kernel::domain::hash::{Sha256, content_hash, is_content_hash};
