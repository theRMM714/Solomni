//! 目的：会话历史的内存形态——会话元信息与历史视图。
//! 管：`SessionMeta` / `HistoryView` / `Delegation` / `RunState` 等落盘事实的类型；
//!   agent 实例（`AgentMeta`）归登记处（`registry::api`），这里只持有快照。
//! 不管：落盘机制（在 `detail/fs_history.rs` 的 `HistoryStore` 实现）；会话状态派生（在 `session.rs`）。
//! 联动：由 `service.rs` 与 `ports.rs` 使用，`api.rs` 对外重导出。

use crate::units::business::session_model::domain::profile::SessionSpec;
use crate::units::pure::registry_model::api::AgentMeta;
use serde::{Deserialize, Serialize};

/// 目的：会话元信息：创建工作时由用户决定的身份（形态 / 名单 / 档位 / 归属），
///   加上**此后会被机制改写的运行态**（`run`：暂停 / 关闭）。两者都是落盘事实，
///   是"这条会话是什么、还该不该被驱动"的唯一真相（不在内存里留影子状态）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionMeta {
    pub name: String,
    /// 目的：这条会话的**解析后声明**（类型名 / 静止席位 / 按顺序跑的流程）。
    ///   它是落盘事实（**不在内存里留影子状态**）：拿不到它就无法判断怎么重建这条会话。
    ///   约束：落的是解析结果而不是对表的引用——改那张表不影响已建会话的重建。
    pub spec: SessionSpec,
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
    /// 目的：这条会话要的执行环境：策略 id、定版表与策略自己的参数。
    /// 约束：解释权在策略实现；会话只认事实（下面那格），不读 `params` 里的键。
    #[serde(default)]
    pub sandbox: crate::units::business::sandbox::api::SandboxDecl,
    /// 目的：上面那份声明解析出来的事实（哪一侧、客机是什么）——路径与安装侧据此判定。
    /// 约束：改声明时同一次重新求值（`work` 的改配路径负责），别让两格漂。
    #[serde(default)]
    pub facts: crate::units::business::sandbox::api::SandboxFacts,
    /// 目的：这条会话要的工作台（策略 id / 外部根 / 策略自己的参数）。
    /// 约束：外部根只写在工作根上（子会话沿 `parent` 链继承同一个根），创建后不可改；
    ///   `params` 由工作台单元解释，会话不认里面的字段。
    #[serde(default)]
    pub workspace: crate::units::business::session_model::domain::workspace::WorkspaceDecl,
    /// 目的：这条会话的权限层：根是 ceiling，子是 narrowing。全局给默认值，这里只覆盖显式字段。
    #[serde(default)]
    pub permission: crate::units::pure::permission::api::PermissionOverride,

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
    /// 目的：**过期内容保底**（百分比）：发送视图里前这么多内容原样保留，只替换其后的过期读取。
    /// 约束：只写在工作根上（子会话沿根继承）；值域 0..=90，0 = 不保底。
    #[serde(default = "default_freshness_keep_percent")]
    pub freshness_keep_percent: u8,
    /// 目的：这条会话**累计**的模型花费（跨回合、跨压缩；回档不动它——花出去的钱退不回来）。
    /// 约束：只累计供应商**精确**报回的调用（字符估算不进）；由核心线程在会话交回时累加。
    #[serde(default, skip_serializing_if = "UsageTotals::is_empty")]
    pub usage_totals: UsageTotals,
    /// 目的：运行态：停止后不再被派发或唤醒；关闭是终态。缺省（active）= 正常运行。
    ///   它与"这一刻在不在跑"（推的 `Working`，短暂不落盘）是两件事：**它是持久事实**，
    ///   重启/回档后照样成立（见 docs/session/session-model.md 二）。
    #[serde(default, skip_serializing_if = "RunState::is_active")]
    pub run: RunState,
}

/// 目的：meta 里省略保底百分比时的默认值（开启开关时的默认保底 80%）。
pub fn default_freshness_keep_percent() -> u8 {
    80
}

/// 目的：一条会话**累计**的模型花费（面板读它显示累计输入 / 输出与缓存命中）。
/// 约束：它是账面事实——不随回档、折叠、压缩变化，只累加供应商精确报回的调用；
///   单位是 token；Anthropic 形状的缓存读/写已在 llm 层并进 `prompt`，这里分开记只为算出命中率。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageTotals {
    /// 目的：累计的模型调用次数（含压缩调用）。
    pub calls: u64,
    /// 目的：累计输入 token（含命中缓存与写入缓存的部分）。
    pub prompt: u64,
    /// 目的：累计输出 token。
    pub completion: u64,
    /// 目的：累计**命中**前缀缓存的输入 token。
    pub cache_read: u64,
    /// 目的：累计**写进**前缀缓存的输入 token。
    pub cache_creation: u64,
}

impl UsageTotals {
    /// 目的：serde 的 `skip_serializing_if` 用：全零（还没调用过）时不写进 meta.yaml。
    pub fn is_empty(&self) -> bool {
        *self == UsageTotals::default()
    }

    /// 目的：把一次调用的用量并进来（增量由会话在交回核心线程时给出）。
    pub fn add(&mut self, other: UsageTotals) {
        self.calls += other.calls;
        self.prompt += other.prompt;
        self.completion += other.completion;
        self.cache_read += other.cache_read;
        self.cache_creation += other.cache_creation;
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
    /// 已归档：从主列表收起，不再被派发或唤醒（「恢复归档」回到 Active）。
    ///   归档不是终态：删除才是终局。
    Archived,
}

impl RunState {
    pub fn as_str(self) -> &'static str {
        match self {
            RunState::Active => "active",
            RunState::Stopped => "stopped",
            RunState::Archived => "archived",
        }
    }
    /// 目的：serde 的 `skip_serializing_if` 用：缺省态不写进 meta.yaml。
    pub fn is_active(&self) -> bool {
        *self == RunState::Active
    }
}

/// 目的：任务级委托：代理模式下，真实用户把决定权**整块**交给核心（全权）。
///   它只是一个存在标志与授予时间；范围 / 期限 / 决定粒度与额外路径权限归独立的
///   **权限管理**能力（`units/pure/permission/`，与某个会话本身无关）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Delegation {
    /// 目的：授予时间（Unix 秒）：授予由真实用户在建工作时完成。
    pub granted_at: i64,
}

impl SessionMeta {
    /// 目的：这条会话是不是代理会话：没有流程、也没有自己的人员名单——活交给子会话，核心代理在此工作。
    /// 约束：判据只看落盘声明与名单（不看类型名），自定的代理式预设同样算；单机与会议都有名单或流程。
    pub fn is_proxy(&self) -> bool {
        self.spec.is_proxy_spec() && self.agents.is_empty()
    }

    /// 目的：把落盘的执行环境声明与事实换成执行选型（计划、就绪、寻址都读这一份派生值）。
    /// 约束：不落盘；改声明时同一次重算 facts，声明与派生值永远同源。
    pub fn target(&self) -> crate::units::pure::exec_model::api::TargetSpec {
        self.sandbox.target(&self.facts)
    }

    /// 目的：允许被派发 / 唤醒吗？停止与归档都拒绝——**运行态的唯一判据只有这一处**
    ///   （派发入口、代理转达、叫醒都问它，不各写一份）。None = 放行。
    pub fn dispatch_refusal(&self) -> Option<String> {
        match self.run {
            RunState::Active => None,
            RunState::Stopped => Some(format!(
                "会话 {} 已停止：先「继续」再派发（停止期间不接受任何派发或唤醒）",
                self.name
            )),
            RunState::Archived => Some(format!(
                "会话 {} 已归档：先「恢复归档」再派发（归档期间不接受任何派发或唤醒）",
                self.name
            )),
        }
    }
}

/// 目的：历史列表条目。
#[derive(Debug, Clone, serde::Serialize)]
pub struct HistoryView {
    pub name: String,
    /// 目的：会话类型名（单机 / 会议 / 代理，或用户自定的预设名）。
    pub kind: String,
    /// 目的：这条会话有没有流程在推进（界面据此决定能不能自由发言）。
    pub flows: bool,
    /// 目的：决定权是不是整块交给核心（没有名单、没有需求：界面上叫代理式）。
    pub auto: bool,
    pub ts: i64,
    pub done: bool,
    /// 目的：这条会话记的执行环境声明（meta.yaml 的 sandbox 段；缺字段按默认 = 本机策略读回）。
    ///   列表视图据此提示"环境已变"——**不拦打开**，记录是用户的。
    #[serde(default)]
    pub sandbox: crate::units::business::sandbox::api::SandboxDecl,
    /// 目的：声明解析出来的事实（哪一侧、guest 名）：列表视图与就绪判定读它，不各自重算。
    #[serde(default)]
    pub facts: crate::units::business::sandbox::api::SandboxFacts,
    /// 目的：谁编排的（子会话 = 父会话名）：侧栏据此把子会话缩进挂在父会话下。
    #[serde(default)]
    pub parent: Option<String>,
    /// 目的：外部工作区根；None = 产品内布局。侧栏据此把删除文案说成“只删记录”。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// 目的：绑定外部工作区但根已不可达（侧栏标红、禁止打开）。
    #[serde(default)]
    pub workspace_missing: bool,
    /// 目的：运行态（`active` / `stopped` / `archived`）：侧栏据此标出已停止 / 已归档的会话
    ///   （"这一刻在不在跑"是另一回事，见 `SessionView.running`）。
    pub run: RunState,
}
