//! 目的：conversation 单元——一段对话的转录落盘与草稿。
//! 管：session 的转录与回合落盘、slate 的草稿。
//! 不管：多人协作与裁决（collab）、模型网关转发（proxy）、一轮对话的环境事实（turn）、工具面（tools）、登记处（catalog）。
//! 联动：单元内子业务各自保留四分区，跨单元引用经 conversation::<子业务>::api（见 docs/session/README.md、docs/slate/README.md）。
pub mod session;
pub mod slate;
