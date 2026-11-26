//! 目的：随包只读内容的领域知识——落点派生与文档清单。
//! 管：`paths`（落点）与 `doc`（有哪些固定文档）。
//! 不管：读与枚举（在 `service` 与 `detail`）；文档内容如何解释（在消费方）。
//! 联动：由 `api` 重导出；契约见 docs/assets/README.md。

pub mod doc;
pub mod paths;
