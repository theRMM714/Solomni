//! 会话历史：会话元信息与历史视图的内存形态。
//! 落盘机制（session/<名字>/ 目录与 jsonl）在适配层（HistoryStore 端口）。

use serde::{Deserialize, Serialize};

/// 会话元信息：创建工作时由用户决定的身份（形态 / 名单 / 档位 / 归属），
/// 加上**此后会被机制改写的运行态**（`run`：暂停 / 关闭）。两者都是落盘事实，
/// 是"这条会话是什么、还该不该被驱动"的唯一真相（不在内存里留影子状态）。
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
    pub exec: crate::capabilities::workspace::api::ExecSpec,
    /// 谁编排的（子会话 = 父会话名；顶层会话为空）。
    /// 也是**沙箱锚点**（协作的节点子会话与父会话共用一套工作区，产物要在一起）。
    #[serde(default)]
    pub parent: Option<String>,
    /// 这个子会话服务任务链里的哪个节点（顶层会话为空）。
    #[serde(default)]
    pub node: Option<String>,
    /// 任务级委托（全权）：`Some` = 这条会话的核心可以用代理工具代用户决定。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation: Option<Delegation>,
    /// 工作区锚点：`false`（缺省）= 与父会话**共用**一套工作区（协作的节点子会话）；
    /// `true` = 这是代理在父会话下建的**子工作**，有自己的 `work/` 与沙箱——
    /// 只有"谁编排的"仍是父会话。
    #[serde(default, skip_serializing_if = "is_false")]
    pub own_work: bool,
    /// 运行态：暂停后不再被派发或唤醒；关闭是终态。缺省（active）= 正常运行。
    /// 它与"这一刻在不在跑"（推的 `Working`，短暂不落盘）是两件事：**它是持久事实**，
    /// 重启/回档后照样成立（见 docs/session/session-model.md 二）。
    #[serde(default, skip_serializing_if = "RunState::is_active")]
    pub run: RunState,
}

/// 会话运行态（**持久**事实，落在 meta.yaml）：派发与唤醒都要先过它。
/// 它是"这段工作还该不该继续被驱动"的开关，与"此刻有没有一次生成在跑"分开。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    /// 正常：可以被派发与唤醒。
    #[default]
    Active,
    /// 暂停：一切派发与唤醒都不再启动（在跑的那次已被停下）。可 resume 回到 Active。
    Paused,
    /// 关闭：终态，不能再被派发、唤醒或 resume。
    Closed,
}

impl RunState {
    pub fn as_str(self) -> &'static str {
        match self {
            RunState::Active => "active",
            RunState::Paused => "paused",
            RunState::Closed => "closed",
        }
    }
    /// serde 的 `skip_serializing_if` 用：缺省态不写进 meta.yaml。
    pub fn is_active(&self) -> bool {
        *self == RunState::Active
    }
}

/// 任务级委托：代理模式下，真实用户把决定权**整块**交给核心（全权）。
/// 本轮不做范围/期限/撤销——它只是一个存在标志与授予时间；
/// 细粒度权限与撤销属独立的"会话权限状态"能力
/// （见 src/capabilities/conductor/testgaps.yaml 的 conductor.proxy-permission-state）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Delegation {
    /// 授予时间（Unix 秒）：授予由真实用户在建工作时完成。
    pub granted_at: i64,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl SessionMeta {
    /// 沙箱锚点：**子工作**锚在自己身上（自己的 work/ 与沙箱）；
    /// 其余子会话（协作的节点会话）锚在父会话上；顶层会话锚在自己身上。
    pub fn work(&self) -> &str {
        if self.own_work {
            &self.name
        } else {
            self.parent.as_deref().unwrap_or(&self.name)
        }
    }

    /// 允许被派发 / 唤醒吗？暂停与关闭都拒绝——**运行态的唯一判据只有这一处**
    /// （派发入口、代理转达、叫醒都问它，不各写一份）。None = 放行。
    pub fn dispatch_refusal(&self) -> Option<String> {
        match self.run {
            RunState::Active => None,
            RunState::Paused => Some(format!(
                "会话 {} 已暂停：先 resume 再派发（暂停期间不接受任何派发或唤醒）",
                self.name
            )),
            RunState::Closed => Some(format!(
                "会话 {} 已关闭：终态，不能再派发、唤醒或 resume",
                self.name
            )),
        }
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
    pub exec: crate::capabilities::workspace::api::ExecSpec,
    /// 谁编排的（子会话 = 父会话名）：侧栏据此把子会话缩进挂在父会话下。
    #[serde(default)]
    pub parent: Option<String>,
}
