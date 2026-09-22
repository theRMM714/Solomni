//! 任务链：协作从讨论走到交付的那张图（**纯数据 + 纯函数**，不做 IO、不碰会话）。
//!
//! 契约见 docs/architecture/task-chain.md：
//! - 串并混合是**同一张依赖图的形状**，不是两种模式；
//! - `deps` 全部完成 = 就绪；空 = 立即就绪；
//! - 环、悬空依赖、重复 id、空目标、未知负责人都是**装配错误**，如实列出，不静默丢节点。

use serde::{Deserialize, Serialize};

/// 节点状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeStatus {
    /// 未开始（等依赖）。
    Pending,
    /// 子会话在跑。
    Running,
    /// 已完成（验收见 `acceptance`）。
    Done,
    /// 失败 / 超时（链不静默跳过：会暂停并通知用户）。
    Failed,
}

/// 一个任务节点的验收结论。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Acceptance {
    pub ok: bool,
    /// 为什么（通过或打回都要说清）。
    pub note: String,
}

/// 一个任务节点。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskNode {
    pub id: String,
    pub title: String,
    /// 该节点的**目标**（就是它的任务提示词）。用户可改；**改完不作废**——跑完由核心按当前值验收。
    pub objective: String,
    /// 负责的 agent 实例名（必须在名单里）。
    pub assignee: String,
    /// 依赖：这些节点都完成后本节点才就绪；空 = 立即就绪。
    #[serde(default)]
    pub deps: Vec<String>,
    #[serde(default = "pending")]
    pub status: NodeStatus,
    /// 它跑在哪个子会话里（未启动 = None）。
    #[serde(default)]
    pub sub_session: Option<String>,
    /// 验收结论（未验收 = None）。
    #[serde(default)]
    pub acceptance: Option<Acceptance>,
}

fn pending() -> NodeStatus {
    NodeStatus::Pending
}

/// 任务链（依赖图）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskChain {
    #[serde(default)]
    pub nodes: Vec<TaskNode>,
}

impl TaskChain {
    /// 现在可以启动的节点：自身待办 + **依赖全部完成**。
    /// 返回多个 = 它们可以**并发**跑；返回空 = 要么在跑、要么全结束、要么卡住（卡住由 problems 挡）。
    pub fn ready(&self) -> Vec<&TaskNode> {
        self.nodes
            .iter()
            .filter(|n| n.status == NodeStatus::Pending)
            .filter(|n| {
                n.deps.iter().all(|dep| {
                    self.nodes
                        .iter()
                        .find(|x| &x.id == dep)
                        .map(|x| x.status == NodeStatus::Done)
                        .unwrap_or(false)
                })
            })
            .collect()
    }

    /// 全部结束：链非空且每个节点都 Done 或 Failed。
    pub fn finished(&self) -> bool {
        !self.nodes.is_empty()
            && self
                .nodes
                .iter()
                .all(|n| matches!(n.status, NodeStatus::Done | NodeStatus::Failed))
    }

    /// 装配期的自洽检查：**空 = 一切正常**，非空 = 逐条可读的原因。
    /// 由装配期与测试双重消费（与工具总表/角色表同一套哲学：不靠人看）。
    pub fn problems(&self, roster: &[String]) -> Vec<String> {
        let mut out = Vec::new();
        if self.nodes.is_empty() {
            out.push("任务链是空的".to_string());
        }
        let mut seen: Vec<&str> = Vec::new();
        for n in &self.nodes {
            if n.id.trim().is_empty() {
                out.push("有节点没有 id".to_string());
            }
            if seen.contains(&n.id.as_str()) {
                out.push(format!("节点 id 重复：{}", n.id));
            }
            seen.push(&n.id);
            if n.objective.trim().is_empty() {
                out.push(format!("节点 {} 没有目标（任务提示词为空）", n.id));
            }
            if !roster.iter().any(|a| a == &n.assignee) {
                out.push(format!("节点 {} 的负责人不在名单里：{}", n.id, n.assignee));
            }
            for dep in &n.deps {
                if !self.nodes.iter().any(|x| &x.id == dep) {
                    out.push(format!("节点 {} 依赖了不存在的节点：{}", n.id, dep));
                }
            }
        }
        // 环：反复摘掉"依赖都已解决"的节点，摘不动就说明剩下的互相等（有环）。
        let mut done: Vec<String> = Vec::new();
        loop {
            let mut moved = false;
            for n in &self.nodes {
                if done.contains(&n.id) {
                    continue;
                }
                if n.deps.iter().all(|d| done.contains(d)) {
                    done.push(n.id.clone());
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
        for n in &self.nodes {
            if !done.contains(&n.id) {
                out.push(format!("节点 {} 处在环里（依赖互相等待）", n.id));
            }
        }
        out
    }
}
