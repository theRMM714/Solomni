//! 目的：共享区版本化工作流测试入口：按用例族分到子模块。
//! 管：读门禁与判定、提交与历史、状态与视图、传送、重置、冲突与同步。
//! 不管：夹具（`support.rs`）、工作区寻址（`resolve.rs`）。
//! 联动：`docs/storage/README.md`。

mod commit;
mod gate;
mod push;
mod reset;
mod status;
mod sync;
