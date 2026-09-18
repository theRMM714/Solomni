//! 工具参数契约（**声明在文本层**，代码只解析/校验/渲染）。
//!
//! 为什么单独一层：参数 schema 不该埋在 Rust 代码里——它是**模块与产品对模型的承诺**，
//! 要能像文案一样被看到、被修改、被复核。所以：
//! - 内置工具的参数声明在 `prompts.yaml` 的 `core.builtin_tools`；
//! - 模块工具的参数声明在 `module.yaml` 的 `tools.<名字>.params`（**可选**：不写就照旧工作）。
//!
//! 同一份声明同时驱动两件事：模型侧说明（`render_for_prompt`）与调用校验（`check`），
//! 所以"缺少 path / limit 不能大于 2000"这类文案不再硬编码在代码里。

use serde::Deserialize;
use std::collections::BTreeMap;

/// 参数类型（只支持机器能判定的最小集合；不猜、不做隐式转换）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamType {
    String,
    Integer,
    Number,
    Boolean,
}

impl ParamType {
    /// 模型侧与 JSON Schema 共用的类型名。
    pub fn name(self) -> &'static str {
        match self {
            ParamType::String => "string",
            ParamType::Integer => "integer",
            ParamType::Number => "number",
            ParamType::Boolean => "boolean",
        }
    }

    /// 该值是否属于这个类型（整数与数字分开判定，不做 1 == 1.0 的宽容）。
    fn accepts(self, v: &serde_json::Value) -> bool {
        match self {
            ParamType::String => v.is_string(),
            ParamType::Integer => v.is_i64() || v.is_u64(),
            ParamType::Number => v.is_number(),
            ParamType::Boolean => v.is_boolean(),
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

/// 一个工具的参数契约。
#[derive(Debug, Clone, Deserialize)]
pub struct ToolSchema {
    /// 给模型看的一句话说明（工具做什么）。
    #[serde(default)]
    pub desc: String,
    /// 参数表（名字 → 声明）。
    /// 整个不写 = **不做参数校验**（模块工具没声明 schema 时的情形，保持照旧工作）；
    /// 写了但为空 = 这个工具不收任何参数。
    #[serde(default)]
    pub params: Option<BTreeMap<String, Param>>,
    /// 这个工具**可并发执行**（缺省 false = 独占串行）。
    /// 只该给"只读、无副作用"的工具写：同一回复里的多个可并发调用会真的并发跑；
    /// 未声明的（含写入类）独占执行，并作为并发批次之间的屏障。
    #[serde(default)]
    pub parallel: bool,
}

impl ToolSchema {
    /// 声明了的参数表；没声明 = 调用方不做校验。
    fn declared(&self) -> Option<&BTreeMap<String, Param>> {
        self.params.as_ref()
    }
}

/// 一本书里的全部工具契约（key = 工具名）。内置工具与模块工具都用它。
pub type ToolBook = BTreeMap<String, ToolSchema>;

impl ToolSchema {
    /// 模型侧的参数说明：每个参数一行，带类型、必填/缺省与说明。
    pub fn render_for_prompt(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        if !self.desc.trim().is_empty() {
            lines.push(self.desc.trim().to_string());
        }
        for (name, p) in self.params.iter().flatten() {
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
            let desc = p.desc.trim();
            lines.push(if desc.is_empty() {
                format!("- {}（{}）", name, marks.join("，"))
            } else {
                format!("- {}（{}）：{}", name, marks.join("，"), desc)
            });
        }
        lines.join("\n")
    }

    /// JSON Schema 形态（同一声明的第二形态：附加属性一律拒收，与主流一致）。
    /// 原生工具调用通道由 decl() 取它渲染工具声明；手写信封通道不发它（模型看的是提示词里的参数说明）。
    pub fn to_json_schema(&self) -> serde_json::Value {
        let mut props = serde_json::Map::new();
        let mut required: Vec<serde_json::Value> = Vec::new();
        for (name, p) in self.params.iter().flatten() {
            let mut node = serde_json::Map::new();
            node.insert(
                "type".to_string(),
                serde_json::Value::String(p.ty.name().to_string()),
            );
            if !p.desc.trim().is_empty() {
                node.insert(
                    "description".to_string(),
                    serde_json::Value::String(p.desc.trim().to_string()),
                );
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

    /// 按声明校验一次调用的参数。返回**事实**（哪个参数、哪种不符），文案由调用方套册子。
    pub fn check(&self, args: &serde_json::Value) -> Result<(), ArgFault> {
        let Some(params) = self.declared() else {
            return Ok(());
        };
        let obj = match args.as_object() {
            Some(o) => o,
            None => return Err(ArgFault::NotObject),
        };
        for (name, p) in params {
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
                    if let Some(n) = v.as_f64() {
                        if let Some(lo) = p.min {
                            if n < lo {
                                return Err(ArgFault::TooSmall {
                                    name: name.clone(),
                                    min: compact_num(lo),
                                });
                            }
                        }
                        if let Some(hi) = p.max {
                            if n > hi {
                                return Err(ArgFault::TooBig {
                                    name: name.clone(),
                                    max: compact_num(hi),
                                });
                            }
                        }
                    }
                }
            }
        }
        // 声明没写的键一律拒收：不猜模型想干什么（与 `additionalProperties: false` 同义）。
        for k in obj.keys() {
            if !params.contains_key(k) {
                return Err(ArgFault::Unknown(k.clone()));
            }
        }
        Ok(())
    }

    /// 原生工具调用用的声明形态：名字 + 一句话说明 + JSON Schema 参数。
    pub fn decl(&self, name: &str) -> crate::core::ports::ToolDecl {
        crate::core::ports::ToolDecl {
            name: name.to_string(),
            description: self.desc.clone(),
            parameters: self.to_json_schema(),
        }
    }

    /// 补上缺省值（不改动已有键）。
    pub fn apply_defaults(&self, args: &mut serde_json::Value) {
        let Some(params) = self.declared() else {
            return;
        };
        let Some(obj) = args.as_object_mut() else {
            return;
        };
        for (name, p) in params {
            if !obj.contains_key(name) {
                if let Some(d) = &p.default {
                    obj.insert(name.clone(), d.clone());
                }
            }
        }
    }
}

/// 参数不符的**事实**（文案在册子里；本层只说发生了什么）。
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
}

/// 一本书的模型侧说明：每个工具一段（工具名 + 签名），给模型读。
pub fn render_book(book: &ToolBook) -> String {
    book.iter()
        .map(|(name, s)| format!("{}\n{}", name, s.render_for_prompt()))
        .collect::<Vec<_>>()
        .join("\n")
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
        serde_yaml::from_str(yaml).expect("声明必须能解析")
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
        let k: ToolSchema = serde_yaml::from_str(
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
    }

    #[test]
    fn undeclared_params_means_no_validation() {
        let s: ToolSchema = serde_yaml::from_str("desc: 没有声明参数的工具").unwrap();
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
        let s: ToolSchema = serde_yaml::from_str("params: {}").unwrap();
        assert!(s.check(&serde_json::json!({})).is_ok());
        assert_eq!(
            s.check(&serde_json::json!({"任意": 1})),
            Err(ArgFault::Unknown("任意".to_string()))
        );
    }
}
