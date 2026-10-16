//! 目的：模块打包契约（`module.yaml`）与清单的纯逻辑。
//! 管：契约类型（`ModuleManifest` / `ToolDecl` / `ServiceDecl` / `SecretDecl` / `Param`）、校验、`listing`。
//! 不管：目录遍历（在 `detail/`）；身份块渲染（归 `session`）；模型/供应商偏好（会话级决定）。
//! 联动：契约文档见 MODULE_SPEC.md；由 `detail/fs_modules.rs` 调校验、`api.rs` 对外重导出。

use serde::Deserialize;
/// 目的：参数类型（只支持机器能判定的最小集合；不猜、不做隐式转换）。
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
    /// 目的：模型侧与 JSON Schema 共用的类型名。
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

    /// 目的：该值是否属于这个类型（整数与数字分开判定，不做 1 == 1.0 的宽容）。
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

/// 目的：一个参数的声明。
#[derive(Debug, Clone, Deserialize)]
pub struct Param {
    /// 目的：YAML 里的 `type`。
    #[serde(rename = "type")]
    pub ty: ParamType,
    /// 目的：必填（缺省 false）。
    #[serde(default)]
    pub required: bool,
    /// 目的：字符串参数不允许是空串（缺省 false）。
    #[serde(default)]
    pub non_empty: bool,
    /// 目的：给模型看的一句话说明。
    #[serde(default)]
    pub desc: String,
    /// 目的：缺省值（模型不写时用；也写进模型侧说明）。
    #[serde(default)]
    pub default: Option<serde_json::Value>,
    /// 目的：数值下界（含）。
    #[serde(default)]
    pub min: Option<f64>,
    /// 目的：数值上界（含）。
    #[serde(default)]
    pub max: Option<f64>,
}

use std::collections::BTreeMap;
use std::path::PathBuf;

/// 目的：module.yaml —— 打包契约。模块对世界的全部自我介绍。
#[derive(Debug, Clone, Deserialize)]
pub struct ModuleManifest {
    pub id: String,
    pub brief: String,
    pub system: String,
    /// 目的：作者默认的模块启用状态（缺省 true；用户覆盖落 `.home/modules.yaml`）。
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 目的：运行能力声明：本模块的工具需要哪些运行包能力（如 python / node / bash / c-c++）。
    ///   只声明能力名，不写版本——版本由用户在会话的执行档位里定（见 capabilities/workspace/domain/exec.rs 与 RUNTIME_SPEC.md）。
    #[serde(default)]
    pub runtimes: Vec<String>,
    /// 目的：外部工具表：工具名 → 该工具的声明（启动命令 + 可选的参数契约）。
    ///   内置工具名（read / write / search）为保留名，模块不得占用（见 check_tools）。
    #[serde(default)]
    pub tools: BTreeMap<String, ToolDecl>,
    /// 目的：常驻服务表：服务名 → 该服务的声明（用哪个适配器 + 启动命令 + 是否默认启用）。
    ///   适配器名只在组合根注册表里匹配；核心不解释它的含义（见 check_services）。
    #[serde(default)]
    pub services: BTreeMap<String, ServiceDecl>,
    /// 目的：隐秘字段表：字段名 → 它注入该模块进程的环境变量名。**只声明标识，永不声明值**（见 check_secrets）。
    #[serde(default)]
    pub secrets: BTreeMap<String, SecretDecl>,
}

/// 目的：一个常驻服务：模块作者声明「用哪个适配器拉起它、怎么启动、默认开不开」。
///   协议语义全在适配器里（capabilities/residents）；这里只有文本声明，没有执行。
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)] // 字段由 residents / 呈现层 / 适配器读取；本能力只做声明形态与语法校验
pub struct ServiceDecl {
    /// 目的：组合根注册的适配器名（核心只做注册表匹配，未知 = 拒收并列出可用名）。
    pub adapter: String,
    /// 目的：启动命令（相对模块根写；工作目录 = 该模块的根目录）。
    pub command: String,
    /// 目的：给模型/用户看的一句话说明（可选）。
    #[serde(default)]
    pub desc: String,
    /// 目的：是否默认启用（缺省 false：没经用户同意不起外部常驻进程）。
    #[serde(default)]
    pub enabled: bool,
    /// 目的：只给该适配器读的私有配置（核心不解释、不校验；键值都是字符串）。
    #[serde(default)]
    pub options: BTreeMap<String, String>,
}

/// 目的：一个隐秘字段：模块声明「我需要哪个隐秘字段、注入成哪个环境变量」——**只声明标识，不声明值**。
///   值由用户经设置配置、只存 `.home/`，由 `capabilities/secrets` 按模块作用域注入。
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)] // 字段由 secrets / 呈现层读取；本能力只做声明形态与语法校验
pub struct SecretDecl {
    /// 目的：注入该模块工具/服务进程的环境变量名（必填、非空）。
    pub env: String,
    /// 目的：给人/模型看的一句话说明（可选）。
    #[serde(default)]
    pub desc: String,
}

/// 目的：一个模块工具：模块作者声明「怎么启动它」以及「它吃什么参数」。
///   参数契约**可选**：不写就照旧不校验、也不写进提示词；写了就由核心按它校验，并把说明写进系统提示。
#[derive(Debug, Clone, Deserialize)]
pub struct ToolDecl {
    /// 目的：启动命令（相对模块根写，例如 python tools/x.py）；工作目录 = 该模块的根目录。
    pub command: String,
    /// 目的：给模型看的一句话说明（可选）。
    #[serde(default)]
    pub desc: String,
    /// 目的：参数契约（可选）：参数名 → 声明。
    #[serde(default)]
    pub params: Option<BTreeMap<String, Param>>,
    /// 目的：这个工具**可并发执行**（缺省 false = 独占串行）：只读、无副作用的工具才该声明 true，
    ///   同一回复里的多个可并发调用会真的并发跑（结果仍按调用顺序回填）。
    #[serde(default)]
    pub parallel: bool,
}

/// 目的：运行能力声明的校验（纯逻辑；扫描模块时由适配层调用）：非法或重复 = 拒收并说明原因，不纠正。
pub fn check_runtimes(m: &ModuleManifest) -> Result<(), String> {
    let mut seen: Vec<&String> = Vec::new();
    for c in &m.runtimes {
        if !crate::kernel::api::valid_capability(c) {
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

/// 目的：外部工具表的校验（纯逻辑；扫描模块时由适配层调用）：**保留名由调用方给**
///   （工具名空间归工具能力，清单主人不反向依赖它——见 ARCHITECTURE.md §九.3）。
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

/// 目的：常驻服务表的校验（纯逻辑；扫描模块时由适配层调用）：**只校验语法**。
///   适配器名是否存在由 `residents` 在使用时按注册表判——模块能力不认识适配器。
pub fn check_services(m: &ModuleManifest) -> Result<(), String> {
    for (name, decl) in &m.services {
        if name.trim().is_empty() {
            return Err("services 里的服务名为空".to_string());
        }
        if decl.adapter.trim().is_empty() {
            return Err(format!(
                "services 里的 {} 没写 adapter（用哪个适配器）",
                name
            ));
        }
        if decl.command.trim().is_empty() {
            return Err(format!("services 里的 {} 没写 command（启动命令）", name));
        }
    }
    Ok(())
}

/// 目的：隐秘字段表的校验（纯逻辑；扫描模块时由适配层调用）：字段名与注入的 env 名都不得为空。
pub fn check_secrets(m: &ModuleManifest) -> Result<(), String> {
    for (name, decl) in &m.secrets {
        if name.trim().is_empty() {
            return Err("secrets 里的字段名为空".to_string());
        }
        if decl.env.trim().is_empty() {
            return Err(format!(
                "secrets 里的 {} 没写 env（注入的环境变量名）",
                name
            ));
        }
    }
    Ok(())
}

/// 目的：一个已发现的模块 = 文件夹 + 清单。
#[derive(Debug, Clone)]
pub struct Module {
    pub manifest: ModuleManifest,
    pub root: PathBuf,
    /// 目的：模块根下有没有 userdata/ 目录（工作区扫描读出的事实；策略层据此决定派不派它）。
    pub has_userdata: bool,
}

/// 目的：工具命令的首个 token 若是**模块内的产物入口**（相对路径）就给出它；裸名与绝对路径给 None。
/// 参数：command = 模块声明的启动命令（工作目录 = 模块根）。
/// 约束：约定"模块内的产物用相对路径写"（如 `build/indexer`）——裸名走 PATH / 运行能力，绝对路径不在模块目录里找。
pub fn local_entry(command: &str) -> Option<&str> {
    let first = command.split_whitespace().next()?;
    let looks_like_path = first.contains('/') || first.contains('\\');
    let absolute = first.starts_with('/') || first.starts_with('\\') || first.contains(":\\");
    if looks_like_path && !absolute {
        Some(first)
    } else {
        None
    }
}

/// 目的：扫描结果：合法模块 + 拒收原因（校验，不是挑选——如实呈现）。
pub struct Roster {
    pub modules: Vec<Module>,
    pub rejected: Vec<String>,
}

/// 目的：模块公地清单（拟名单时给模型看）：id / 简述。
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

/// 目的：布尔字段的作者默认（缺省 true）。
fn default_true() -> bool {
    true
}
