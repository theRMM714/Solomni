//! 目的：turn 单元——一轮对话的编排与阶段推进。
//! 管：turn 子业务的回合编排、阶段推进、重建与代理会话装配。
//! 不管：会话转录（conversation）、工具执行与虚拟机（tools / vm）、工作台内部模型（workspace）、构建产物（maintenance）。
//! 联动：跨单元引用经 turn::turn::api（见 docs/turn/README.md）；一席的运行环境规则在 tools，工作台声明与沙箱清单在 workspace。
// 单元与同名子业务是两级归层结构（`turn/turn/`）：单元是归层单位，子业务是能力单位（见 ARCHITECTURE.md 分层表、R10），不是重复嵌套。
#[allow(clippy::module_inception)]
pub mod turn;
