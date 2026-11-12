//! 目的：provision 的单元测试入口——按被测面把用例分到三个子模块。
//! 管：子模块声明与下游入口。
//! 不管：具体用例（分别在子模块里）。
//! 联动：被测实现是 `capabilities::provision`；夹具用 `crate::tests::scratch` 铺临时目录。

mod guest;
mod image;
mod ports;
mod service;
