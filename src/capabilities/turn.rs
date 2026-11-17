//! 目的：turn 单元——一轮对话的编排与环境事实。
//! 管：turn 子业务的回合编排与阶段推进、env 子业务的工作目录 / 凭据 / 运行时等环境事实。
//! 不管：会话转录（conversation）、工具执行与虚拟机（tools / vm）、构建产物（maintenance）。
//! 联动：单元内子业务各自保留四分区，跨单元引用经 turn::<子业务>::api（见 docs/turn/README.md、docs/env/README.md）。
pub mod env;
pub mod turn;
