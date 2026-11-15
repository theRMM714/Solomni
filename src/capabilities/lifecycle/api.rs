//! 目的：lifecycle 能力的入站契约：历史列表 / 打开与删除收尾类型的唯一出口。
//! 管：重导出 `domain::history` 的纯派生、`service::history` 的读盘函数与 `service::delete` 的收尾类型。
//! 不管：实现（`service/`）、纯规则（`domain/`）、删前的内存动作与装配（`conductor`、组合根）。
//! 联动：`conductor` 与 `src/tests/session/` 只经本模块取用。

#[allow(unused_imports)] // 入站面：测试经这里取树序排列
pub use crate::capabilities::lifecycle::domain::history::{subtree_of, tree_order};
pub use crate::capabilities::lifecycle::service::delete::{
    FenceRelease, ReleaseByLease, SessionDelete,
};
pub use crate::capabilities::lifecycle::service::history::{list, open};
