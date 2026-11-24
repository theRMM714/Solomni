//! 目的：命令契约的统一形状（Op）与文本入口的解析器——一份声明，五个消费者。
//! 管：`Op`（一条命令或动作的声明：分组、相位、执行者、确认门槛、参数 schema、回执说明）、
//!   `parse`（把命令行词法转成参数 JSON）、`usage` / `complete` / `problems`（写法、补全与自洽都由声明派生）、
//!   `OpBook`（一本书 + 自洽检查）。
//! 不管：命令怎么执行（各执行者自己的入口）、终端怎么渲染（呈现层只消费解析结果）。
//! 联动：`systools/commands.yaml` 经 `OpBook::from_commands` 投影成 op；动作目录（`systools/tools.yaml`）同形。
//!   文本永远只是 JSON 的投影：CLI/TUI 写的是同一份 schema 的位置参数与旗语，GUI 与模型直接传 JSON，
//!   两条路进的是同一份 `check`。

use super::commands::{CommandBook, Phase};
use super::schema::{ArgFault, ToolSchema};
use crate::capabilities::catalog::modules::api::{Param, ParamType};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// 目的：这条命令最后一步落在哪——**声明与解析共用，只有执行入口不同**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Executor {
    /// 走能力面：校验 → 授权 → 执行 → 审计（`ActionOps::act`）。
    #[default]
    Core,
    /// 走系统能力：装配核心之前就要能跑（doctor / gc / provision …）。
    System,
    /// 纯粹是界面动作：help / version / exit / web / rescan。
    Ui,
}

impl Executor {
    /// 目的：声明里与文案里用的名字。
    pub fn name(self) -> &'static str {
        match self {
            Executor::Core => "core",
            Executor::System => "system",
            Executor::Ui => "ui",
        }
    }
}

/// 目的：一条命令/动作的统一声明（CLI 手册、TUI 命令、Web 菜单、模型工具声明都从它派生）。
#[derive(Debug, Clone)]
pub struct Op {
    /// 目的：命令词或动作 id（`work` / `module.render.report`）。
    pub id: String,
    /// 目的：别名（`/help`、`?`）：只对文本入口有意义。
    pub aliases: Vec<String>,
    pub group: String,
    pub brief: String,
    pub phase: Phase,
    pub executor: Executor,
    /// 目的：这条命令有写操作，需要 `--yes` 才算数（缺省只预览或只报状态）。
    pub confirm: bool,
    /// 目的：参数契约——模型侧说明、JSON Schema、文本解析与校验都读它。
    pub schema: ToolSchema,
    /// 目的：`--json` 时的机器回执字段（给手册用；回执本身由呈现层排版）。
    pub json: Option<String>,
}

/// 目的：整本书：命令与动作**同一种形状**，按声明顺序排。
#[derive(Debug, Clone, Default)]
pub struct OpBook {
    ops: Vec<Op>,
}

impl OpBook {
    /// 目的：直接拿一份声明清单成书（模块工具按动作目录拼书时用）。
    #[cfg(test)]
    pub fn new(ops: Vec<Op>) -> OpBook {
        OpBook { ops }
    }

    /// 目的：把命令表投影成 op——命令与动作从此只差"谁执行"，不差形状。
    pub fn from_commands(book: &CommandBook) -> OpBook {
        let ops = book
            .commands()
            .iter()
            .map(|c| Op {
                id: c.name.clone(),
                aliases: c.aliases.clone(),
                group: c.group.clone(),
                brief: c.brief.clone(),
                phase: c.phase,
                executor: c.executor,
                confirm: c.confirm,
                schema: ToolSchema {
                    desc: c.brief.clone(),
                    params: c.params.clone(),
                    parallel: false,
                    capability: String::new(),
                    callers: vec!["user".to_string()],
                    publish: false,
                },
                json: c.json.clone(),
            })
            .collect();
        OpBook { ops }
    }

    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    /// 目的：按名字或别名查一条（接受前导 `--` 与 `/`）。
    pub fn find(&self, raw: &str) -> Option<&Op> {
        let name = word_of(raw);
        self.ops
            .iter()
            .find(|o| o.id == name || o.aliases.iter().any(|a| word_of(a) == name))
    }

    /// 目的：书占用的全部词（名字与别名）：模块 id 不得与它们同形。
    pub fn ids(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for op in &self.ops {
            for word in std::iter::once(&op.id).chain(op.aliases.iter()) {
                let w = word_of(word);
                if !out.contains(&w) {
                    out.push(w);
                }
            }
        }
        out
    }

    /// 目的：书自身的矛盾（空 = 一切正常）：重名、位置序号冲突、rest 位置、枚举用在非字符串上。
    pub fn problems(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        for op in &self.ops {
            if op.id.trim().is_empty() {
                out.push("有条命令没写名字".to_string());
                continue;
            }
            // 名字与别名共用一份词表：别名只是同一个词的另一种写法（`/help` 与 `help` 是一回事），不算重复。
            for word in std::iter::once(&op.id).chain(op.aliases.iter()) {
                let w = word_of(word);
                if w == op.id {
                    continue;
                }
                if seen.contains(&w) {
                    out.push(format!("名字 {} 重复声明", w));
                } else {
                    seen.push(w);
                }
            }
            // 相位与执行者必须自洽：启动相在核心装配之前跑，只可能是系统能力；系统能力也不许挂在转录相。
            match (op.phase, op.executor) {
                (Phase::Boot, Executor::System) | (Phase::Core, Executor::Core | Executor::Ui) => {}
                (Phase::Boot, other) => out.push(format!(
                    "{} 是启动相命令，执行者只能是 system（现在写的是 {}）",
                    op.id,
                    other.name()
                )),
                (Phase::Core, Executor::System) => out.push(format!(
                    "{} 是转录相命令，执行者不能是 system（装配核心之前不会跑它）",
                    op.id
                )),
            }
            out.extend(schema_problems(&op.id, op.schema.params.as_ref()));
        }
        out
    }
}

/// 目的：一个位置参数（序号、名字、声明）。
type Slot<'a> = (u32, &'a String, &'a Param);

/// 目的：有固定序号的参数（按序号排）：位置参数按它取，不写旗语的词就落在这里。
fn slots(params: &BTreeMap<String, Param>) -> Vec<Slot<'_>> {
    let mut out: Vec<Slot<'_>> = params
        .iter()
        .filter_map(|(name, p)| p.positional.filter(|_| !p.rest).map(|i| (i, name, p)))
        .collect();
    out.sort_by_key(|(i, _, _)| *i);
    out
}

/// 目的：吃光剩余位置参数的那一个（如 `single <agent名…>`）。
fn rest_param(params: &BTreeMap<String, Param>) -> Option<(&String, &Param)> {
    params.iter().find(|(_, p)| p.rest)
}

/// 目的：参数声明的自洽检查（与具体命令无关的规则）。
fn schema_problems(id: &str, params: Option<&BTreeMap<String, Param>>) -> Vec<String> {
    let Some(params) = params else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for (name, p) in params {
        if let Some(vals) = &p.values {
            if p.ty != ParamType::String {
                out.push(format!(
                    "{} 的参数 {} 用了可选值清单，但类型不是 string",
                    id, name
                ));
            }
            if vals.is_empty() {
                out.push(format!("{} 的参数 {} 的可选值清单是空的", id, name));
            }
        }
        if p.optional_value && (p.ty == ParamType::Boolean || p.positional.is_some()) {
            out.push(format!(
                "{} 的参数 {} 声明了 optional_value，但它要么是布尔、要么是位置参数（都用不上）",
                id, name
            ));
        }
        if p.rest && p.positional.is_some() {
            out.push(format!(
                "{} 的参数 {} 声明了 rest，就不要再写位置序号（rest 永远排在最后）",
                id, name
            ));
        }
    }
    let rests: Vec<&String> = params
        .iter()
        .filter(|(_, p)| p.rest)
        .map(|(n, _)| n)
        .collect();
    if rests.len() > 1 {
        out.push(format!("{} 有多个 rest 参数（只能有一个）", id));
    }
    let mut numbers: Vec<u32> = Vec::new();
    for (index, name, _) in slots(params) {
        if index == 0 {
            out.push(format!("{} 的参数 {} 位置序号要从 1 起", id, name));
        }
        if numbers.contains(&index) {
            out.push(format!("{} 的位置序号 {} 给了两个参数", id, index));
        }
        numbers.push(index);
    }
    out
}

/// 目的：别名词去掉前导旗/斜杠（`/help` → `help`）。
pub fn word_of(word: &str) -> String {
    word.trim()
        .trim_start_matches('/')
        .trim_start_matches('-')
        .to_string()
}

/// 目的：由声明派生写法。位置参数在前（按序号），rest 次之，具名的在后；必填用 `<>`，可选与布尔用 `[]`。
pub fn usage(op: &Op) -> String {
    usage_named(op, &op.id)
}

/// 目的：换个词头写同一条声明（模块工具在命令行上叫 `render report`，动作 id 不是这个写法）。
pub fn usage_named(op: &Op, head: &str) -> String {
    let mut out: Vec<String> = vec![head.to_string()];
    let Some(params) = op.schema.params.as_ref() else {
        return out.join(" ");
    };
    for (_, name, p) in slots(params) {
        out.push(slot_text(name, p));
    }
    if let Some((name, p)) = rest_param(params) {
        out.push(slot_text(&format!("{}…", name), p));
    }
    for (name, p) in params {
        if p.positional.is_some() || p.rest {
            continue;
        }
        let head = if p.ty == ParamType::Boolean {
            format!("--{}", name)
        } else {
            let hint = match &p.values {
                Some(vals) => vals.join("|"),
                None => type_hint(p),
            };
            if p.optional_value {
                // 可光写旗语：`[--ensure[=<string>]]`。
                format!("--{}[={}]", name, hint)
            } else {
                format!("--{} {}", name, hint)
            }
        };
        if p.required && !p.optional_value {
            out.push(head);
        } else {
            out.push(format!("[{}]", head));
        }
    }
    out.join(" ")
}

/// 目的：一个位置参数的写法：必填 `<x>`、可选 `[x]`；有枚举就列出来。
fn slot_text(name: &str, p: &Param) -> String {
    let inner = match &p.values {
        Some(vals) => vals.join("|"),
        None => name.to_string(),
    };
    if p.required {
        format!("<{}>", inner)
    } else {
        format!("[{}]", inner)
    }
}

/// 目的：旗语的取值提示（`<integer>` / `<string>` …）。
fn type_hint(p: &Param) -> String {
    format!("<{}>", p.ty.name())
}

/// 目的：把命令行词法解析成参数 JSON——**文本只是 JSON 的投影**。
///   词法规则（与手册、补全同源）：
///   R1 `--name` / `--name=值` / `--name 值` 等价；`--` 之后一律当位置参数；`--no-<布尔>` 取反；裸 `-` 是普通值。
///   R2 全局修饰 `--root` 由入口先摘走，不进这里；`--json` 与 `--yes` 是呈现/确认修饰
///   （`--yes` 只对声明了确认门槛的命令收，R6），命令自己的参数前后位置无所谓。
///   R3 位置参数按声明的 `positional` 序号取，声明了才允许不写旗语；rest 参数吃光剩下的词。
///   R4 布尔只认存在性：`--flag=值` 报错。
///   R5 缺值 / 未知 / 重复 / 类型不符 / 越界 / 枚举外与动作面共用同一套 `ArgFault`。
pub fn parse(op: &Op, words: &[&str]) -> Result<Value, ArgFault> {
    let empty: BTreeMap<String, Param> = BTreeMap::new();
    let params = op.schema.params.as_ref().unwrap_or(&empty);
    let slots = slots(params);
    let mut out: Map<String, Value> = Map::new();
    // 声明之外的修饰符（目前只有确认门槛的 `yes`）：不进 schema 校验，最后并回结果。
    let mut modifiers: Map<String, Value> = Map::new();
    let mut next = 0usize;
    let mut rest_words: Vec<String> = Vec::new();
    let mut flags_done = false;
    let mut i = 0usize;
    while i < words.len() {
        let word = words[i];
        i += 1;
        if !flags_done && word == "--" {
            flags_done = true;
            continue;
        }
        // R8：短旗只留 -h / -v（它们在入口层摘走），这里出现单破折号多字符一律当未知旗语。
        if !flags_done && word.starts_with('-') && !word.starts_with("--") && word.len() > 1 {
            return Err(ArgFault::UnknownFlag(word.to_string()));
        }
        if !flags_done && word.starts_with("--") {
            let body = &word[2..];
            let (name, inline) = match body.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (body, None),
            };
            if name.is_empty() {
                return Err(ArgFault::UnknownFlag(word.to_string()));
            }
            // R6：`--yes` 不是每条命令自己的参数——只有声明了确认门槛的命令收它。
            if name == "yes" {
                if !op.confirm {
                    return Err(ArgFault::UnknownFlag(word.to_string()));
                }
                if inline.is_some() {
                    return Err(ArgFault::FlagNeedsValue(word.to_string()));
                }
                if modifiers.contains_key("yes") {
                    return Err(ArgFault::Duplicate("yes".to_string()));
                }
                modifiers.insert("yes".to_string(), Value::Bool(true));
                continue;
            }
            // R6：`--json` 同样是呈现修饰而不是参数：文本入口（TUI 行内）与启动旗语 `solomni --json` 同义。
            if name == "json" {
                if inline.is_some() {
                    return Err(ArgFault::FlagNeedsValue(word.to_string()));
                }
                if modifiers.contains_key("json") {
                    return Err(ArgFault::Duplicate("json".to_string()));
                }
                modifiers.insert("json".to_string(), Value::Bool(true));
                continue;
            }
            // `--no-<布尔>` 取反：只对声明成布尔的参数成立。
            if let Some(neg) = name.strip_prefix("no-") {
                let Some((pname, p)) = params.get_key_value(neg) else {
                    return Err(ArgFault::UnknownFlag(word.to_string()));
                };
                if p.ty != ParamType::Boolean || inline.is_some() {
                    return Err(ArgFault::FlagNeedsValue(word.to_string()));
                }
                if out.contains_key(pname) {
                    return Err(ArgFault::Duplicate(pname.clone()));
                }
                out.insert(pname.clone(), Value::Bool(false));
                continue;
            }
            let Some((pname, p)) = params.get_key_value(name) else {
                return Err(ArgFault::UnknownFlag(word.to_string()));
            };
            if out.contains_key(pname) {
                return Err(ArgFault::Duplicate(pname.clone()));
            }
            if p.ty == ParamType::Boolean {
                if inline.is_some() {
                    return Err(ArgFault::FlagNeedsValue(word.to_string()));
                }
                out.insert(pname.clone(), Value::Bool(true));
                continue;
            }
            let raw = match inline {
                Some(v) => v,
                None => match words.get(i) {
                    // 下一个词不是旗语 ⇒ 它就是值（R1 的 `--x v` 写法）。
                    Some(next_word) if !next_word.starts_with("--") => {
                        i += 1;
                        next_word.to_string()
                    }
                    // 声明了 optional_value：光写旗语就收空串（`--ensure` = 全量）。
                    _ if p.optional_value => String::new(),
                    // 其余：缺值，且不吞掉下一条旗语。
                    _ => return Err(ArgFault::FlagNeedsValue(flag(name))),
                },
            };
            out.insert(pname.clone(), coerce(pname, p, &raw)?);
            continue;
        }
        // 位置参数：按声明顺序填（已经用旗语给过的不重复填，直接跳到下一个空位）。
        while slots
            .get(next)
            .is_some_and(|(_, n, _)| out.contains_key(*n))
        {
            next += 1;
        }
        if let Some((_, pname, p)) = slots.get(next) {
            out.insert((*pname).clone(), coerce(pname, p, word)?);
            next += 1;
            continue;
        }
        if rest_param(params).is_some() {
            rest_words.push(word.to_string());
            continue;
        }
        return Err(ArgFault::ExtraOperand(word.to_string()));
    }
    if let (Some((pname, p)), false) = (rest_param(params), rest_words.is_empty()) {
        out.insert(
            pname.clone(),
            match p.ty {
                ParamType::Array => {
                    Value::Array(rest_words.iter().cloned().map(Value::String).collect())
                }
                _ => Value::String(rest_words.join(" ")),
            },
        );
    }
    // 缺省值：声明里有 default 且这次没给就补上（与模型侧调用同一条规则）。
    for (name, p) in params {
        if !out.contains_key(name)
            && let Some(d) = &p.default
        {
            out.insert(name.clone(), d.clone());
        }
    }
    let value = Value::Object(out.clone());
    op.schema.check(&value)?;
    // 修饰符最后并回来：它们不属于声明，所以不参与声明校验。
    for (name, v) in modifiers {
        out.insert(name, v);
    }
    Ok(Value::Object(out))
}

/// 目的：旗语的规范写法（错误文案里用）。
fn flag(name: &str) -> String {
    format!("--{}", name)
}

/// 目的：一个词 → 参数值：按声明类型转换，转不过去就是类型不符（不静默当字符串收下）。
fn coerce(name: &str, p: &Param, raw: &str) -> Result<Value, ArgFault> {
    match p.ty {
        ParamType::String => Ok(Value::String(raw.to_string())),
        ParamType::Integer => {
            raw.parse::<i64>()
                .map(Value::from)
                .map_err(|_| ArgFault::WrongType {
                    name: name.to_string(),
                    want: "integer",
                })
        }
        ParamType::Number => raw
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .ok_or(ArgFault::WrongType {
                name: name.to_string(),
                want: "number",
            }),
        ParamType::Boolean => match raw.to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" => Ok(Value::Bool(true)),
            "false" | "no" | "0" => Ok(Value::Bool(false)),
            _ => Err(ArgFault::WrongType {
                name: name.to_string(),
                want: "boolean",
            }),
        },
        // 结构化载荷文本入口不收：整段 JSON 走 GUI/模型的 JSON 入口。
        ParamType::Array | ParamType::Object => Err(ArgFault::WrongType {
            name: name.to_string(),
            want: "JSON",
        }),
    }
}

/// 目的：按已输入的词给补全候选（只从声明出）：`--` 开头给旗语，否则给当前位置的枚举值。
pub fn complete(op: &Op, prefix: &str) -> Vec<String> {
    let Some(params) = op.schema.params.as_ref() else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    if prefix.starts_with("--") {
        for (name, p) in params {
            let head = if p.ty == ParamType::Boolean {
                format!("--{}", name)
            } else {
                format!("--{}=", name)
            };
            if head.starts_with(prefix) {
                out.push(head);
            }
            if p.ty == ParamType::Boolean {
                let neg = format!("--no-{}", name);
                if neg.starts_with(prefix) {
                    out.push(neg);
                }
            }
        }
        return out;
    }
    for (_, _, p) in slots(params) {
        if let Some(vals) = &p.values {
            for v in vals {
                if v.starts_with(prefix) {
                    out.push(v.clone());
                }
            }
        }
    }
    out
}
