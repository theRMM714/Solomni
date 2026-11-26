//! 目的：业务单元清单——按业务功能垂直切分的一等公民模块声明。
//! 管：各单元的模块根声明；角色桶收同类单元——环 1 的纯单元挂在 `pure` 下、盘主单元挂在 `resource` 下（路径编码角色，不编码环号），其余平铺。
//! 不管：单元内部实现与跨单元契约（在各单元 api / domain）；分环由 layers.yaml 认定（零豁免：实际违规即失败）。
//! 联动：src/main.rs 组合根装配这些单元；runtime 与其它单元平级、只经各单元 api 编排（docs/architecture/boundaries.md）。

pub mod agentloop;
pub mod catalog;
pub mod collab;
pub mod conversation;
pub mod lifecycle;
pub mod llm;
pub mod maintenance;
pub mod prompt;
pub mod provision;
pub mod proxy;
pub mod pure;
pub mod residents;
pub mod resource;
pub mod runtime;
pub mod session_model;
pub mod system;
pub mod tools;
pub mod transcript;
pub mod turn;
pub mod turn_state;
pub mod vm;
pub mod work;
pub mod workspace;
