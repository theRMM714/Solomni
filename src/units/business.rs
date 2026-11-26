//! 目的：业务单元的角色桶——有副作用、有状态或编排用例的单元住这里，是 `src/units/` 下数量最多的一桶。
//! 管：二十一个业务单元的模块根声明；桶是角色不是环号，环仍由 layers.yaml 决定（本桶同时住环 2 与环 3）。
//! 不管：单元内部实现与跨单元契约（在各单元 api / domain）；盘面归属（resource 桶 + tests/gates/structure/disk-owners.js）；纯逻辑（pure 桶）。
//! 联动：src/units.rs 声明本桶；docs/architecture/layers.md 记三角色桶口径；单元名判据一律走 tests/gates/structure/unitpath.js。
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
pub mod residents;
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
