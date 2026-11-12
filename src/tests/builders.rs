//! 目的：测试脚手架（builders）——造会话、造名单、执行入口与脚本通道。
//! 管：各子模块按事件视图/执行入口/通道替身/进程 runner/会话装配分组；本模块对只有用例才用到的项放行 dead_code。
//! 不管：内存替身（`doubles/`）、用例本身（各业务测试文件）。
//! 联动：`src/tests.rs` 的 prelude；语义规范见 docs/testing/doubles.md。

#![allow(dead_code)]

use super::prelude::*;
use crate::kernel::api::RunOutcome;

mod chat;
mod events;
mod exec;
mod runners;
mod session;

pub(crate) use chat::*;
pub(crate) use events::*;
pub(crate) use exec::*;
pub(crate) use runners::*;
pub(crate) use session::*;
