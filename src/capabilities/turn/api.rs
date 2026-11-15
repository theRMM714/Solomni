//! 目的：turn 的入站面——`TurnState` / `Session` 与一席会话装配构件的对外重导出。
//! 管：把 state 的两个类型、domain/assemble 的纯规则、service/assemble 的装配构件按 `api` 契约暴露给唯一调用方 `conductor`。
//! 不管：状态的持有与访问器实现（在 service/state.rs）、装配实现（在 service/assemble.rs）；不碰 IO、不碰用例编排。
//! 联动：`conductor` 的 service 各文件经 `crate::capabilities::turn::api` 取类型与装配函数。

#[allow(unused_imports)] // 入站面：conductor 与测试都经这里取纯规则
pub use crate::capabilities::turn::domain::assemble::{face_tools, service_ops_of, serviced_tools};
pub use crate::capabilities::turn::service::assemble::{
    MemberSpec, SessionDeps, SingleSpec, member_tools, single_session,
};
pub use crate::capabilities::turn::service::state::{Session, TurnState};
