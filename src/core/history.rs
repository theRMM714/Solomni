//! 会话历史：会话元信息与历史视图的内存形态。
//! 落盘机制（session/<名字>/ 目录与 jsonl）在适配层（HistoryStore 端口）。

use serde::{Deserialize, Serialize};

/// 会话元信息：创建工作时由用户决定、此后不再改动的部分。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionMeta {
    pub name: String,
    /// direct / compose / collab
    pub mode: String,
    /// 代拟（协作）时为 true：创建时没有名单，确认名单后才写回 agents。
    #[serde(default)]
    pub delegate: bool,
    /// 本次工作的模块扁平清单（展示用；按 agents 的顺序展开）。
    pub modules: Vec<String>,
    /// 协作：本次需求。
    #[serde(default)]
    pub task: Option<String>,
    pub ts: i64,
    /// 本次工作参与的 agent 实例（模型随 agent 记；名单的唯一真相）。
    #[serde(default)]
    pub agents: Vec<AgentMeta>,
    /// 执行档位与运行包选型（exec 段；缺字段的旧会话按默认 = 本机档读回）。
    #[serde(default)]
    pub exec: crate::core::exec::ExecSpec,
    /// 谁编排的（子会话 = 父会话名；顶层会话为空）。
    /// 也是**沙箱锚点**：子会话与父会话共用一套工作区（协作的产物要在一起）。
    #[serde(default)]
    pub parent: Option<String>,
    /// 这个子会话服务任务链里的哪个节点（顶层会话为空）。
    #[serde(default)]
    pub node: Option<String>,
}

impl SessionMeta {
    /// 沙箱锚点：子会话锚在父会话上，顶层会话锚在自己身上。
    pub fn work(&self) -> &str {
        self.parent.as_deref().unwrap_or(&self.name)
    }
}

/// 会话里的一个 agent 实例记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMeta {
    /// 实例名（重名时已加 -2/-3；也是其沙箱目录名）。
    pub name: String,
    /// 是否临时 agent（不来自 agents.yaml）。
    #[serde(default)]
    pub transient: bool,
    pub modules: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
}

/// 历史列表条目。
#[derive(Debug, Clone, serde::Serialize)]
pub struct HistoryView {
    pub name: String,
    pub mode: String,
    pub ts: i64,
    pub done: bool,
    /// 这条会话记的执行档位与选型（meta.yaml 的 exec 段；缺字段的旧会话按默认 = 本机档读回）。
    /// 列表视图据此提示"环境已变"——**不拦打开**，记录是用户的。
    #[serde(default)]
    pub exec: crate::core::exec::ExecSpec,
    /// 谁编排的（子会话 = 父会话名）：侧栏据此把子会话缩进挂在父会话下。
    #[serde(default)]
    pub parent: Option<String>,
}
