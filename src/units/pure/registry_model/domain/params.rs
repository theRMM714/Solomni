//! 目的：参数声明的形状——模块工具、系统工具、命令、会话类型与沙箱策略共用的一种声明。
//! 管：`ParamType`（机器能判定的最小类型集合）与 `Param`（一个参数的声明）。
//! 不管：谁声明、声明在哪个文件（各自的单元）；文本入口怎么解析（呈现层）；JSON Schema 怎么渲染（tools）。
//! 联动：`catalog::modules`、`tools`、`presentation::cli` 与 `sandbox` 经本单元的 `api` 取。

use serde::{Deserialize, Serialize};

/// 目的：参数类型（只支持机器能判定的最小集合；不猜、不做隐式转换）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
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
#[derive(Debug, Clone, Deserialize, Serialize)]
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
    /// 目的：位置参数序号（1 起）。声明了才允许不写 `--名字`：命令行按这个序号取位置参数。
    ///    约束：只给呈现层的文本入口用；模型侧工具调用永远传具名 JSON，不受它影响。
    #[serde(default)]
    pub positional: Option<u32>,
    /// 目的：这个位置参数**吃光剩下的全部位置参数**（如 `single <agent名…>`）。
    ///    约束：只允许出现在序号最大的那个位置参数上，且一个声明里至多一个。
    #[serde(default)]
    pub rest: bool,
    /// 目的：可选值清单（枚举）：值只能是其中一项；写进模型侧说明与 JSON Schema 的 enum。
    #[serde(default)]
    pub values: Option<Vec<String>>,
    /// 目的：文本入口里可以**光写旗语不给值**（值收成空串，如 `--ensure` = 全量补齐）。
    ///    约束：只对非布尔参数有意义；模型侧工具调用永远传具名 JSON，不受它影响。
    #[serde(default)]
    pub optional_value: bool,
}
