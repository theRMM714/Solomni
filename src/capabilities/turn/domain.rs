//! 目的：turn 的 domain 分区入口——纯装配规则，无状态、无端口、无 IO。
//! 管：子模块声明；工具面发放与已就绪服务操作的归并都在 `assemble`。
//! 不管：取事实（权限面 / 沙箱 / 服务视图由调用方给）；会话对象怎么建（`service/assemble.rs`）。
//! 联动：`conductor` 装配成员工具面与会话时经 `api` 取用。

pub mod assemble;
