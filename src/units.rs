//! 目的：业务单元清单——按业务功能垂直切分的一等公民模块声明。
//! 管：各单元的模块根声明；环 1 的纯单元统一挂在 `pure` 下（路径编码硬不变量），其余平铺。
//! 不管：单元内部实现与跨单元契约（在各单元 api / domain）；分环由 layers.yaml 认定（零豁免：实际违规即失败）。
//! 联动：src/main.rs 组合根装配这些单元；runtime 与其它单元平级、只经各单元 api 编排（docs/architecture/boundaries.md）。

pub mod catalog;
pub mod conversation;
pub mod lifecycle;
pub mod llm;
pub mod maintenance;
pub mod prompt;
pub mod provision;
pub mod pure;
pub mod residents;
pub mod runtime;
pub mod system;
pub mod tools;
pub mod transcript;
pub mod turn;
pub mod vm;
pub mod work;
pub mod workspace;
