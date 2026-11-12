//! 目的：挂载仅 Windows 可跑的探针。
//! 管：container 子模块。
//! 不管：其它平台。
//! 联动：tests/windows/。

mod container;
mod ledger;
mod startup;

#[path = "../../helpers/https_probe.rs"]
mod https;
