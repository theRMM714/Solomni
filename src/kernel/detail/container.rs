//! 目的：容器身份与容器内启动——按一次围栏的 reach 派生容器名与 SID，建 profile，再在容器里拉起进程。
//! 管：AppContainer profile 与 SID、产品能力 SID、internetClient 能力、UTF-16 转换、system shell、kill-on-close job 对象、容器内启动。
//! 不管：授权与撤销（在 `acl`）、台账（在 `ledger`）、环境与程序名改写（在 `launch`）、策略（在 `confine::windows::prepare_fence`）。
//! 联动：由 `confine` 的 Windows 后端在准备与执行时调用；容器名前缀也是 `--fence-clean` 认自己建过哪些 profile 的判据。

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::*;
