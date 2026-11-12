//! 目的：taskchain 的入站能力面——任务链的事实与派生（值对象 + 图算法）。
//! 管：Acceptance / NodeStatus / TaskChain / TaskNode 的再导出。
//! 不管：domain 内部规则实现与装配；端口与用例编排（本能力没有）。
//! 联动：collab 驱动、session 线格式携带、呈现层渲染都经这里（docs/collab/task-chain.md）。

pub use crate::capabilities::taskchain::domain::chain::{
    Acceptance, NodeStatus, TaskChain, TaskNode,
};
