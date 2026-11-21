//! 目的：命令队列的对外面——一条一条来，按序执行、可取消。
//! 管：`Actor`。
//! 不管：队列落在哪个线程、怎么等（在 `domain/actor`）。
//! 联动：能力侧经本面取；内核内部直接取 `domain::actor`。

pub use crate::kernel::domain::actor::Actor;
