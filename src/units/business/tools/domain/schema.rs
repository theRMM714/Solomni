//! 目的：工具参数契约——**声明在文本层**，代码只解析 / 校验 / 渲染。
//! 管：内置工具的参数声明在 `systools/tools.yaml` 的 `tools`；模块工具的参数声明在 `module.yaml` 的 `tools.<名字>.params`（**可选**：不写就照旧工作）；
//!   同一份声明同时驱动模型侧说明（`render_for_prompt`）与调用校验（`check`），"缺少 path / limit 不能大于 2000"这类文案不再硬编码在代码里。
//! 不管：参数 schema 的硬编码——它是**模块与产品对模型的承诺**，要能像文案一样被看到、被修改、被复核。
//! 联动：由 `detail/yaml_systools.rs` 与模块清单读入，`service/` 使用。

use crate::units::pure::registry_model::api::{Param, ParamType};
use serde::Deserialize;
use std::collections::BTreeMap;

/// 目的：一个工具的参数契约。
#[derive(Debug, Clone, Deserialize)]
pub struct ToolSchema {
    /// 目的：给模型看的一句话说明（工具做什么）。
    #[serde(default)]
    pub desc: String,
    /// 目的：参数表（名字 → 声明）。
    ///    整个不写 = **不做参数校验**（模块工具没声明 schema 时的情形，保持照旧工作）；
    ///    写了但为空 = 这个工具不收任何参数。
    #[serde(default)]
    pub params: Option<BTreeMap<String, Param>>,
    /// 目的：这个工具**可并发执行**（缺省 false = 独占串行）。
    ///    只该给"只读、无副作用"的工具写：同一回复里的多个可并发调用会真的并发跑；
    ///    未声明的（含写入类）独占执行，并作为并发批次之间的屏障。
    #[serde(default)]
    pub parallel: bool,
    /// 目的：**只在呈现层可用**的参数名（人经界面或文本入口给的）。
    ///    模型面（提示词说明、原生 JSON Schema、调用校验、缺省值）一律看不到也收不到这些参数，
    ///    给了就是明确拒绝（`ArgFault::HumanOnly`）——它们表达"只有人能做的选择"（如会话执行环境），
    ///    模型越权暗示自己选只会得到一条静默继承的会话，比报错更难查。
    #[serde(default)]
    pub human: Vec<String>,
}

impl ToolSchema {
    /// 声明了的参数表；没声明 = 调用方不做校验。
    fn declared(&self) -> Option<&BTreeMap<String, Param>> {
        self.params.as_ref()
    }

    /// 目的：这个参数是不是**只给人用**（模型面不出现、也不收）。
    fn human_only(&self, name: &str) -> bool {
        self.human.iter().any(|h| h == name)
    }
}

/// 目的：一本书里的全部工具契约（key = 工具名）。内置工具与模块工具都用它。
pub type ToolBook = BTreeMap<String, ToolSchema>;

impl ToolSchema {
    /// 目的：模型侧的参数说明：每个参数一行，带类型、必填/缺省与说明。
    /// 约束：`human` 声明的人用参数不发（模型看不到，也收不到，见 `check_model`）。
    pub fn render_for_prompt(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        if !self.desc.trim().is_empty() {
            lines.push(self.desc.trim().to_string());
        }
        for (name, p) in self.params.iter().flatten() {
            if self.human_only(name) {
                continue;
            }
            let mut marks: Vec<String> = vec![p.ty.name().to_string()];
            if p.required {
                marks.push("必填".to_string());
            }
            if let Some(d) = &p.default {
                marks.push(format!("缺省 {}", compact(d)));
            }
            if let Some(lo) = p.min {
                marks.push(format!("不小于 {}", compact_num(lo)));
            }
            if let Some(hi) = p.max {
                marks.push(format!("不大于 {}", compact_num(hi)));
            }
            if let Some(vals) = &p.values {
                marks.push(format!("可选 {}", vals.join(" / ")));
            }
            let desc = p.desc.trim();
            lines.push(if desc.is_empty() {
                format!("- {}（{}）", name, marks.join("，"))
            } else {
                format!("- {}（{}）：{}", name, marks.join("，"), desc)
            });
        }
        lines.join("\n")
    }

    /// 目的：JSON Schema 形态（同一声明的第二形态：附加属性一律拒收，与主流一致）。
    ///    原生工具调用通道由 decl() 取它渲染工具声明；手写信封通道不发它（模型看的是提示词里的参数说明）。
    /// 约束：这是**模型面**的声明，`human` 参数不进 properties 也不进 required。
    pub fn to_json_schema(&self) -> serde_json::Value {
        let mut props = serde_json::Map::new();
        let mut required: Vec<serde_json::Value> = Vec::new();
        for (name, p) in self.params.iter().flatten() {
            if self.human_only(name) {
                continue;
            }
            let mut node = serde_json::Map::new();
            node.insert(
                "type".to_string(),
                serde_json::Value::String(p.ty.name().to_string()),
            );
            // 数组参数带 items：JSON Schema 里数组成员形状由它给，缺了会被有些供应商按 schema 非法拒掉。
            // 成员形状写成空 schema（不限制）——真正的形状校验按语义在做完调用后做。
            if p.ty == ParamType::Array {
                node.insert(
                    "items".to_string(),
                    serde_json::Value::Object(serde_json::Map::new()),
                );
            }
            if !p.desc.trim().is_empty() {
                node.insert(
                    "description".to_string(),
                    serde_json::Value::String(p.desc.trim().to_string()),
                );
            }
            // 提示词形态用「必填 / 非空」文案表达；JSON Schema 侧用 minLength 表达同一条约束。
            if p.non_empty && p.ty == ParamType::String {
                node.insert("minLength".to_string(), serde_json::json!(1));
            }
            if let Some(lo) = p.min {
                node.insert("minimum".to_string(), json_num(lo));
            }
            if let Some(hi) = p.max {
                node.insert("maximum".to_string(), json_num(hi));
            }
            if let Some(d) = &p.default {
                node.insert("default".to_string(), d.clone());
            }
            if let Some(vals) = &p.values {
                node.insert(
                    "enum".to_string(),
                    serde_json::Value::Array(
                        vals.iter()
                            .map(|v| serde_json::Value::String(v.clone()))
                            .collect(),
                    ),
                );
            }
            props.insert(name.clone(), serde_json::Value::Object(node));
            if p.required {
                required.push(serde_json::Value::String(name.clone()));
            }
        }
        let mut root = serde_json::Map::new();
        root.insert(
            "type".to_string(),
            serde_json::Value::String("object".to_string()),
        );
        root.insert("properties".to_string(), serde_json::Value::Object(props));
        root.insert("required".to_string(), serde_json::Value::Array(required));
        root.insert(
            "additionalProperties".to_string(),
            serde_json::Value::Bool(false),
        );
        serde_json::Value::Object(root)
    }

    /// 目的：按**完整声明**校验一次调用（呈现层：人经界面 / 文本入口给的参数）。返回**事实**（哪个参数、哪种不符），文案由调用方套册子。
    pub fn check(&self, args: &serde_json::Value) -> Result<(), ArgFault> {
        self.check_with(args, false)
    }

    /// 目的：按**模型面**校验一次调用：`human` 参数不收（给了就是 `HumanOnly`，不是静默忽略），必填也不按它算。
    pub fn check_model(&self, args: &serde_json::Value) -> Result<(), ArgFault> {
        self.check_with(args, true)
    }

    fn check_with(&self, args: &serde_json::Value, model: bool) -> Result<(), ArgFault> {
        let Some(params) = self.declared() else {
            return Ok(());
        };
        let obj = match args.as_object() {
            Some(o) => o,
            None => return Err(ArgFault::NotObject),
        };
        for (name, p) in params {
            if model && self.human_only(name) {
                continue;
            }
            match obj.get(name) {
                None => {
                    if p.required {
                        return Err(ArgFault::Missing(name.clone()));
                    }
                }
                Some(v) if v.is_null() => {
                    if p.required {
                        return Err(ArgFault::Missing(name.clone()));
                    }
                }
                Some(v) => {
                    if !p.ty.accepts(v) {
                        return Err(ArgFault::WrongType {
                            name: name.clone(),
                            want: p.ty.name(),
                        });
                    }
                    if p.non_empty && v.as_str().map(str::is_empty).unwrap_or(false) {
                        return Err(ArgFault::Empty(name.clone()));
                    }
                    if let Some(vals) = &p.values
                        && let Some(s) = v.as_str()
                        && !vals.iter().any(|x| x == s)
                    {
                        return Err(ArgFault::NotInValues {
                            name: name.clone(),
                            values: vals.clone(),
                        });
                    }
                    if let Some(n) = v.as_f64() {
                        if let Some(lo) = p.min
                            && n < lo
                        {
                            return Err(ArgFault::TooSmall {
                                name: name.clone(),
                                min: compact_num(lo),
                            });
                        }
                        if let Some(hi) = p.max
                            && n > hi
                        {
                            return Err(ArgFault::TooBig {
                                name: name.clone(),
                                max: compact_num(hi),
                            });
                        }
                    }
                }
            }
        }
        // 声明没写的键一律拒收：不猜模型想干什么（与 `additionalProperties: false` 同义）。
        // 模型面把"人用参数"单列一条事实：它**是**声明过的键，但只有人能填。
        for k in obj.keys() {
            if !params.contains_key(k) {
                return Err(ArgFault::Unknown(k.clone()));
            }
            if model && self.human_only(k) {
                return Err(ArgFault::HumanOnly(k.clone()));
            }
        }
        Ok(())
    }

    /// 目的：原生工具调用用的声明形态：名字 + 一句话说明 + JSON Schema 参数。
    pub fn decl(&self, name: &str) -> crate::units::business::llm::api::ToolDecl {
        crate::units::business::llm::api::ToolDecl {
            name: name.to_string(),
            description: self.desc.clone(),
            parameters: self.to_json_schema(),
        }
    }

    /// 目的：补上缺省值（不改动已有键；呈现层的完整声明）。
    pub fn apply_defaults(&self, args: &mut serde_json::Value) {
        self.apply_defaults_with(args, false);
    }

    /// 目的：模型面的缺省值补充——`human` 参数的缺省不注入（模型面根本不该出现这些键）。
    pub fn apply_model_defaults(&self, args: &mut serde_json::Value) {
        self.apply_defaults_with(args, true);
    }

    fn apply_defaults_with(&self, args: &mut serde_json::Value, model: bool) {
        let Some(params) = self.declared() else {
            return;
        };
        let Some(obj) = args.as_object_mut() else {
            return;
        };
        for (name, p) in params {
            if model && self.human_only(name) {
                continue;
            }
            if !obj.contains_key(name)
                && let Some(d) = &p.default
            {
                obj.insert(name.clone(), d.clone());
            }
        }
    }
}

/// 目的：把一份**服务给的 JSON Schema 原样**渲染成模型侧的参数说明行（信封通道用；原生通道直接发这份 schema）。
/// 返回：`properties` 每个参数一行（名字、类型、必填、缺省、可选值、说明）；没有 `properties` = 没有可列的参数；
///   整份不是对象 schema 时如实给一行紧凑 JSON（不猜结构）。
/// 约束：这是**说明**，不是校验——核心不按它拦调用（服务是参数的权威，见 `residents::api::Operation`）。
pub fn json_schema_lines(schema: &serde_json::Value) -> Vec<String> {
    let Some(root) = schema.as_object() else {
        return vec![compact(schema)];
    };
    let Some(props) = root.get("properties").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let required: Vec<&str> = root
        .get("required")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
        .unwrap_or_default();
    props
        .iter()
        .map(|(name, node)| {
            let ty = node.get("type").and_then(|v| v.as_str()).unwrap_or("any");
            let mut marks: Vec<String> = vec![ty.to_string()];
            if required.contains(&name.as_str()) {
                marks.push("必填".to_string());
            }
            if let Some(d) = node.get("default") {
                marks.push(format!("缺省 {}", compact(d)));
            }
            if let Some(e) = node.get("enum").and_then(|v| v.as_array()) {
                let items: Vec<String> = e.iter().map(compact).collect();
                marks.push(format!("可选 {}", items.join(" / ")));
            }
            let desc = node
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if desc.is_empty() {
                format!("- {}（{}）", name, marks.join("，"))
            } else {
                format!("- {}（{}）：{}", name, marks.join("，"), desc)
            }
        })
        .collect()
}

/// 目的：参数不符的**事实**（文案在册子里；本层只说发生了什么）。
#[derive(Debug, Clone, PartialEq)]
pub enum ArgFault {
    /// 参数不是对象。
    NotObject,
    /// 缺少必填项。
    Missing(String),
    /// 类型不符。
    WrongType { name: String, want: &'static str },
    /// 字符串参数是空串。
    Empty(String),
    /// 数值小于下界。
    TooSmall { name: String, min: String },
    /// 数值大于上界。
    TooBig { name: String, max: String },
    /// 声明里没有这个键。
    Unknown(String),
    /// 声明过、但**只在呈现层可选**的键被模型给了（会话环境 / 权限这类选择只有人能填）。
    HumanOnly(String),
    /// 值不在声明的可选值清单里。
    NotInValues { name: String, values: Vec<String> },
    /// 文本入口（CLI/TUI）的词法错误：命令里出现了声明没有的旗语。
    UnknownFlag(String),
    /// 旗语缺值（布尔旗给了 `=值` 也归这里）。
    FlagNeedsValue(String),
    /// 位置参数或旗语重复给了。
    Duplicate(String),
    /// 多了声明没收的位置参数。
    ExtraOperand(String),
}

/// 数值的 JSON 写法：整数就写整数（2000 而不是 2000.0）。
fn json_num(n: f64) -> serde_json::Value {
    if n.fract() == 0.0 && n.abs() < 9.0e15 {
        serde_json::Value::from(n as i64)
    } else {
        serde_json::Value::from(n)
    }
}

/// 数值的紧凑写法（1.0 → 1；用于说明文案，不参与判定）。
fn compact_num(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{}", n as i64)
    } else {
        format!("{}", n)
    }
}

/// JSON 值的紧凑写法（缺省值展示用）。
fn compact(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> ToolSchema {
        let yaml = r#"
desc: 读取文本文件的行区间
params:
  path: { type: string, required: true, desc: 真实绝对路径 }
  offset: { type: integer, desc: 起始行, default: 1, min: 1 }
  limit: { type: integer, desc: 最多返回行数, max: 2000 }
"#;
        yaml_serde::from_str(yaml).expect("声明必须能解析")
    }

    /// 服务给的 JSON Schema 原样渲染成说明行（信封通道用）：参数名、类型、必填、缺省、可选值、说明都出来。
    #[test]
    fn json_schema_lines_renders_service_contract() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "真实绝对路径" },
                "limit": { "type": "integer", "default": 10 },
                "mode": { "type": "string", "enum": ["fast", "slow"] }
            },
            "required": ["path"]
        });
        let lines = json_schema_lines(&schema);
        assert!(
            lines
                .iter()
                .any(|l| l.contains("path") && l.contains("必填") && l.contains("真实绝对路径")),
            "{:?}",
            lines
        );
        assert!(
            lines
                .iter()
                .any(|l| l.contains("limit") && l.contains("缺省 10")),
            "{:?}",
            lines
        );
        assert!(
            lines
                .iter()
                .any(|l| l.contains("mode") && l.contains("可选 fast / slow")),
            "{:?}",
            lines
        );
        // 没有 properties = 没有可列的参数；整份不是对象 schema 时如实给紧凑 JSON。
        assert!(json_schema_lines(&serde_json::json!({"type": "object"})).is_empty());
        assert_eq!(json_schema_lines(&serde_json::json!("weird")).len(), 1);
    }

    #[test]
    fn check_reports_the_fact_not_a_guess() {
        let s = book();
        assert_eq!(
            s.check(&serde_json::json!({})),
            Err(ArgFault::Missing("path".to_string()))
        );
        assert_eq!(
            s.check(&serde_json::json!({"path": 1})),
            Err(ArgFault::WrongType {
                name: "path".to_string(),
                want: "string"
            })
        );
        assert_eq!(
            s.check(&serde_json::json!({"path": "p", "limit": 2001})),
            Err(ArgFault::TooBig {
                name: "limit".to_string(),
                max: "2000".to_string()
            })
        );
        assert_eq!(
            s.check(&serde_json::json!({"path": "p", "offset": 0})),
            Err(ArgFault::TooSmall {
                name: "offset".to_string(),
                min: "1".to_string()
            })
        );
        assert_eq!(
            s.check(&serde_json::json!({"path": "p", "nope": 1})),
            Err(ArgFault::Unknown("nope".to_string()))
        );
        assert!(
            s.check(&serde_json::json!({"path": "p"})).is_ok(),
            "只给必填项就是合法调用"
        );
        let k: ToolSchema = yaml_serde::from_str(
            "params:
  keyword: { type: string, required: true, non_empty: true }
",
        )
        .unwrap();
        assert_eq!(
            k.check(&serde_json::json!({"keyword": ""})),
            Err(ArgFault::Empty("keyword".to_string()))
        );
        assert!(k.check(&serde_json::json!({"keyword": "x"})).is_ok());
        assert!(
            s.check(&serde_json::json!({"path": "p", "limit": 2000}))
                .is_ok(),
            "上界含在内"
        );
        assert!(
            s.check(&serde_json::json!({"path": "p", "offset": null}))
                .is_ok(),
            "可选项给 null 等于没给"
        );
    }

    #[test]
    fn defaults_are_applied_without_overwriting() {
        let s = book();
        let mut a = serde_json::json!({"path": "p"});
        s.apply_defaults(&mut a);
        assert_eq!(a["offset"], serde_json::json!(1));
        let mut b = serde_json::json!({"path": "p", "offset": 7});
        s.apply_defaults(&mut b);
        assert_eq!(b["offset"], serde_json::json!(7), "已有值不动");
    }

    #[test]
    fn renders_both_shapes_from_one_declaration() {
        let s = book();
        let text = s.render_for_prompt();
        assert!(text.contains("读取文本文件的行区间"));
        assert!(
            text.contains("- path（string，必填）：真实绝对路径"),
            "{}",
            text
        );
        assert!(
            text.contains("- offset（integer，缺省 1，不小于 1）：起始行"),
            "{}",
            text
        );
        assert!(
            text.contains("- limit（integer，不大于 2000）：最多返回行数"),
            "{}",
            text
        );

        let js = s.to_json_schema();
        assert_eq!(js["type"], serde_json::json!("object"));
        assert_eq!(
            js["properties"]["path"]["type"],
            serde_json::json!("string")
        );
        assert_eq!(
            js["properties"]["limit"]["maximum"],
            serde_json::json!(2000)
        );
        assert_eq!(js["required"], serde_json::json!(["path"]));
        assert_eq!(js["additionalProperties"], serde_json::json!(false));

        // 人用参数（`human`）在模型面整体消失：说明、原生 schema、必填、缺省都不带它。
        let h: ToolSchema = yaml_serde::from_str(
            "desc: 建会话
params:
  request_id: { type: string, required: true }
  sandbox: { type: string, desc: 执行环境策略 }
human: [sandbox]
",
        )
        .unwrap();
        let text = h.render_for_prompt();
        assert!(!text.contains("sandbox"), "人用参数不发：{}", text);
        assert!(text.contains("- request_id"), "其余参数照发：{}", text);
        let js = h.to_json_schema();
        assert!(
            js["properties"].get("sandbox").is_none(),
            "原生声明不带人用参数：{}",
            js
        );
        assert_eq!(js["required"], serde_json::json!(["request_id"]));
        // 模型给了人用参数是**明确拒绝**，不是静默忽略（静默忽略会得到"环境悄悄继承"的会话）。
        assert_eq!(
            h.check_model(&serde_json::json!({"request_id": "r", "sandbox": "vm"})),
            Err(ArgFault::HumanOnly("sandbox".to_string()))
        );
        assert!(
            h.check_model(&serde_json::json!({"request_id": "r"}))
                .is_ok()
        );
        assert!(
            h.check(&serde_json::json!({"request_id": "r", "sandbox": "vm"}))
                .is_ok(),
            "呈现层照完整声明收"
        );
        // 人用参数的缺省只注入呈现层，不进模型面。
        let d: ToolSchema = yaml_serde::from_str(
            "params:
  sandbox: { type: string, default: host }
human: [sandbox]
",
        )
        .unwrap();
        let mut model_args = serde_json::json!({});
        d.apply_model_defaults(&mut model_args);
        assert_eq!(model_args, serde_json::json!({}), "模型面不注入人用缺省");
        let mut human_args = serde_json::json!({});
        d.apply_defaults(&mut human_args);
        assert_eq!(human_args["sandbox"], serde_json::json!("host"));
    }

    #[test]
    fn non_empty_becomes_min_length_in_native_schema() {
        let s: ToolSchema =
            yaml_serde::from_str("params:\n  q: { type: string, non_empty: true }").unwrap();
        let js = s.to_json_schema();
        assert_eq!(
            js["properties"]["q"]["minLength"],
            serde_json::json!(1),
            "原生声明要带上与提示词一致的「非空」约束"
        );
    }

    #[test]
    fn undeclared_params_means_no_validation() {
        let s: ToolSchema = yaml_serde::from_str("desc: 没有声明参数的工具").unwrap();
        assert!(
            s.check(&serde_json::json!({})).is_ok(),
            "没声明参数 = 不校验"
        );
        assert!(
            s.check(&serde_json::json!({"任意": 1})).is_ok(),
            "模块工具照旧能收到自由参数"
        );
        let mut a = serde_json::json!({"任意": 1});
        s.apply_defaults(&mut a);
        assert_eq!(a, serde_json::json!({"任意": 1}), "不校验也不动参数");
        assert_eq!(s.render_for_prompt(), "没有声明参数的工具");
    }

    #[test]
    fn declared_empty_params_rejects_every_key() {
        let s: ToolSchema = yaml_serde::from_str("params: {}").unwrap();
        assert!(s.check(&serde_json::json!({})).is_ok());
        assert_eq!(
            s.check(&serde_json::json!({"任意": 1})),
            Err(ArgFault::Unknown("任意".to_string()))
        );
    }

    /// 数组载荷（核心操作的结构化参数）：只认数组，且声明形态必须是**供应商认的 JSON Schema**。
    /// 为什么单独立一条：`type: json` 不是合法 JSON Schema 类型，整条请求会被供应商按"schema 非法"拒掉——
    /// 这种错夹具看不见（假供应商不校验 schema），只在真机上暴露。
    #[test]
    fn array_payload_renders_a_valid_json_schema() {
        let s: ToolSchema = yaml_serde::from_str(
            "desc: 交出方案与任务链\nparams:\n  nodes: { type: array, required: true, desc: 节点数组 }\n",
        )
        .unwrap();
        assert!(s.check(&serde_json::json!({"nodes": []})).is_ok());
        for bad in [
            serde_json::json!({}),
            serde_json::json!("n"),
            serde_json::json!(1),
        ] {
            assert_eq!(
                s.check(&serde_json::json!({"nodes": bad})),
                Err(ArgFault::WrongType {
                    name: "nodes".to_string(),
                    want: "array"
                })
            );
        }
        let js = s.to_json_schema();
        assert_eq!(
            js["properties"]["nodes"]["type"],
            serde_json::json!("array")
        );
        assert!(
            js["properties"]["nodes"]["items"].is_object(),
            "数组参数要在声明里带 items：{}",
            js
        );
        // 声明里出现的类型名必须是 JSON Schema 认的（供应商先校验 schema 本身，再执行调用）。
        for (name, p) in js["properties"].as_object().expect("properties 是对象") {
            let t = p["type"].as_str().unwrap_or("");
            assert!(
                ["string", "integer", "number", "boolean", "array", "object"].contains(&t),
                "参数 {} 的类型 {} 不是合法 JSON Schema 类型",
                name,
                t
            );
        }
    }
}
