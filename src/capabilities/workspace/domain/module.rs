//! 模块打包契约（module.yaml）与扫描结果。
//! 目录遍历机制在本能力的 detail（ModuleSource 端口）；「清单即事实」的重扫策略由 service 执行。
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
    /// 对象：键值表这类结构（如会话编辑的定版表 `pins`）用它承载；成员形状由语义校验管。
    Object,
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
            ParamType::Object => "object",
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
            ParamType::Object => v.is_object(),
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
    /// 只声明能力名，不写版本——版本由用户在会话的执行档位里定（见 capabilities/workspace/domain/exec.rs 与 RUNTIME_SPEC.md）。
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

/// 外部工具表的校验（纯逻辑；扫描模块时由适配层调用）：**保留名由调用方给**
/// （工具名空间归工具能力，清单主人不反向依赖它——见 ARCHITECTURE.md §九.3）。
pub fn check_tools(m: &ModuleManifest, reserved: &[String]) -> Result<(), String> {
    for (name, decl) in &m.tools {
        if reserved.iter().any(|r| r == name) {
            return Err(format!(
                "tools 里的 {} 是核心内置工具名（保留名），模块不得占用",
                name
            ));
        }
        if decl.command.trim().is_empty() {
            return Err(format!("tools 里的 {} 没写 command（启动命令）", name));
        }
    }
    Ok(())
}

/// 一个已发现的模块 = 文件夹 + 清单。
#[derive(Debug, Clone)]
pub struct Module {
    pub manifest: ModuleManifest,
    pub root: PathBuf,
}

/// **身份块的系统提示**：把 `{{mechanism}}` / `{{env}}` / `{{tool_calling}}` 三件事按同一口径填进模板。
/// 模块只是能力包（没有"发言"这回事）；发言席是 agent，所以这份 system 按 agent 成文。
/// env 由 tools 的 systool 按该 agent 的沙箱渲染后传入。
/// **工具清单不在这里**：本回合能用哪些工具随回合注入（见 collab 的 engine::tools_block）。
pub fn agent_system(
    prompt: &dyn crate::capabilities::prompt::api::Prompt,
    agent: &str,
    modules: &[(String, String)],
    env: &str,
    mode: crate::capabilities::llm::api::ToolMode,
    session_kind: &str,
    role: &str,
) -> String {
    render_system(
        prompt,
        crate::capabilities::prompt::api::Segment::AgentSystem,
        agent,
        modules,
        env,
        mode,
        session_kind,
        role,
    )
}

/// **角色身份块**（不是 agent 的核心身份）：渲染角色提示词段 + 环境 + 调用约定。
/// 代理会话（core_proxy）用它：它的身份是角色提示词（`prompts/roles/core_proxy.yaml`），
/// 不是"某个 agent 的模块能力包"；机制 / 环境 / 调用约定与 agent 身份同一份口径。
#[allow(clippy::too_many_arguments)]
pub fn role_system(
    prompt: &dyn crate::capabilities::prompt::api::Prompt,
    segment: crate::capabilities::prompt::api::Segment,
    agent: &str,
    env: &str,
    mode: crate::capabilities::llm::api::ToolMode,
    session_kind: &str,
    role: &str,
) -> String {
    render_system(prompt, segment, agent, &[], env, mode, session_kind, role)
}

/// 两处身份（agent / 角色）**共用这一份装配**：模板不同，变量与取值口径完全相同——
/// 各写一份必然漂移。
#[allow(clippy::too_many_arguments)]
fn render_system(
    prompt: &dyn crate::capabilities::prompt::api::Prompt,
    segment: crate::capabilities::prompt::api::Segment,
    agent: &str,
    modules: &[(String, String)],
    env: &str,
    mode: crate::capabilities::llm::api::ToolMode,
    session_kind: &str,
    role: &str,
) -> String {
    use crate::capabilities::prompt::api::Segment;
    let parts = modules
        .iter()
        .map(|(id, system)| format!("\n== {} ==\n{}", id, system.trim()))
        .collect::<Vec<_>>()
        .join("");
    // 机制说明**按（会话使用类型 × 角色）**从册子里取：对应关系是数据，这里只做匹配。
    //（AI 不知道机制就只会写散文。）
    let mechanism = prompt.mechanism(session_kind, role);
    prompt.render(
        segment,
        &[
            ("agent", agent.to_string()),
            ("modules", parts),
            ("mechanism", mechanism),
            ("env", env.to_string()),
            // 两套调用约定**互斥**：一个通道只用一套（同时教会让模型在正文里讲解参数而被误判成调用）
            (
                "tool_calling",
                match mode {
                    crate::capabilities::llm::api::ToolMode::Native => {
                        prompt.text(Segment::ToolCallingNative).to_string()
                    }
                    crate::capabilities::llm::api::ToolMode::Envelope => {
                        prompt.text(Segment::ToolCallingEnvelope).to_string()
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
