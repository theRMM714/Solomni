//! 入站能力面：**其它能力、conductor 与呈现层只准用这里**（不许碰 `domain` / `ports`）。
//!
//! 两样东西在这里：
//! - **DTO 的重导出**：登记处的词汇（供应商 / 模型 / agent / 设置）只有一份定义，在 `domain`。
//! - **`Registry` 能力面**：登记处的状态与用例（`service.rs` 实现它）。`conductor` 只持有
//!   `Box<dyn Registry>`，看不见它的字段——**四份 yaml 的内存形态只由 `service.rs` 写**。
//!
//! 读方法取 `&self`、写方法取 `&mut self`：状态住在 conductor 的执行线程上，靠单线程命令队列
//! 保证互斥，因此**不额外上锁**（见 ARCHITECTURE.md §一 的并发不变式）。

pub use crate::capabilities::registry::domain::agents::{
    check_reserved, listing, model_listing, resolve_picks, unique_instance_name, validate_name,
    AgentView, Agents, RosterPick,
};
pub use crate::capabilities::registry::domain::providers::{
    AppSettings, ModelEntry, ModelView, Provider, ProviderView, Settings,
};

use crate::capabilities::llm::api::{Channel, ProbeOutcome, ReplayReport, ToolMode};
use crate::capabilities::workspace::api::Roster;

/// 登记处的**队列面**：呈现层经 `ConductorHandle`（核心自己的线程 + 命令队列）调它。
///
/// 与 `Registry`（能力面）的分工是**有意的不对称**，不是重复：
/// - `Registry` 的写方法取 `&mut self`——只有那样"单写者"才是**编译期事实**（R4/R5）；
/// - 本 trait 全取 `&self`——呈现层持有的是可克隆的句柄（多连接共用），队列独占在**线程那一侧**，
///   不是靠类型系统在调用点表达。
///
/// 两者因此**不能收成一个 trait**（接收者不同）；收口判据见 ARCHITECTURE.md §九.6 R12。
pub trait RegistryOps: Send + Sync {
    fn providers(&self) -> Result<Vec<ProviderView>, String>;
    fn upsert_provider(&self, id: &str, base_url: &str, api_key: &str) -> Result<(), String>;
    fn remove_provider(&self, id: &str) -> Result<bool, String>;
    fn models(&self) -> Result<Vec<ModelView>, String>;
    fn core_model(&self) -> Result<Option<String>, String>;
    fn upsert_model(
        &self,
        id: &str,
        name: &str,
        api_model: &str,
        provider: &str,
        note: &str,
        // 上下文窗口（tokens）；0 = 保留现值（新建缺省 32k）。
        context: u64,
    ) -> Result<(), String>;
    fn remove_model(&self, id: &str) -> Result<bool, String>;
    fn set_core_model(&self, id: &str) -> Result<bool, String>;
    fn agents(&self) -> Result<Vec<AgentView>, String>;
    /// 点名：按名字取 agent 视图；**不猜、不代选**——名字不在登记处就如实报错。
    fn pick_agents(&self, names: &[String]) -> Result<Vec<AgentView>, String>;
    fn upsert_agent(
        &self,
        name: &str,
        modules: &[String],
        model: &str,
        note: &str,
    ) -> Result<(), String>;
    fn remove_agent(&self, name: &str) -> Result<bool, String>;
    fn settings(&self) -> Result<AppSettings, String>;
    fn set_settings(&self, app: AppSettings) -> Result<(), String>;
    fn discover_models(&self, provider_id: &str) -> Result<Vec<String>, String>;
    /// 实测一条通道支不支持原生工具调用（要真实网络；三种结论都如实回报，
    /// 只把**确定**的结论写回登记处 —— 这条规则在能力里，不在呈现层）。
    fn probe_model_tools(&self, id: &str) -> Result<ProbeOutcome, String>;
    /// 实测这种"回放形状"供应商收不收、模型有没有真的读懂（要真实网络；**不改登记处**）。
    fn probe_replay_shape(&self, id: &str) -> Result<ReplayReport, String>;
}

/// 登记处能力面：供应商 / 模型 / agent / 基本设置，以及通道上的模型发现与探测。
///
/// 两类方法对应两类调用方：
/// - **呈现层要的**（`provider_views` / `model_views` / `agent_views` / `app_settings` …）——
///   它经 `RegistryOps`（本文件）走命令队列进来（见 `conductor/api.rs`）；
/// - **其它能力与 conductor 要的只读事实**（`resolve` / `tool_mode` / `context_of` / `snapshot` …）——
///   谁是登记处的主人就由谁答，调用方不自己维护一份镜像。
///
/// 实现者是 `service.rs` 的 `RegistryService`（**状态在它里面**，conductor 只持 `Box<dyn Registry>`）。
pub trait Registry: Send + Sync {
    // ---- 视图：给呈现层与别的能力看的登记处（**永不携带密钥**） ----

    /// 供应商视图（不含密钥）。
    fn provider_views(&self) -> Vec<ProviderView>;
    /// 模型视图（含「是否核心默认」与形态）。
    fn model_views(&self) -> Vec<ModelView>;
    /// agent 视图。
    fn agent_views(&self) -> Vec<AgentView>;
    /// 点名：按名字取 agent 视图；不猜、不代选，名字不在册就如实报错。
    fn pick_agents(&self, names: &[String]) -> Result<Vec<AgentView>, String>;
    /// 基本设置（**借用**：conductor 在自己进程里读设置项用它，不复制一份）。
    fn app(&self) -> &AppSettings;
    /// 基本设置（**自持一份**：经命令队列回给呈现层时必须拥有所有权）。
    fn app_settings(&self) -> AppSettings;
    /// 核心 AI 默认模型 id。
    fn core_model(&self) -> Option<String>;

    // ---- 只读事实：别的能力问「通道怎么发 / 有没有它 / 窗口多大」 ----

    /// 模型 id → 成品通道；模型或供应商缺失 = 报错（不猜测回退）。
    fn resolve(&self, model: &str) -> Result<Channel, String>;
    /// 核心 AI 默认通道；未设定或解析失败 = None（调用方回落演示并如实告知）。
    fn core_channel(&self) -> Option<Channel>;
    /// 本次调用要用的通道：给了模型用它，否则用核心默认；两者都缺 = None。
    fn channel(&self, model: Option<&str>) -> Option<Channel>;
    /// 这个模型登记在册吗。
    fn has_model(&self, id: &str) -> bool;
    /// 登记处里一个模型都没有吗（推荐与代拟的前置判据）。
    fn any_models(&self) -> bool;
    /// 这个名字在册吗；不在 = 临时组合的实例（界面按此区分）。
    fn has_agent(&self, name: &str) -> bool;
    /// 该模型的工具调用形态：它自己的优先，其次核心默认，都没有 = envelope。
    fn tool_mode(&self, model: Option<&str>) -> ToolMode;
    /// 该模型（缺省用核心默认）的上下文窗口（tokens）；未登记 = 保守缺省。
    fn context_of(&self, model: Option<&str>) -> u64;
    /// 登记处**快照**：协作会话自持一份（与今日 `settings.clone()` 同义，语义不变）。
    fn snapshot(&self) -> Settings;
    // ---- 写：只有这里改登记处，改完就落盘 ----

    /// 新建/更新供应商。更新时 `api_key` 留空 = 保留原密钥（界面从不回显密钥）。
    fn provider_upsert(&mut self, id: &str, base_url: &str, api_key: &str) -> Result<(), String>;
    /// 删除供应商；仍被模型引用时拒绝（不静默级联删除）。
    fn provider_remove(&mut self, id: &str) -> Result<bool, String>;
    /// 新建/更新模型。`context` 为 0 = 保留现值（新建缺省 32k）；工具形态保留原值。
    fn model_upsert(
        &mut self,
        id: &str,
        name: &str,
        api_model: &str,
        provider: &str,
        note: &str,
        context: u64,
    ) -> Result<(), String>;
    /// 删除模型；是核心默认模型时拒绝（先改默认再删）。
    fn model_remove(&mut self, id: &str) -> Result<bool, String>;
    /// 设定核心默认模型；模型不存在 = false（不改动）。
    fn core_set_model(&mut self, id: &str) -> Result<bool, String>;
    /// 新建/覆盖一个 agent（模块按**调用方给的清单**校验，模型必须真实存在）。
    fn agent_upsert(
        &mut self,
        name: &str,
        modules: &[String],
        model: &str,
        note: &str,
        roster: &Roster,
    ) -> Result<(), String>;
    /// 删除 agent。
    fn agent_remove(&mut self, name: &str) -> Result<bool, String>;
    /// 保存基本设置。
    fn set_app_settings(&mut self, app: AppSettings) -> Result<(), String>;
    /// 实测一条通道支不支持原生工具调用，并把**确定**的结论写回登记处。
    fn probe_model_tools(&mut self, id: &str) -> Result<ProbeOutcome, String>;
    /// 实测一条通道的**回放形状**；只报事实、**不写登记处**。
    fn probe_replay_shape(&self, id: &str) -> Result<ReplayReport, String>;
    /// 用已存的供应商去拉取它的可用模型名（发现机制在适配层）。
    fn discover_models(&self, provider_id: &str) -> Result<Vec<String>, String>;
}
