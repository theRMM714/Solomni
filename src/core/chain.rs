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
    /// 该节点的**产出**（子会话跑完一轮的最后一条转录行）；未跑完 = None。
    #[serde(default)]
    pub report: Option<String>,
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
    /// 每个节点的**阶段**（依赖图里的最长路径层数，1 起）：无依赖 = 1，其余 = 依赖里最大的阶段 + 1。
    /// 这是"阶段"的**唯一判据**（纯图论，不看 id 怎么写）：同阶段互不依赖 → 可并发；跨阶段串行。
    /// 依赖成环时 problems() 会挡下（这里给环里的节点留 1，不死循环）。
    pub fn stages(&self) -> Vec<usize> {
        let mut stage: Vec<Option<usize>> = vec![None; self.nodes.len()];
        loop {
            let mut moved = false;
            for (i, n) in self.nodes.iter().enumerate() {
                if stage[i].is_some() {
                    continue;
                }
                let mut max = 0usize;
                let mut ok = true;
                for dep in &n.deps {
                    match self.nodes.iter().position(|x| &x.id == dep) {
                        Some(j) => match stage[j] {
                            Some(s) => max = max.max(s),
                            None => ok = false,
                        },
                        // 悬空依赖：装配门禁（problems）会挡下，这里不静默算通过。
                        None => ok = false,
                    }
                }
                if ok {
                    stage[i] = Some(max + 1);
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
        stage.into_iter().map(|s| s.unwrap_or(1)).collect()
    }

    /// **按阶段定名**：第 s 阶段的第 k 个节点 = n{s}-{k}（s 与 k 都由依赖图派生，k 从 1 起、按链内顺序）。
    /// 依赖按"旧 id → 新 id"整体重映射：模型给的 id 只用来解析依赖，**落库的 id 由核心派生**
    /// （与"代码里不出现可由表派生的名单"同一口径）。装配门禁通过之后调用。
    pub fn renumber_by_stage(&mut self) {
        let stages = self.stages();
        let mut used: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
        let mut map: Vec<(String, String)> = Vec::new();
        for (i, n) in self.nodes.iter().enumerate() {
            let s = stages[i];
            let k = used.entry(s).or_insert(0);
            *k += 1;
            map.push((n.id.clone(), format!("n{}-{}", s, *k)));
        }
        for (i, n) in self.nodes.iter_mut().enumerate() {
            n.id = map[i].1.clone();
        }
        for n in self.nodes.iter_mut() {
            n.deps = n
                .deps
                .iter()
                .filter_map(|d| {
                    map.iter()
                        .find(|(old, _)| old == d)
                        .map(|(_, new)| new.clone())
                })
                .collect();
        }
    }

    /// 某个阶段的节点（按链内顺序）。
    pub fn stage_nodes(&self, stage: usize) -> Vec<&TaskNode> {
        let stages = self.stages();
        self.nodes
            .iter()
            .enumerate()
            .filter(|(i, _)| stages[*i] == stage)
            .map(|(_, n)| n)
            .collect()
    }

    /// 某个阶段**是否已通过**：该阶段每个节点都 Done 且验收结论为"通过"。
    pub fn stage_passed(&self, stage: usize) -> bool {
        let nodes = self.stage_nodes(stage);
        !nodes.is_empty()
            && nodes.iter().all(|n| {
                n.status == NodeStatus::Done && n.acceptance.as_ref().map(|a| a.ok).unwrap_or(false)
            })
    }

    /// **当前阶段**：最小的"还没通过"的阶段；None = 所有阶段都过了（可以总验收）。
    pub fn current_stage(&self) -> Option<usize> {
        let stages = self.stages();
        let max = stages.iter().copied().max().unwrap_or(0);
        (1..=max).find(|s| !self.stage_passed(*s))
    }

    /// 这一阶段**跑完了吗**（每个节点都 Done / Failed）——跑完才做阶段验收。
    pub fn stage_settled(&self, stage: usize) -> bool {
        let nodes = self.stage_nodes(stage);
        !nodes.is_empty()
            && nodes
                .iter()
                .all(|n| matches!(n.status, NodeStatus::Done | NodeStatus::Failed))
    }

    /// 这一阶段**可以派发**的节点（待办）：同阶段的依赖都在同阶段之前且已通过，所以不必再查依赖。
    pub fn stage_ready(&self, stage: usize) -> Vec<&TaskNode> {
        self.stage_nodes(stage)
            .into_iter()
            .filter(|n| n.status == NodeStatus::Pending)
            .collect()
    }

    /// 全部结束：链非空且每个节点都 Done 或 Failed。消费方同 ready()。
    #[cfg(test)]
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
