//! 供应商与模型登记处（内存形态）与通道解析。
//! 供应商 = 端点 + 密钥；模型 = 独立实体（展示名 + 实际模型串 + 所属供应商 + 能力说明）。
//! 两者分开保存（机制在适配层的两个文件）；密钥只存在于登记处与核心发起的出站调用。
//! 「模型 → 供应商」的绑定只对本产品可见；模块与会话只持模型 id 引用。

use crate::capabilities::llm::api::{Channel, ToolMode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 一条供应商通道（端点 + 密钥）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub base_url: String,
    pub api_key: String,
}

/// 一条可用模型：展示名 + 实际模型串 + 所属供应商 + 能力说明 + 工具调用形态。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    pub name: String,
    pub api_model: String,
    pub provider: String,
    #[serde(default)]
    pub note: String,
    /// 工具调用形态（缺省 envelope；填 native 前应当用探测确认真实支持，见 REGISTRY_SPEC）。
    #[serde(default)]
    pub tools: ToolMode,
    /// 这个模型的**上下文窗口**（tokens）。缺省给保守值：宁可早压，也别撑爆。
    /// 自动压缩按它 × 设置的百分比触发（见 docs/session/session-model.md 六）。
    #[serde(default = "default_context_tokens")]
    pub context: u64,
}

/// 保守的上下文窗口缺省值（模型没声明、或登记处里查不到时用）。
pub const DEFAULT_CONTEXT_TOKENS: u64 = 32_000;

fn default_context_tokens() -> u64 {
    DEFAULT_CONTEXT_TOKENS
}

/// 目的：沙箱策略（全局硬约束，会话不可填）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SandboxSettings {
    #[serde(default)]
    pub policy: SandboxPolicy,
    #[serde(default)]
    pub defaults: crate::capabilities::permission::api::PermissionDefaults,
}

/// 目的：全局沙箱硬约束。会话不能覆盖这里的任何字段。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SandboxPolicy {
    /// 目的：是否允许工具 / guest / 常驻服务 / 构建出站。`false` = 断网；是否强制由 `enforce.network` 决定。
    #[serde(default)]
    pub network: bool,
    #[serde(default)]
    pub enforce: SandboxEnforce,
    /// 目的：机制不可用时的处置（`ask` 询问本进程降级；`refuse` 直接拒绝；没人答/没有前端 = 拒绝）。
    #[serde(default)]
    pub on_unavailable: SandboxUnavailable,
    #[serde(default)]
    pub windows: WindowsSandbox,
    /// 目的：用户显式授权的主机绝对路径只读根（默认空）。
    #[serde(default)]
    pub host_read_roots: Vec<String>,
    /// 目的：全局黑名单（工作区相对路径）——硬底线，会话只能追加、不能删除。
    #[serde(default)]
    pub deny: Vec<String>,
}

/// 目的：哪些围栏必须强制；`best_effort` 表示机制不可用时允许降级（但仍如实报告）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SandboxEnforce {
    #[serde(default)]
    pub filesystem: SandboxEnforceLevel,
    #[serde(default)]
    pub network: SandboxEnforceLevel,
    #[serde(default)]
    pub process_tree: SandboxEnforceLevel,
    #[serde(default)]
    pub env_whitelist: SandboxEnforceLevel,
}

/// 目的：一条围栏特性的要求等级。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SandboxEnforceLevel {
    /// 必须强制；不可用时按 `on_unavailable` 处置。
    #[default]
    Require,
    /// 尽力而为；不可用时降级并如实报告。
    BestEffort,
}

/// 目的：机制不可用时的处置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SandboxUnavailable {
    /// 询问用户是否本进程降级；没人答/没有前端 = 拒绝。
    #[default]
    Ask,
    /// 直接拒绝。
    Refuse,
}

/// 目的：Windows 容器围栏需要写本机目录 ACL 的授权。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct WindowsSandbox {
    #[serde(default)]
    pub write_acl: bool,
}

/// 基本设置（settings.yaml）：一般 agent 都有的开关。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppSettings {
    /// 流式传输：向供应商请求逐片返回。
    #[serde(default = "default_true")]
    pub streaming: bool,
    /// 思维链显示：开启后每条回答下的思维链块才出现（永远默认折叠，点击展开）。
    #[serde(default = "default_true")]
    pub show_reasoning: bool,
    /// 默认执行档位：新建会话未单独选定时用它（本机 = 在宿主上跑；虚拟机 = 整台 guest）。
    #[serde(default)]
    pub tier: crate::kernel::api::Tier,
    /// 全局沙箱策略与会话默认权限。
    #[serde(default)]
    pub sandbox: SandboxSettings,
    /// 虚拟机档用的 QEMU 可执行文件路径（默认空 = 兜底看 PATH，或由 setup 脚本装进项目内）。
    /// 依赖不预置、不分发；setup 脚本按平台下载进项目内，产品检测与指路。
    #[serde(default)]
    pub qemu_path: String,
    /// 目的：虚拟机档**共享后端**要用的外部工具路径（如 Linux 上的 virtiofsd；默认空 = 兜底看 PATH）。
    /// 约束：依赖不预置、不分发，由 provision 能力按清单装进项目内；不需要外部工具的共享档（9p / 复制）忽略它。
    #[serde(default)]
    pub virtiofsd_path: String,
    /// 目的：虚拟机档 **smb 共享档**要用的外部 helper 路径（用户态 SMB 服务；默认空 = 兜底看 PATH）。
    /// 约束：不预置、不分发，由 provision 能力按清单装进项目内；只有 smb 共享档用它。
    #[serde(default)]
    pub smbd_path: String,
    /// 目的：虚拟机档同时最多几台 guest（0 = 不限制）。
    /// 约束：每台固定 `-m 2048 -smp 2`；同时开太多会把宿主的 CPU / 内存吃光。默认 2，见 docs/vm/README.md 的「并发」。
    #[serde(default = "default_max_guests")]
    pub max_guests: u32,
    /// **单次模型调用的总预算（秒）**，全局通用（讨论、执行、验收、单 agent 都用它）。
    /// 默认给得比较宽：非流式下供应商要等整段生成完才发响应头，长回复本来就要几十秒；
    /// 预算用尽 = 中断这一轮并如实告知（用户可以点「继续」重试），不是把会话作废。
    #[serde(default = "default_llm_timeout_secs")]
    pub llm_timeout_secs: u64,
    /// **上下文用到多少就该压**（占模型窗口的百分比）。到点自动压一次；用户也可以手动 /compact。
    /// 默认 70：留三成余量给"这一轮还要生成的内容"，免得刚压完又爆。
    #[serde(default = "default_compact_percent")]
    pub compact_at_percent: u8,
    /// **一轮内对同一个成员最多提醒几次**（提醒 = 系统注入"你还没用讨论动词表态"）。
    /// 到顶就放它过去：主会话如实记一行"未回应"，整轮继续（不阻塞）。
    #[serde(default = "default_discuss_remind_cap")]
    pub discuss_remind_cap: u32,
}

fn default_discuss_remind_cap() -> u32 {
    3
}

/// 目的：max_guests 的默认值（2 台）。
fn default_max_guests() -> u32 {
    2
}

fn default_compact_percent() -> u8 {
    70
}

fn default_llm_timeout_secs() -> u64 {
    crate::kernel::api::DEFAULT_LLM_TIMEOUT_SECS
}

fn default_true() -> bool {
    true
}

impl Default for AppSettings {
    fn default() -> Self {
        AppSettings {
            streaming: true,
            show_reasoning: true,
            tier: crate::kernel::api::Tier::Host,
            sandbox: SandboxSettings::default(),
            qemu_path: String::new(),
            virtiofsd_path: String::new(),
            smbd_path: String::new(),
            max_guests: 2,
            llm_timeout_secs: crate::kernel::api::DEFAULT_LLM_TIMEOUT_SECS,
            compact_at_percent: 70,
            discuss_remind_cap: 3,
        }
    }
}

/// 目的：校验 settings.yaml 的沙箱段——路径、枚举与硬黑名单。
pub fn validate_sandbox_settings(s: &SandboxSettings) -> Result<(), String> {
    crate::capabilities::permission::api::validate_defaults(&s.defaults)?;
    crate::capabilities::permission::api::validate(&s.defaults.to_permissions(&s.policy.deny))?;
    for r in &s.policy.host_read_roots {
        let raw = r.trim();
        if raw.is_empty() {
            return Err("sandbox.policy.host_read_roots 里的路径不能为空".to_string());
        }
        if !std::path::Path::new(raw).is_absolute() {
            return Err(format!(
                "sandbox.policy.host_read_roots 必须是绝对路径：{}",
                raw
            ));
        }
    }
    Ok(())
}

/// 登记处（providers.yaml + models.yaml + settings.yaml + agents.yaml 的内存合体）。
#[derive(Debug, Default, Clone)]
pub struct Settings {
    pub providers: BTreeMap<String, Provider>,
    pub models: BTreeMap<String, ModelEntry>,
    /// 核心 AI 默认模型 id。
    pub core: Option<String>,
    /// 基本设置。
    pub app: AppSettings,
    /// 用户配置的具名 agent。
    pub agents: crate::capabilities::registry::domain::agents::Agents,
}

impl Settings {
    /// 模型 id → 成品通道；模型或供应商缺失 = 报错（不猜测回退）。
    pub fn resolve(&self, model_id: &str) -> Result<Channel, String> {
        let m = self
            .models
            .get(model_id)
            .ok_or_else(|| format!("无此模型：{}", model_id))?;
        let p = self
            .providers
            .get(&m.provider)
            .ok_or_else(|| format!("模型 {} 引用的供应商不存在：{}", model_id, m.provider))?;
        // 摊平成通道层的形态（见 `capabilities/llm/ports.rs` 的 `Channel`）：
        // 它是**解析结果**，不再嵌 `Provider`——嵌回去会让 `llm → registry` 成环。
        Ok(Channel {
            base_url: p.base_url.clone(),
            api_key: p.api_key.clone(),
            model: m.api_model.clone(),
        })
    }

    /// 某 agent 的工具调用形态：它自己的模型优先，其次核心默认；两者都没有 = envelope（兜底，任何供应商都能用）。
    pub fn tool_mode_for(&self, model: Option<&str>) -> ToolMode {
        model
            .or(self.core.as_deref())
            .and_then(|id| self.models.get(id))
            .map(|m| m.tools)
            .unwrap_or_default()
    }

    /// 核心 AI 默认通道；未设定或解析失败 = None（调用方回落演示并如实告知）。
    pub fn core_channel(&self) -> Option<Channel> {
        self.core.as_deref().and_then(|id| self.resolve(id).ok())
    }

    /// 供应商展示视图（永不携带密钥）。
    pub fn provider_views(&self) -> Vec<ProviderView> {
        self.providers
            .iter()
            .map(|(id, p)| ProviderView {
                id: id.clone(),
                base_url: p.base_url.clone(),
            })
            .collect()
    }

    /// 模型展示视图（含「是否核心默认」）。
    pub fn model_views(&self) -> Vec<ModelView> {
        self.models
            .iter()
            .map(|(id, m)| ModelView {
                id: id.clone(),
                name: m.name.clone(),
                api_model: m.api_model.clone(),
                provider: m.provider.clone(),
                note: m.note.clone(),
                tools: m.tools,
                context: m.context,
                is_core: self.core.as_deref() == Some(id.as_str()),
            })
            .collect()
    }
}

/// 供应商展示视图（不含密钥）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProviderView {
    pub id: String,
    pub base_url: String,
}

/// 模型展示视图。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelView {
    pub id: String,
    pub name: String,
    pub api_model: String,
    pub provider: String,
    pub note: String,
    /// 工具调用形态（envelope / native）——前端据此显示，也让用户知道当前走哪套协议。
    pub tools: ToolMode,
    /// 上下文窗口（tokens）：自动压缩按它 × 设置里的百分比触发；前端显示并可改。
    pub context: u64,
    pub is_core: bool,
}
