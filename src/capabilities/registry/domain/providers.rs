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
    /// 目的：思考强度：这个模型支持哪些档、每档发什么请求体片段。
    /// 约束：缺省 = 不发任何思考参数（随供应商默认）；片段不得覆盖协议字段（见 THINKING_RESERVED_KEYS）。
    #[serde(default, skip_serializing_if = "ThinkingSpec::is_empty")]
    pub thinking: ThinkingSpec,
}

/// 上下文窗口的兜底值（模型没声明、或用户登记 0「不填」时，自动压缩按它算）。
/// 取 128K：宁可早压，也别等供应商直接撑爆上下文。
pub const DEFAULT_CONTEXT_TOKENS: u64 = 128_000;

fn default_context_tokens() -> u64 {
    DEFAULT_CONTEXT_TOKENS
}

/// 目的：一档思考强度（全局目录里的一项）：`id` 是标签，也是模型映射与会话选择的键。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThinkingLevel {
    /// 目的：档位标签（界面显示、模型映射与会话选择都用这一串）。
    pub id: String,
}

/// 目的：思考强度目录（settings.yaml 的 `thinking` 段）：顺序就是界面顺序。
/// 约束：模型映射只能引用这里的档位；删档前先清掉引用它的模型映射。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThinkingCatalog {
    /// 目的：档位目录（顺序即界面顺序）。
    #[serde(default = "default_thinking_levels")]
    pub levels: Vec<ThinkingLevel>,
}

/// 目的：默认四档 off / low / high / max（与 DeepSeek 的线上取值同名；各模型再映射成自己的写法）。
pub fn default_thinking_levels() -> Vec<ThinkingLevel> {
    vec![
        ThinkingLevel {
            id: "off".to_string(),
        },
        ThinkingLevel {
            id: "low".to_string(),
        },
        ThinkingLevel {
            id: "high".to_string(),
        },
        ThinkingLevel {
            id: "max".to_string(),
        },
    ]
}

impl Default for ThinkingCatalog {
    fn default() -> Self {
        ThinkingCatalog {
            levels: default_thinking_levels(),
        }
    }
}

/// 目的：一个模型的思考强度登记：默认档 + 档位 → 请求体片段。
/// 约束：值是字符串（简写 `{"reasoning_effort": 值}`）或对象（原样合并）；空对象 = 这一档不发字段。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ThinkingSpec {
    /// 目的：默认档位（None = 不指定，随供应商默认）。
    #[serde(default)]
    pub default: Option<String>,
    /// 目的：档位 → 请求体片段（字符串 = reasoning_effort 简写；对象 = 原样合并；空对象 = 不发字段）。
    #[serde(default)]
    pub levels: BTreeMap<String, serde_json::Value>,
}

impl ThinkingSpec {
    /// 目的：没登记任何思考信息（落盘时省略整段，旧文件不会平白多出空结构）。
    pub fn is_empty(&self) -> bool {
        self.default
            .as_deref()
            .map(str::trim)
            .map(str::is_empty)
            .unwrap_or(true)
            && self.levels.is_empty()
    }
}

/// 目的：登记 / 更新一个模型的全部字段（写面收口，调用方不逐个传 7 个参数）。
#[derive(Debug, Clone)]
pub struct ModelDraft {
    /// 目的：模型 id（登记处的键）。
    pub id: String,
    /// 目的：展示名（给人看）。
    pub name: String,
    /// 目的：实际模型串（发给供应商）。
    pub api_model: String,
    /// 目的：所属供应商 id。
    pub provider: String,
    /// 目的：能力说明（给人看）。
    pub note: String,
    /// 目的：上下文窗口（tokens）；0 = 用户不填（自动压缩按 DEFAULT_CONTEXT_TOKENS 兜底）。
    pub context: u64,
    /// 目的：思考强度（默认档 + 档位映射）。
    pub thinking: ThinkingSpec,
}

/// 目的：校验目录——标签非空、唯一、无首尾空白、无控制字符；顺序即界面顺序。
pub fn validate_thinking_catalog(c: &ThinkingCatalog) -> Result<(), String> {
    if c.levels.is_empty() {
        return Err("thinking.levels 不能为空".to_string());
    }
    let mut seen = std::collections::BTreeSet::new();
    for l in &c.levels {
        let id = l.id.trim();
        if id.is_empty() {
            return Err("thinking.levels 里的标签不能为空".to_string());
        }
        if id != l.id {
            return Err(format!("thinking.levels 的标签首尾不能有空白：{:?}", l.id));
        }
        if id.chars().any(char::is_control) {
            return Err(format!("thinking.levels 的标签不能包含控制字符：{:?}", id));
        }
        if !seen.insert(id.to_string()) {
            return Err(format!("thinking.levels 的标签重复：{}", id));
        }
    }
    Ok(())
}

/// 目的：把登记值归一成请求体片段。字符串 = reasoning_effort 简写；对象 = 原样合并；空对象 = 不发字段。
pub fn thinking_fragment(raw: &serde_json::Value) -> Result<serde_json::Value, String> {
    match raw {
        serde_json::Value::String(s) => {
            if s.trim().is_empty() {
                return Err("空字符串不是合法的 reasoning_effort 值".to_string());
            }
            Ok(serde_json::json!({ "reasoning_effort": s }))
        }
        serde_json::Value::Object(o) => {
            for k in o.keys() {
                if crate::capabilities::llm::api::REQUEST_RESERVED_KEYS.contains(&k.as_str()) {
                    return Err(format!("不能覆盖请求协议字段：{}", k));
                }
            }
            Ok(serde_json::Value::Object(o.clone()))
        }
        _ => Err("只能是字符串（reasoning_effort 简写）或 JSON 对象".to_string()),
    }
}

/// 目的：校验一个模型的思考登记：档位必须在目录里、默认档必须被支持、每个片段形状合法。
pub fn validate_thinking_spec(
    spec: &ThinkingSpec,
    catalog: &ThinkingCatalog,
) -> Result<(), String> {
    let ids: Vec<&str> = catalog.levels.iter().map(|l| l.id.as_str()).collect();
    for (level, raw) in &spec.levels {
        if !ids.contains(&level.as_str()) {
            return Err(format!(
                "思考档位 {} 不在全局目录里（现有：{}）",
                level,
                ids.join(" / ")
            ));
        }
        thinking_fragment(raw).map_err(|e| format!("思考档位 {} 的映射非法：{}", level, e))?;
    }
    if let Some(d) = spec.default.as_deref() {
        if d.trim().is_empty() {
            return Err("思考默认档位不能是空串（不指定请省略该字段）".to_string());
        }
        if !spec.levels.contains_key(d) {
            return Err(format!(
                "思考默认档位 {} 不在这个模型支持的档位里（支持：{}）",
                d,
                spec.levels.keys().cloned().collect::<Vec<_>>().join(" / ")
            ));
        }
    }
    Ok(())
}

/// 目的：把目录与所有模型映射放在一起校验（删档 / 改档后旧映射不能悬空）。
pub fn validate_thinking(
    models: &BTreeMap<String, ModelEntry>,
    catalog: &ThinkingCatalog,
) -> Result<(), String> {
    validate_thinking_catalog(catalog)?;
    for (id, m) in models {
        validate_thinking_spec(&m.thinking, catalog).map_err(|e| format!("模型 {}：{}", id, e))?;
    }
    Ok(())
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
    /// 目的：思考强度目录（全局）：默认 off / low / high / max，用户可追加。
    /// 约束：模型映射只能引用这里的档位；删掉仍被模型引用的档位会被拒绝。
    #[serde(default)]
    pub thinking: ThinkingCatalog,
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
            thinking: ThinkingCatalog::default(),
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
    for e in &s.policy.deny {
        crate::capabilities::permission::api::validate_entry(e)
            .map_err(|why| format!("sandbox.policy.deny：{}", why))?;
    }
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

/// 目的：Windows 上 `require` 文件系统/网络围栏与“不授权写 ACL”互斥——这组配置永远满足不了。
/// 约束：只在 Windows 上判定；Linux / macOS 的围栏在守门进程里自足，`write_acl` 不参与。
pub fn validate_sandbox_authorization(p: &SandboxPolicy) -> Result<(), String> {
    #[cfg(windows)]
    {
        let fs_required = p.enforce.filesystem == SandboxEnforceLevel::Require;
        let net_required = p.enforce.network == SandboxEnforceLevel::Require && !p.network;
        if (fs_required || net_required) && !p.windows.write_acl {
            return Err(
                "Windows 上 require 文件系统/网络围栏时必须同时授权 sandbox.policy.windows.write_acl，\
                 或把对应的 enforce 降为 best_effort"
                    .to_string(),
            );
        }
    }
    #[cfg(not(windows))]
    {
        let _ = p;
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

    /// 目的：核心 AI 默认通道；未设定或解析失败 = 如实报错（不回落、不猜）。
    pub fn core_channel(&self) -> Result<Channel, String> {
        match self.core.as_deref() {
            Some(id) => self
                .resolve(id)
                .map_err(|e| format!("核心默认模型 {} 无法解析：{}", id, e)),
            None if self.models.is_empty() => {
                Err("登记处还没有任何模型：请先登记供应商与模型".to_string())
            }
            None => Err(format!(
                "核心 AI 未设定默认模型（已登记 {} 个模型）：请到「模型与核心默认」设定",
                self.models.len()
            )),
        }
    }

    /// 目的：本次调用要用的通道；给了模型就用它，否则用核心默认，失败带具体原因。
    pub fn effective_channel(&self, model: Option<&str>) -> Result<Channel, String> {
        match model {
            Some(id) => self.resolve(id),
            None => self.core_channel(),
        }
    }

    /// 目的：该模型（缺省用核心默认）的上下文窗口；登记为 0 = 用户不填，按兜底值算。
    pub fn context_of(&self, model: Option<&str>) -> u64 {
        model
            .map(|s| s.to_string())
            .or_else(|| self.core.clone())
            .and_then(|id| self.models.get(&id).map(|m| m.context))
            .filter(|c| *c > 0)
            .unwrap_or(DEFAULT_CONTEXT_TOKENS)
    }

    /// 目的：这个模型（`None` = 核心默认）在给定会话档位下要并入请求体的片段；None = 不发任何思考参数。
    /// 约束：显式会话档位优先，其次模型的 `default`；两者都没有 = 不发；显式档位而模型没登记 = 如实报错（不静默降级）。
    pub fn thinking_fragment(
        &self,
        model: Option<&str>,
        level: Option<&str>,
    ) -> Result<Option<serde_json::Value>, String> {
        let id = match model.map(str::trim).filter(|m| !m.is_empty()) {
            Some(m) => m.to_string(),
            None => match self.core.clone() {
                Some(c) => c,
                None => return Ok(None),
            },
        };
        let Some(entry) = self.models.get(&id) else {
            return Ok(None);
        };
        let chosen = match level.map(str::trim).filter(|l| !l.is_empty()) {
            Some(l) => Some(l.to_string()),
            None => entry
                .thinking
                .default
                .clone()
                .filter(|d| !d.trim().is_empty()),
        };
        let Some(chosen) = chosen else {
            return Ok(None);
        };
        let raw = entry.thinking.levels.get(&chosen).ok_or_else(|| {
            let known = if entry.thinking.levels.is_empty() {
                "该模型未登记任何思考档位".to_string()
            } else {
                format!(
                    "已登记：{}",
                    entry
                        .thinking
                        .levels
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" / ")
                )
            };
            format!("模型 {} 未登记思考档位 {}（{}）", id, chosen, known)
        })?;
        let frag = thinking_fragment(raw)
            .map_err(|e| format!("模型 {} 的思考档位 {} 映射非法：{}", id, chosen, e))?;
        if frag.as_object().map(|o| o.is_empty()).unwrap_or(true) {
            Ok(None)
        } else {
            Ok(Some(frag))
        }
    }

    /// 目的：这个模型（`None` = 核心默认）支持这一档吗（会话切换前的可用性判据）。
    pub fn thinking_supports(&self, model: Option<&str>, level: &str) -> bool {
        let id = match model.map(str::trim).filter(|m| !m.is_empty()) {
            Some(m) => m.to_string(),
            None => match self.core.clone() {
                Some(c) => c,
                None => return false,
            },
        };
        self.models
            .get(&id)
            .map(|m| m.thinking.levels.contains_key(level))
            .unwrap_or(false)
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
                thinking: ModelThinkingView {
                    default: m.thinking.default.clone(),
                    levels: m.thinking.levels.clone(),
                },
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

/// 目的：模型展示视图。
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
    /// 目的：思考强度（默认档 + 档位 → 原始映射值，前端据此回填表单）。
    pub thinking: ModelThinkingView,
    pub is_core: bool,
}

/// 目的：模型视图里的思考强度事实（原样给前端；接口形状见 `ThinkingSpec`）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelThinkingView {
    /// 目的：默认档位（None = 不指定）。
    pub default: Option<String>,
    /// 目的：档位 → 原始映射值（字符串或对象）。
    pub levels: BTreeMap<String, serde_json::Value>,
}
