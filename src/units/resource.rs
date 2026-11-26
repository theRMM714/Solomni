//! 目的：盘主单元的角色桶——每个单元管一块盘，盘面经 `kernel::domain::layout` 的访问器取。
//! 管：五个盘主单元（assets / content / home / sessions / vmstore）的模块根声明；桶是角色不是环号，环仍由 layers.yaml 决定。
//! 不管：单元内部实现与跨单元契约（在各单元 api / domain）；盘面归属与允许调用点（tests/gates/structure/disk-owners.js）。
//! 联动：src/units.rs 声明本桶；docs/architecture/io.md 记哪块盘归谁；单元名判据一律走 tests/gates/structure/unitpath.js。

pub mod assets;
pub mod content;
pub mod home;
pub mod sessions;
pub mod vmstore;
