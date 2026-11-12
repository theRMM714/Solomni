//! 目的：业务能力清单——按业务功能垂直切分的一等公民模块声明。
//! 管：各能力模块根的声明；每个能力自带 api / ports / domain / detail 四分区。
//! 不管：能力内部实现与跨能力契约（在各能力 api / domain）；依赖方向由 T0 结构审查判定（零豁免：实际违规即失败）。
//! 联动：src/main.rs 组合根装配这些能力；conductor 与其它能力平级、只经各能力 api 编排（ARCHITECTURE.md §九）。

pub mod builds;
pub mod collab;
pub mod conductor;
pub mod llm;
pub mod modules;
pub mod packages;
pub mod permission;
pub mod prompt;
pub mod provision;
pub mod registry;
pub mod residents;
pub mod secrets;
pub mod session;
pub mod slate;
pub mod storage;
pub mod taskchain;
pub mod tools;
pub mod vm;
pub mod workspace;
