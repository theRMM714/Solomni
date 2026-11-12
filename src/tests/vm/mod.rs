//! 目的：虚拟机档能力的用例测试——执行档位与计划、就绪检测与诊断、guest 装配。
//! 管：本能力的不变式（档位计划、入口判据、承载判定、挂载/叠写脚本）。
//! 不管：工作区与沙箱（`src/tests/workspace.rs`）、conductor 的编排。
//! 联动：契约见 docs/vm/README.md 与 RUNTIME_SPEC.md。

mod guestpath;
mod readiness;
mod engine;
mod entry;
mod guest;
mod share;
mod live;
mod concurrency;
mod fence;
mod diagnose;
