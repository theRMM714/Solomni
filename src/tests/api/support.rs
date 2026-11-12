//! 目的：本族用例共用的装配——核心手柄与诊断替身。
//! 管：内存装配的核心手柄、只回答诊断的 vm 替身。
//! 不管：具体用例（同目录其它文件）。
//! 联动：`src/tests/builders.rs` 与 `src/tests/doubles.rs`。

use super::super::ops_with;
use crate::capabilities::conductor::api::ConductorHandle;
use crate::capabilities::modules::api::Module;

pub(crate) fn spawn(modules: Vec<Module>, core_script: Vec<&str>) -> ConductorHandle {
    ops_with(modules, core_script).0
}
