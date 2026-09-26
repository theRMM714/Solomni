//! 模块打包契约（module.yaml）与扫描结果。
//! 目录遍历机制在 adapters（ModuleSource 端口）；「清单即事实」的重扫策略由 core 执行。
//! 模型选择是会话级决定（记录在会话里），模块清单不再承载模型/供应商偏好。

use serde::Deserialize;
/// 参数类型（只支持机器能判定的最小集合；不猜、不做隐式转换）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamType {
    String,
    Integer,
    Number,
    Boolean,
    /// 数组：核心操作的**结构化载荷**（节点表 / 结论表 / 验收清单 / 名单）用它承载。
    /// 为什么需要：这种嵌套结构标量类型表达不了；它照样是一次**工具调用**（名字、存在性、
    /// 载荷形状都校验，且进工具台账），语义校验（负责人在不在名单、依赖成不成环）由代码在做完调用后照旧执行。
    Array,
}

impl ParamType {
    /// 模型侧与 JSON Schema 共用的类型名。
    pub fn name(self) -> &'static str {
        match self {
            ParamType::String => "string",
            ParamType::Integer => "integer",
            ParamType::Number => "number",
            ParamType::Boolean => "boolean",
            ParamType::Array => "array",
        }
    }

    /// 该值是否属于这个类型（整数与数字分开判定，不做 1 == 1.0 的宽容）。
    pub fn accepts(self, v: &serde_json::Value) -> bool {
        match self {
            ParamType::String => v.is_string(),
            ParamType::Integer => v.is_i64() || v.is_u64(),
            ParamType::Number => v.is_number(),
            ParamType::Boolean => v.is_boolean(),
            // 结构化载荷一律是数组（标量走 string/integer 那几种）。
            ParamType::Array => v.is_array(),
        }
    }
}

/// 一个参数的声明。
#[derive(Debug, Clone, Deserialize)]
pub struct Param {
    /// YAML 里的 `type`。
    #[serde(rename = "type")]
    pub ty: ParamType,
    /// 必填（缺省 false）。
    #[serde(default)]
    pub required: bool,
    /// 字符串参数不允许是空串（缺省 false）。
    #[serde(default)]
    pub non_empty: bool,
    /// 给模型看的一句话说明。
    #[serde(default)]
    pub desc: String,
    /// 缺省值（模型不写时用；也写进模型侧说明）。
    #[serde(default)]
    pub default: Option<serde_json::Value>,
    /// 数值下界（含）。
    #[serde(default)]
    pub min: Option<f64>,
    /// 数值上界（含）。
    #[serde(default)]
    pub max: Option<f64>,
}

use std::collections::BTreeMap;
use std::path::PathBuf;

/// module.yaml —— 打包契约。模块对世界的全部自我介绍。
#[derive(Debug, Clone, Deserialize)]
pub struct ModuleManifest {
    pub id: String,
    pub brief: String,
    pub system: String,
    /// 运行能力声明：本模块的工具需要哪些运行包能力（如 python / node / bash / c-c++）。
    /// 只声明能力名，不写版本——版本由用户在会话的执行档位里定（见 core/exec.rs 与 RUNTIME_SPEC.md）。
    #[serde(default)]
    pub runtimes: Vec<String>,
    /// 外部工具表：工具名 → 该工具的声明（启动命令 + 可选的参数契约）。
    /// 内置工具名（read / write / search）为保留名，模块不得占用（见 check_tools）。
    #[serde(default)]
    pub tools: BTreeMap<String, ToolDecl>,
}

/// 一个模块工具：模块作者声明「怎么启动它」以及「它吃什么参数」。
/// 参数契约**可选**：不写就照旧不校验、也不写进提示词；写了就由核心按它校验，并把说明写进系统提示。
#[derive(Debug, Clone, Deserialize)]
pub struct ToolDecl {
    /// 启动命令（相对模块根写，例如 python tools/x.py）；工作目录 = 该模块的根目录。
    pub command: String,
    /// 给模型看的一句话说明（可选）。
    #[serde(default)]
    pub desc: String,
    /// 参数契约（可选）：参数名 → 声明。
    #[serde(default)]
    pub params: Option<BTreeMap<String, Param>>,
    /// 这个工具**可并发执行**（缺省 false = 独占串行）：只读、无副作用的工具才该声明 true，
    /// 同一回复里的多个可并发调用会真的并发跑（结果仍按调用顺序回填）。
    #[serde(default)]
    pub parallel: bool,
}

/// 运行能力声明的校验（纯逻辑；扫描模块时由适配层调用）：非法或重复 = 拒收并说明原因，不纠正。
pub fn check_runtimes(m: &ModuleManifest) -> Result<(), String> {
    let mut seen: Vec<&String> = Vec::new();
    for c in &m.runtimes {
        if !crate::capabilities::workspace::domain::packages::valid_capability(c) {
            return Err(format!(
                "runtimes 里的能力名不合法：{}（只允许小写字母、数字、- _ .，且以字母或数字开头）",
                c
            ));
        }
        if seen.contains(&c) {
            return Err(format!("runtimes 里重复声明了：{}", c));
        }
        seen.push(c);
    }
    Ok(())
}

/// 一个已发现的模块 = 文件夹 + 清单。
#[derive(Debug, Clone)]
pub struct Module {
    pub manifest: ModuleManifest,
    pub root: PathBuf,
}

/// 一个 agent 的职责提示词：把它的模块 system 合成一份能力包，再挂工作环境与调用约定。
/// 模块只是能力包（没有"发言"这回事）；发言席是 agent，所以这份 system 按 agent 成文。
/// env 由 core::systool 按该 agent 的沙箱渲染后传入。
/// **工具清单不在这里**：本回合能用哪些工具随回合注入（见 core::engine::tools_block）。
pub fn agent_system(
    prompts: &crate::capabilities::prompt::api::Prompts,
    agent: &str,
    modules: &[(String, String)],
    env: &str,
    mode: crate::capabilities::llm::api::ToolMode,
) -> String {
    let parts = modules
        .iter()
        .map(|(id, system)| format!("\n== {} ==\n{}", id, system.trim()))
        .collect::<Vec<_>>()
        .join("");
    prompts.render(
        &prompts.core.agent.system,
        &[
            ("agent", agent.to_string()),
            ("modules", parts),
            // 机制说明：AI 不知道机制就只会写散文（真机上就是这样空转的）。
            ("mechanism", prompts.core.mechanism.clone()),
            ("env", env.to_string()),
            // 两套调用约定**互斥**：一个通道只用一套（同时教会让模型在正文里讲解参数而被误判成调用）
            (
                "tool_calling",
                match mode {
                    crate::capabilities::llm::api::ToolMode::Native => {
                        prompts.core.tool_calling_native.clone()
                    }
                    crate::capabilities::llm::api::ToolMode::Envelope => {
                        prompts.core.tool_calling_envelope.clone()
                    }
                },
            ),
        ],
    )
}

/// 扫描结果：合法模块 + 拒收原因（校验，不是挑选——如实呈现）。
pub struct Roster {
    pub modules: Vec<Module>,
    pub rejected: Vec<String>,
}

/// 模块公地清单（拟名单时给模型看）：id / 简述。
pub fn listing(roster: &Roster, texts: &crate::capabilities::prompt::api::ToolTexts) -> String {
    roster
        .modules
        .iter()
        .map(|m| {
            texts.render(
                &texts.module_listing_line,
                &[
                    ("id", m.manifest.id.clone()),
                    ("brief", m.manifest.brief.clone()),
                ],
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
