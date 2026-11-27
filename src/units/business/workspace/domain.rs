//! 目的：工作区的纯逻辑——沙箱寻址与越界判定、共享区路径归属、过滤名单、沙箱清单。
//! 管：`path_filter` / `scope` / `seats`（沙箱清单的唯一构造点）/ `workspace`（沙箱与寻址视图）四个子模块。
//! 不管：版本化规则（在 `versions` 单元）；IO 与适配（在 `detail/`）；端口定义（在 `ports.rs`）。
//! 联动：由本单元 `service.rs` 消费；`api.rs` 对外重导出其中的共享类型。

pub mod path_filter;
pub mod scope;
pub mod seats;
pub mod workspace;
