//! 目的：登记处的值形态——供应商、模型、思考档位与视图；脱离 IO、用例与其它单元。
//! 管：Provider / ModelEntry / ModelDraft / ToolMode / ThinkingLevel / ThinkingCatalog / ThinkingSpec / StorageSettings / 三张视图与思考目录校验。
//! 不管：基本设置与沙箱策略、通道解析与校验（在 catalog 的 domain/providers.rs）；yaml 落盘（在 catalog 的 detail/yaml_settings.rs）。
//! 联动：catalog 的 api 与 service、turn / runtime / 呈现层都经 registry_model::api 取供应商与模型的形状。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 目的：该通道的工具调用形态（登记处里的事实；**缺省 native** = 现代供应商普遍支持）。
/// 约束：手写信封只给模型表单里**显式选择**的通道（保存前不实测）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ToolMode {
    /// 手写信封：模型在正文里写 {"type":"tool",…}，核心解析。
    Envelope,
    /// 原生工具调用：参数走供应商的结构化槽位（保存前实测通过才允许落盘）。
    #[default]
    Native,
}

/// 目的：思考参数片段**不得覆盖**的请求字段：这几串在主流协议里都承载请求结构本身（模型、消息、流开关、工具声明），
///    被片段顺手盖掉会直接把请求弄坏。取**各家并集**的保守集合：多挡一个不会出错，少挡一个就是线上事故。
/// 约束：登记处保存模型时挡一次（给用户明确报错），适配层合并前再挡一次（防绕过）。
pub const RESERVED_REQUEST_KEYS: [&str; 6] = [
    "model",
    "messages",
    "stream",
    "stream_options",
    "tools",
    "tool_choice",
];
/// 目的：一条供应商通道（端点 + 密钥）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub base_url: String,
    pub api_key: String,
}

/// 目的：一条可用模型：展示名 + 实际模型串 + 所属供应商 + 能力说明 + 工具调用形态。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    pub name: String,
    pub api_model: String,
    pub provider: String,
    #[serde(default)]
    pub note: String,
    /// 目的：工具调用形态（缺省 native；原生形态在保存前实测通过才允许落盘，见 REGISTRY_SPEC）。
    #[serde(default)]
    pub tools: ToolMode,
    /// 目的：这个模型的**上下文窗口**（tokens）。缺省给保守值：宁可早压，也别撑爆。
    ///    自动压缩按它 × 设置的百分比触发（见 docs/session/session-model.md 六）。
    #[serde(default = "default_context_tokens")]
    pub context: u64,
    /// 目的：思考强度：这个模型支持哪些档、每档发什么请求体片段。
    /// 约束：缺省 = 不发任何思考参数（随供应商默认）；片段不得覆盖请求结构字段（见 RESERVED_REQUEST_KEYS）。
    #[serde(default, skip_serializing_if = "ThinkingSpec::is_empty")]
    pub thinking: ThinkingSpec,
}

/// 目的：上下文窗口的兜底值（模型没声明、或用户登记 0「不填」时，自动压缩按它算）。
///    取 128K：宁可早压，也别等供应商直接撑爆上下文。
pub const DEFAULT_CONTEXT_TOKENS: u64 = 128_000;

fn default_context_tokens() -> u64 {
    DEFAULT_CONTEXT_TOKENS
}

/// 目的：一档思考强度（全局目录里的一项）：`id` 是标签，也是模型映射与会话选择的键。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// 目的：工具调用形态；原生形态在保存前实测通过才允许落盘。
    pub tools: ToolMode,
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
/// 目的：可回收缓存的空间预算与保留策略（settings.yaml 的 storage 段）。
/// 约束：上限只约束缓存，不约束会话 / 虚拟根 / 模块产物；0 有各自语义（见 REGISTRY_SPEC）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StorageSettings {
    /// 目的：启用全局周期回收；构建前 headroom 始终启用，不受它控制。
    pub periodic_gc: bool,
    /// 目的：全局自动回收间隔（分钟）；至少 15，默认 15。
    pub gc_interval_minutes: u64,
    /// 目的：可回收缓存总预算（GiB）；0 = 按磁盘 10% 自动，夹在 10–50 GiB。
    pub cache_cap_gib: u64,
    /// 目的：超过这么久未使用可回收（天）；0 = 不按龄。
    pub cache_keep_days: u64,
    /// 目的：新构建缓存卷的虚拟大小（GiB）；现有卷不自动缩。
    pub build_volume_gib: u64,
    /// 目的：宿主剩余低于它时先回收；回收后仍低才拒绝构建。
    pub min_free_gib: u64,
    /// 目的：日志保留天数（0 = 不按龄；仍保留最近若干份）。
    pub log_keep_days: u64,
}

impl Default for StorageSettings {
    fn default() -> Self {
        StorageSettings {
            periodic_gc: true,
            gc_interval_minutes: 15,
            cache_cap_gib: 20,
            cache_keep_days: 30,
            build_volume_gib: 16,
            min_free_gib: 2,
            log_keep_days: 14,
        }
    }
}
/// 目的：供应商展示视图（不含密钥）。
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
    /// 目的：工具调用形态（envelope / native）——前端据此显示，也让用户知道当前走哪套协议。
    pub tools: ToolMode,
    /// 目的：上下文窗口（tokens）：自动压缩按它 × 设置里的百分比触发；前端显示并可改。
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
