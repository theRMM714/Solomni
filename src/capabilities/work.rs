//! 目的：工作与会话配置的用例——一场工作怎么被建立、改配、看、报，以及 guest 起停那几段慢活。
//! 管：建工作（单 / 协作 / 代理）、会话配置与改配、会话与文件视图、运行能力报告与档位、拟名单、guest 启动与销毁。
//! 不管：会话在世表与运行态（turn）、协作链推进与回档（runtime）、呈现层封装（presentation）。
//! 联动：runtime 把能力面与 `Turn` 传进来；本能力只向下依赖 turn / packages / provision / vm / workspace / session / env / kernel。

pub mod api;
pub(crate) mod domain;
pub(crate) mod service;
