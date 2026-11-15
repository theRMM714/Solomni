//! 目的：turn 的入站面——会话本体 `Turn`、五张内存表的类型、装配构件、树序 / 事实 / 计划与工作区绑定的对外重导出。
//! 管：把 `Turn` / `TurnState` / `Session`、`domain` 的纯规则、`service` 各分区的装配构件与取事实函数按 `api` 契约
//!   暴露给唯一调用方 `conductor`。
//! 不管：状态的持有与访问器实现（在 service/state.rs）、装配与重建实现（在 service/*.rs）；不碰 IO、不碰用例编排。
//! 联动：`conductor` 的 service 各文件经 `crate::capabilities::turn::api` 取类型与装配函数。

#[allow(unused_imports)] // 入站面：conductor 与测试都经这里取纯规则
pub use crate::capabilities::turn::domain::assemble::{face_tools, service_ops_of, serviced_tools};
pub use crate::capabilities::turn::domain::session::{flat_modules, session_started};
pub use crate::capabilities::turn::service::assemble::{
    HandlerSource, MemberSpec, SingleSpec, ToolHandlers, member_tools, single_session,
};
pub use crate::capabilities::turn::service::proxy::PROXY_PRIVATE_DIR;
#[allow(unused_imports)] // 入站面：转录回放的纯函数由单元测试直接调用（与真实重建同一份实现）
pub use crate::capabilities::turn::service::rebuild::replay_dialogue;
pub use crate::capabilities::turn::service::session::Turn;
#[allow(unused_imports)] // 入站面：状态容器类型由单元测试直接构造（生产只经 Turn 读写）
pub use crate::capabilities::turn::service::state::{Session, TurnState};
