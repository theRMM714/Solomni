//! 目的：会话历史的内存形态——会话元信息与历史视图。
//! 管：`SessionMeta` / `AgentMeta` / `HistoryView` / `Delegation` / `RunState` / `WorkspaceBinding` 等落盘事实的类型。
//! 不管：落盘机制（在 `detail/fs_history.rs` 的 `HistoryStore` 实现）；会话状态派生（在 `session.rs`）。
//! 联动：由 `service.rs` 与 `ports.rs` 使用，`api.rs` 对外重导出。

use serde::{Deserialize, Serialize};

/// 目的：会话元信息：创建工作时由用户决定的身份（形态 / 名单 / 档位 / 归属），
///   加上**此后会被机制改写的运行态**（`run`：暂停 / 关闭）。两者都是落盘事实，
///   是"这条会话是什么、还该不该被驱动"的唯一真相（不在内存里留影子状态）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionMeta {
    pub name: String,
    /// 目的：工作形态：`single` / `collab` / `proxy`（界面标签与 `rebuild_session` 的装配依据同源）。
    ///   它是落盘事实（**不在内存里留影子状态**）：拿不到它就无法判断怎么重建这条会话。
    pub mode: String,
    /// 目的：代拟（协作）时为 true：创建时没有名单，确认名单后才写回 agents。
    #[serde(default)]
    pub delegate: bool,
    /// 目的：本次工作的模块扁平清单（展示用；按 agents 的顺序展开）。
    pub modules: Vec<String>,
    /// 目的：协作：本次需求。
    #[serde(default)]
    pub task: Option<String>,
    pub ts: i64,
    /// 目的：本次工作参与的 agent 实例（模型随 agent 记；名单的唯一真相）。
    #[serde(default)]
    pub agents: Vec<AgentMeta>,
    /// 目的：执行档位与运行包选型（exec 段；缺字段按默认 = 本机档读回）。
    #[serde(default)]
    pub exec: crate::kernel::api::ExecSpec,
    /// 目的：共享区主副本绑定的外部工作区根；`None` = 产品内的 `session/<名>/work`。
    /// 约束：只写在工作根上（子会话沿 `parent` 继承同一个根），创建后不可改。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceBinding>,
    /// 目的：这条会话的权限层：根是 ceiling，子是 narrowing。全局给默认值，这里只覆盖显式字段。
    #[serde(default)]
    pub sandbox: crate::capabilities::permission::api::SessionSandbox,
    /// 目的：思考强度（会话级，只记在**工作根**上；子会话沿 parent 链共享同一个值）。
    /// 约束：None = 随模型默认；各模型是否支持由登记处的 thinking 映射决定（不支持的档拒绝切换）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    /// 目的：谁编排的（子会话 = 父会话名；顶层会话为空）。
    ///   也是**落点判据**：子会话落在父会话目录的 `children/` 下（可再嵌套）。
    ///   整棵树不论嵌套多少层，**只有顶层一个 `work/`**——所有子会话共用它。
    #[serde(default)]
    pub parent: Option<String>,
    /// 目的：这个子会话服务任务链里的哪个节点（顶层会话为空）。
    #[serde(default)]
    pub node: Option<String>,
    /// 目的：任务级委托（全权）：`Some` = 这条会话的核心可以用代理工具代用户决定。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation: Option<Delegation>,
    /// 目的：**过期内容自动清理**开关（用户在界面上控制：省上下文，但每次判过期都要读对象库）。
    /// 约束：必须落盘——回放与实时要产出同一条请求，开关状态不能只活在内存里。
    ///   缺省 false = 只记读事实、发送视图原样（不付判定的成本）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub context_freshness: bool,
    /// 目的：运行态：停止后不再被派发或唤醒；关闭是终态。缺省（active）= 正常运行。
    ///   它与"这一刻在不在跑"（推的 `Working`，短暂不落盘）是两件事：**它是持久事实**，
    ///   重启/回档后照样成立（见 docs/session/session-model.md 二）。
    #[serde(default, skip_serializing_if = "RunState::is_active")]
    pub run: RunState,
}

/// 目的：根会话绑定到产品目录之外的共享区主副本时记录的工作区根（绝对路径）与围栏授权粒度。
/// 约束：子会话不写这个字段（沿 `parent` 链继承工作根的绑定）；会话建好后不再改。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceBinding {
    /// 目的：规范化后的绝对路径（创建时校验；之后只读）。
    pub root: std::path::PathBuf,
    /// 目的：外部工作区在围栏里的授权粒度（创建时由人选；缺省 = 今天的行为：逐会话授/撤）。
    #[serde(default)]
    pub grant: WorkspaceGrant,
}

/// 目的：外部工作区的围栏授权粒度——决定容器里的 shell / 模块工具能不能读共享区，以及写 ACL 的频率。
/// 约束：**只影响围栏与共享区读范围**，不改变文件归属；`Workspace` 的机器级 ACE 常驻用户目录，撤销走显式命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceGrant {
    /// 不向容器授共享区：容器里的 shell / 模块工具读不到（内置文件工具同样按权限层拒绝，如实告知）。
    None,
    /// 仅本次会话（缺省）：按次调用授给本会话的容器 SID，会话结束递归撤——首次付 O(子树)，每会话都付。
    #[default]
    Session,
    /// 该工作区一次性：机器级只读授给 `ALL APPLICATION PACKAGES` + `(OI)(CI)`，之后所有会话零写。
    /// 代价：本机任意 AppContainer 应用都能读这个目录；撤销走 `--fence-revoke <path>`。
    Workspace,
}

impl WorkspaceGrant {
    /// 目的：解析创建参数（`none` / `session` / `workspace`）；不认识的值如实拒绝，不静默落默认。
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "none" => Ok(WorkspaceGrant::None),
            "session" => Ok(WorkspaceGrant::Session),
            "workspace" => Ok(WorkspaceGrant::Workspace),
            other => Err(format!(
                "未知的工作区授权粒度：{other}（可用 none / session / workspace）"
            )),
        }
    }

    /// 目的：落盘与人读的稳定写法（与 serde 的 snake_case 同名）。
    pub fn as_str(self) -> &'static str {
        match self {
            WorkspaceGrant::None => "none",
            WorkspaceGrant::Session => "session",
            WorkspaceGrant::Workspace => "workspace",
        }
    }
}

/// 目的：会话运行态（**持久**事实，落在 meta.yaml）：派发与唤醒都要先过它。
///   它是"这段工作还该不该继续被驱动"的开关，与"此刻有没有一次生成在跑"分开。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    /// 正常：可以被派发与唤醒。
    #[default]
    Active,
    /// 已停止：一切派发与唤醒都不再启动（在跑的那次已被停下）。用户「继续」回到 Active。
    Stopped,
    /// 关闭：终态，不能再被派发、唤醒或 resume。
    Closed,
}

impl RunState {
    pub fn as_str(self) -> &'static str {
        match self {
            RunState::Active => "active",
            RunState::Stopped => "stopped",
            RunState::Closed => "closed",
        }
    }
    /// 目的：serde 的 `skip_serializing_if` 用：缺省态不写进 meta.yaml。
    pub fn is_active(&self) -> bool {
        *self == RunState::Active
    }
}

/// 目的：任务级委托：代理模式下，真实用户把决定权**整块**交给核心（全权）。
///   它只是一个存在标志与授予时间；范围 / 期限 / 决定粒度与额外路径权限归独立的
///   **权限管理**能力（`capabilities/permission/`，与某个会话本身无关）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Delegation {
    /// 目的：授予时间（Unix 秒）：授予由真实用户在建工作时完成。
    pub granted_at: i64,
}

impl SessionMeta {
    /// 目的：允许被派发 / 唤醒吗？停止与关闭都拒绝——**运行态的唯一判据只有这一处**
    ///   （派发入口、代理转达、叫醒都问它，不各写一份）。None = 放行。
    pub fn dispatch_refusal(&self) -> Option<String> {
        match self.run {
            RunState::Active => None,
            RunState::Stopped => Some(format!(
                "会话 {} 已停止：先「继续」再派发（停止期间不接受任何派发或唤醒）",
                self.name
            )),
            RunState::Closed => Some(format!(
                "会话 {} 已关闭：终态，不能再派发、唤醒或继续",
                self.name
            )),
        }
    }
}

/// 目的：会话里的一个 agent 实例记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMeta {
    /// 目的：实例名（重名时已加 -2/-3；也是其沙箱目录名）。
    pub name: String,
    /// 目的：是否临时 agent（不来自 agents.yaml）。
    #[serde(default)]
    pub transient: bool,
    pub modules: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// 目的：人设 / 身份（建会话时从登记处或临时声明里快照；之后与会话一起走）。
    #[serde(default)]
    pub chara: String,
}

/// 目的：历史列表条目。
#[derive(Debug, Clone, serde::Serialize)]
pub struct HistoryView {
    pub name: String,
    pub mode: String,
    pub ts: i64,
    pub done: bool,
    /// 目的：这条会话记的执行档位与选型（meta.yaml 的 exec 段；缺字段按默认 = 本机档读回）。
    ///   列表视图据此提示"环境已变"——**不拦打开**，记录是用户的。
    #[serde(default)]
    pub exec: crate::kernel::api::ExecSpec,
    /// 目的：谁编排的（子会话 = 父会话名）：侧栏据此把子会话缩进挂在父会话下。
    #[serde(default)]
    pub parent: Option<String>,
    /// 目的：外部工作区根；None = 产品内布局。侧栏据此把删除文案说成“只删记录”。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// 目的：绑定外部工作区但根已不可达（侧栏标红、禁止打开）。
    #[serde(default)]
    pub workspace_missing: bool,
    /// 目的：运行态（`active` / `stopped` / `closed`）：侧栏据此标出已停止 / 已关闭的会话
    ///   （"这一刻在不在跑"是另一回事，见 `SessionView.running`）。
    pub run: RunState,
}
