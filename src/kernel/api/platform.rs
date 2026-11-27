//! 目的：宿主平台的对外面——这是哪个系统、可执行后缀、宿主路径相等与 ACL 步骤有无。
//! 管：`IS_WINDOWS` / `IS_MACOS` / `IS_LINUX` / `HOST_ARCH` / `EXE_SUFFIX` / `HOST_PATH_CASE_INSENSITIVE` / `HAS_EXTERNAL_ACL_STEP` / `host_path_eq`。
//! 不管：平台差异的具体动作（在各 `detail` 机制的平台子模块）。
//! 联动：单元侧经本面取；内核内部直接取 `domain::platform`。

pub use crate::kernel::domain::platform::{
    EXE_SUFFIX, HAS_EXTERNAL_ACL_STEP, HOST_ARCH, IS_LINUX, IS_MACOS, IS_WINDOWS,
};
// 暂无外部调用方：内核内部（路径桥、文件事务的锁键）直取 domain::platform，按出口纪律显式放行、不删。
#[allow(unused_imports)]
pub use crate::kernel::domain::platform::{HOST_PATH_CASE_INSENSITIVE, host_path_eq};
