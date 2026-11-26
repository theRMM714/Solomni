//! 目的：业务单元清单——按业务功能垂直切分的一等公民模块声明。
//! 管：三个角色桶的模块根声明——环 1 的纯单元挂 `pure`、盘主单元挂 `resource`、其余业务单元挂 `business`（路径编码角色，不编码环号）。
//! 不管：单元内部实现与跨单元契约（在各单元 api / domain）；分环由 layers.yaml 认定（零豁免：实际违规即失败）。
//! 联动：src/main.rs 组合根装配这些单元；runtime 与其它单元平级、只经各单元 api 编排（docs/architecture/boundaries.md）。

pub mod business;
pub mod pure;
pub mod resource;
