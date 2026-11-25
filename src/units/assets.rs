//! 目的：随包只读内容（`prompts/` 与 `systools/`）的落点与读取——资源单元，环 2。
//! 管：这两个目录的介质名、文档清单、按文档读文本、提示词册的目录枚举。
//! 不管：文档形态与解码（各业务单元的 detail）；安装根的其他落点（`bin/` 归 `layout`，`modules/` 归 `content`）。
//! 联动：`prompt`（提示词册）、`tools`（工具总表与保留名）、`conversation`（会话类型表）经本单元 `api` 读；契约见 docs/assets/README.md。

pub mod api;
pub mod detail;
pub mod domain;
pub mod ports;
pub mod service;
