//! 目的：纯层——环 1 的单元住在这里：无 IO、无状态、无端口，规则可穷举测试。
//! 管：环 1 的目录归属；路径就是「纯」这条硬不变量的编码（判据见 docs/architecture/layers.md）。
//! 不管：环号本身——环由 layers.yaml 认定，换环不搬家；src/units/ 的其余单元一律扁平。
//! 联动：门禁按 src/units/pure/ 双向对账环 1（tests/gates/structure/layers.js 的 shapeProblems 与 pureProblems）。

pub mod exec_model;
pub mod permission;
pub mod taskchain;
