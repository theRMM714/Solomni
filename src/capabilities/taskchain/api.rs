//! 入站能力面：**其它能力与呈现层只准用这里**（不许碰 `domain`）。
//!
//! 纯领域业务：这里导出**任务链的事实与派生**（值对象 + 图算法），没有端口、没有用例编排。

pub use crate::capabilities::taskchain::domain::chain::{
    Acceptance, NodeStatus, TaskChain, TaskNode,
};
