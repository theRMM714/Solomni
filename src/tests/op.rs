//! 目的：命令契约的统一形状与文本解析器的用例——同一份声明派生写法、手册、补全与校验。
//! 管：`OpBook` 覆盖全部命令、`usage` 由声明派生、`parse` 的每条词法规则、`complete` 与 `problems` 的自洽。
//! 不管：命令怎么执行（各执行者自己的测试）、手册怎么排版（`src/tests/cli.rs`）。
//! 联动：`systools/commands.yaml`、`src/units/tools/domain/op.rs`。

use crate::units::catalog::modules::api::{Param, ParamType};
use crate::units::tools::api::{
    CommandBook, Executor, Op, OpBook, Phase, Runtime, ToolSchema, complete_op, op_usage, parse_op,
};
use serde_json::json;
use std::collections::BTreeMap;

/// 目的：内置命令表投影成的 op 目录。
fn ops() -> OpBook {
    let book = CommandBook::builtin().expect("命令表可解析");
    OpBook::from_commands(book)
}

fn op(id: &str) -> Op {
    ops()
        .find(id)
        .cloned()
        .unwrap_or_else(|| panic!("没有 {id}"))
}

fn parsed(id: &str, words: &[&str]) -> serde_json::Value {
    parse_op(&op(id), words).unwrap_or_else(|e| panic!("{id} {words:?} 解析失败：{e:?}"))
}

/// 目录：每条命令都投影成 op，名字与别名都能查到，执行者按声明落位。
#[test]
fn every_command_projects_into_one_book() {
    let book = CommandBook::builtin().expect("命令表可解析");
    let ops = OpBook::from_commands(book);
    assert_eq!(ops.ops().len(), book.commands().len(), "一条命令一个 op");
    assert!(ops.problems().is_empty(), "{:?}", ops.problems());
    for cmd in book.commands() {
        assert!(ops.find(&cmd.name).is_some(), "查不到 {}", cmd.name);
        assert_eq!(ops.find(&cmd.name).map(|o| o.phase), Some(cmd.phase));
    }
    assert_eq!(ops.find("--gc").map(|o| o.id.as_str()), Some("gc"));
    assert_eq!(ops.find("/help").map(|o| o.id.as_str()), Some("help"));
    assert_eq!(ops.find("?").map(|o| o.id.as_str()), Some("help"));

    // 执行者：启动相一律走系统能力；界面动作走 ui；其余走能力面。
    for o in ops.ops().iter().filter(|o| o.phase == Phase::Boot) {
        assert_eq!(o.executor, Executor::System, "{} 是启动相命令", o.id);
    }
    assert_eq!(op("help").executor, Executor::Ui);
    assert_eq!(op("version").executor, Executor::Ui);
    assert_eq!(op("work").executor, Executor::Core);
    // 模块词不是命令：模块工具经模块能力面调用，不进命令目录（模块 id 另由保留名拦）。
    assert!(ops.find("render").is_none(), "模块不该出现在命令目录里");
}

/// 写法：位置参数在前（必填 `<>`、可选 `[]`、枚举 `a|b`），具名旗语在后。
#[test]
fn usage_is_derived_from_the_declaration() {
    assert_eq!(
        op_usage(&op("work")),
        "work <session> [log|show|restore] [rev]"
    );
    assert_eq!(
        op_usage(&op("rewind")),
        "rewind <session> <archive|delete|restore> <id>"
    );
    assert_eq!(op_usage(&op("single")), "single [agents…]");
    assert_eq!(op_usage(&op("proxy")), "proxy");
    assert_eq!(op_usage(&op("help")), "help [topic] [--all]");
    assert_eq!(
        op_usage(&op("gc")),
        "gc [--category <string>] [--guest <string>] [--module <string>]"
    );
    assert_eq!(
        op_usage(&op("setup")),
        "setup [path|unset|undo|status|fence|all]"
    );
    assert_eq!(op_usage(&op("web")), "web [port]");
}

/// R1 词法：`--name 值` 与 `--name=值` 等价；`--` 之后一律当位置参数。
#[test]
fn flags_are_equivalent_in_all_three_spellings() {
    let inline = parsed("work", &["s1", "--action=show"]);
    let spaced = parsed("work", &["s1", "--action", "show"]);
    let bare = parsed("work", &["s1", "show"]);
    assert_eq!(inline, spaced);
    assert_eq!(inline, bare);
    assert_eq!(inline, json!({"session": "s1", "action": "show"}));

    // `--` 终止旗语：后面的词即使长得像旗语也是位置参数。
    assert_eq!(
        parsed("single", &["--", "--weird"]),
        json!({"agents": "--weird"})
    );
}

/// R2/R3 顺序：位置参数按声明序号取，具名参数可穿插；具名值不会被吃掉。
#[test]
fn positionals_follow_declaration_order() {
    assert_eq!(
        parsed("rewind", &["s1", "archive", "7"]),
        json!({"session": "s1", "mode": "archive", "id": 7})
    );
    assert_eq!(
        parsed("rewind", &["--mode", "delete", "s1", "9"]),
        json!({"session": "s1", "mode": "delete", "id": 9})
    );
    // rest 参数吃光剩下的词（`single a b` 是两个 agent 名）。
    assert_eq!(parsed("single", &["a", "b"]), json!({"agents": "a b"}));
}

/// R4 布尔只认存在性：`--flag` 置真，`--no-flag` 置假，`--flag=值` 报错。
#[test]
fn booleans_are_presence_only() {
    assert_eq!(parsed("help", &[]), json!({}));
    assert_eq!(parsed("help", &["--all"]), json!({"all": true}));
    assert_eq!(parsed("help", &["--no-all"]), json!({"all": false}));
    assert!(matches!(
        parse_op(&op("help"), &["--all=yes"]),
        Err(crate::units::tools::api::ArgFault::FlagNeedsValue(_))
    ));
}

/// R5 错误：缺值 / 未知 / 重复 / 类型不符 / 越界 / 枚举外与动作面共用同一套错误。
#[test]
fn faults_share_the_action_face_vocabulary() {
    use crate::units::tools::api::ArgFault;
    let help = op("help");
    let gc = op("gc");
    assert!(matches!(
        parse_op(&gc, &["--category"]),
        Err(ArgFault::FlagNeedsValue(f)) if f == "--category"
    ));
    assert!(matches!(
        parse_op(&gc, &["--nope"]),
        Err(ArgFault::UnknownFlag(f)) if f == "--nope"
    ));
    assert!(matches!(
        parse_op(&help, &["a", "--topic", "b"]),
        Err(ArgFault::Duplicate(p)) if p == "topic"
    ));
    assert!(matches!(
        parse_op(&op("proxy"), &["extra"]),
        Err(ArgFault::ExtraOperand(w)) if w == "extra"
    ));
    assert!(matches!(
        parse_op(&op("rewind"), &["s1", "archive", "oops"]),
        Err(ArgFault::WrongType { name, .. }) if name == "id"
    ));
    // 负数要写在 `--` 之后：单破折号多字符一律当未知旗语（R8）。
    assert!(matches!(
        parse_op(&op("rewind"), &["s1", "archive", "-1"]),
        Err(ArgFault::UnknownFlag(f)) if f == "-1"
    ));
    assert!(matches!(
        parse_op(&op("rewind"), &["s1", "archive", "--", "-1"]),
        Err(ArgFault::TooSmall { .. })
    ));
    assert!(matches!(
        parse_op(&op("rewind"), &["s1", "drop", "3"]),
        Err(ArgFault::NotInValues { name, .. }) if name == "mode"
    ));
    assert!(matches!(
        parse_op(&op("work"), &[]),
        Err(ArgFault::Missing { .. })
    ));
}

/// R6 确认门槛：`--yes` 只对声明了 confirm 的命令收；其余命令里它是未知旗语，也不进参数表。
#[test]
fn yes_is_a_gate_not_a_parameter() {
    use crate::units::tools::api::ArgFault;
    assert_eq!(
        parsed("work", &["s1", "--yes"]),
        json!({"session": "s1", "action": "log", "yes": true})
    );
    assert!(matches!(
        parse_op(&op("help"), &["--yes"]),
        Err(ArgFault::UnknownFlag(f)) if f == "--yes"
    ));
    assert!(matches!(
        parse_op(&op("work"), &["s1", "--yes=1"]),
        Err(ArgFault::FlagNeedsValue(_))
    ));
    let work = op("work");
    assert!(
        !work.schema.params.expect("work 有参数").contains_key("yes"),
        "确认门槛不该写进参数表"
    );
}

/// R1 可选值旗语：光写等于空串（清单全量），带值就取那个值，两种写法等价。
#[test]
fn optional_value_flags_may_come_bare() {
    assert_eq!(parsed("provision", &["--ensure"]), json!({"ensure": ""}));
    assert_eq!(
        parsed("provision", &["--ensure=toolchain"]),
        json!({"ensure": "toolchain"})
    );
    assert_eq!(
        parsed("provision", &["--ensure", "toolchain"]),
        json!({"ensure": "toolchain"})
    );
    assert!(
        op_usage(&op("provision")).contains("[--ensure[=<string>]]"),
        "{}",
        op_usage(&op("provision"))
    );
}

/// R8 短旗是闭集：只留 -h / -v（入口层摘走），其余单破折号一律当未知旗语；裸 `-` 是普通值。
#[test]
fn short_flags_are_a_closed_set() {
    use crate::units::tools::api::ArgFault;
    assert!(matches!(
        parse_op(&op("help"), &["-a"]),
        Err(ArgFault::UnknownFlag(f)) if f == "-a"
    ));
    assert!(matches!(
        parse_op(&op("help"), &["-help"]),
        Err(ArgFault::UnknownFlag(f)) if f == "-help"
    ));
    assert_eq!(parsed("single", &["-"]), json!({"agents": "-"}));
}

/// 缺省值：声明里写了 default 的，没给就补上（与模型侧调用同一条规则）。
#[test]
fn defaults_are_filled_from_the_declaration() {
    assert_eq!(
        parsed("work", &["s1"]),
        json!({"session": "s1", "action": "log"})
    );
    assert_eq!(
        parsed("work", &["s1", "log", "5"]),
        json!({"session": "s1", "action": "log", "rev": 5})
    );
}

/// 补全：`--` 开头的给旗语，否则给当前位置的枚举值；都只从声明出。
#[test]
fn completion_comes_from_the_declaration() {
    let help = op("help");
    let every = complete_op(&help, "--");
    assert!(every.contains(&"--all".to_string()), "{every:?}");
    assert!(every.contains(&"--topic=".to_string()), "{every:?}");
    assert_eq!(
        complete_op(&op("gc"), "--c"),
        vec!["--category=".to_string()]
    );
    let modes = complete_op(&op("setup"), "");
    assert!(modes.contains(&"path".to_string()), "{modes:?}");
    assert!(modes.contains(&"status".to_string()), "{modes:?}");
    assert!(complete_op(&op("work"), "z").is_empty());
    // 布尔旗语也补出取反写法。
    assert_eq!(complete_op(&help, "--no"), vec!["--no-all".to_string()]);
}

/// 自洽检查：位置序号冲突、rest 不在最后、枚举用在非字符串上都要报出来。
#[test]
fn broken_declarations_are_reported() {
    fn param(ty: ParamType, positional: Option<u32>, rest: bool) -> Param {
        Param {
            ty,
            required: false,
            non_empty: false,
            desc: String::new(),
            default: None,
            min: None,
            max: None,
            positional,
            rest,
            values: None,
            optional_value: false,
        }
    }
    let mut params: BTreeMap<String, Param> = BTreeMap::new();
    params.insert(
        "first".to_string(),
        param(ParamType::String, Some(1), false),
    );
    params.insert(
        "second".to_string(),
        param(ParamType::String, Some(1), false),
    );
    params.insert("rest".to_string(), param(ParamType::String, Some(2), true));
    let mut values = param(ParamType::Integer, None, false);
    values.values = Some(vec!["a".to_string()]);
    params.insert("count".to_string(), values);
    let op = Op {
        id: "broken".to_string(),
        aliases: Vec::new(),
        group: "测试".to_string(),
        brief: "自相矛盾的声明".to_string(),
        phase: Phase::Core,
        executor: Executor::Core,
        confirm: false,
        runtime: Runtime::None,
        schema: ToolSchema {
            desc: String::new(),
            params: Some(params),
            parallel: false,
            capability: String::new(),
            callers: Vec::new(),
            publish: false,
        },
        json: None,
    };
    let problems = OpBook::new(vec![op]).problems();
    let all = problems.join("；");
    assert!(all.contains("位置序号 1 给了两个参数"), "{all}");
    assert!(all.contains("可选值清单"), "{all}");
}

/// 自洽检查：启动相在核心装配之前跑完，不许声明需要常驻运行时。
#[test]
fn boot_commands_cannot_ask_for_services() {
    let mut boot = op("setup");
    boot.runtime = Runtime::Services;
    let all = OpBook::new(vec![boot]).problems().join("；");
    assert!(all.contains("不能声明 runtime: services"), "{all}");
}
/// 写法：必填的具名旗语不打方括号（缺了就是用法错误，不是可选项）。
#[test]
fn required_named_flags_lose_their_brackets() {
    fn param(ty: ParamType, required: bool) -> Param {
        Param {
            ty,
            required,
            non_empty: false,
            desc: String::new(),
            default: None,
            min: None,
            max: None,
            positional: None,
            rest: false,
            values: None,
            optional_value: false,
        }
    }
    let mut params: BTreeMap<String, Param> = BTreeMap::new();
    params.insert("api-key".to_string(), param(ParamType::String, true));
    params.insert("note".to_string(), param(ParamType::String, false));
    params.insert("force".to_string(), param(ParamType::Boolean, false));
    let op = Op {
        id: "provider".to_string(),
        aliases: Vec::new(),
        group: "登记".to_string(),
        brief: "供应商".to_string(),
        phase: Phase::Core,
        executor: Executor::Core,
        confirm: false,
        runtime: Runtime::None,
        schema: ToolSchema {
            desc: String::new(),
            params: Some(params),
            parallel: false,
            capability: String::new(),
            callers: Vec::new(),
            publish: false,
        },
        json: None,
    };
    assert_eq!(
        op_usage(&op),
        "provider --api-key <string> [--force] [--note <string>]"
    );
}
