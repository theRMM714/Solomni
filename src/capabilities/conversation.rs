//! 目的：conversation 单元——会话转录、协作与模型网关这些围绕一段对话的事实。
//! 管：session 的转录与回合落盘、collab 的多人协作与裁决、proxy 的模型网关转发、slate 的草稿。
//! 不管：一轮对话的环境事实（turn）、工具面（tools）、登记处（catalog）、构建与存储回收（maintenance）。
//! 联动：单元内子业务各自保留四分区，跨单元引用经 conversation::<子业务>::api（见 docs/session/README.md、docs/collab/README.md、docs/proxy/README.md、docs/slate/README.md）。
pub mod collab;
pub mod proxy;
pub mod session;
pub mod slate;
