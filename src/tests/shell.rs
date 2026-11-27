//! 目的：钉命令行的统一入口——模块清单、三级手册与模块工具的参数落点都从声明派生。
//! 管：清单正文、模块手册（文本与 JSON）、工具手册的派生用法、模块工具词法（具名旗 / 整段 JSON）。
//! 不管：词法规则本身（src/tests/op.rs）、业务动作的执行（各单元测试）。
//! 联动：src/presentation/cli/shell.rs、systools/commands.yaml、modules/*/module.yaml。

use super::doubles::module_of;
use super::prelude::*;
use crate::presentation::cli::shell;
use crate::units::business::catalog::modules::api::{Roster, ToolDecl};
use crate::units::pure::registry_model::api::{Param, ParamType};

fn param(ty: ParamType, desc: &str) -> Param {
    Param {
        ty,
        required: false,
        non_empty: false,
        desc: desc.to_string(),
        default: None,
        min: None,
        max: None,
        positional: None,
        rest: false,
        optional_value: false,
        values: None,
    }
}

/// 目的：带一个工具的模块：`report` 吃一个位置参数（语料）与一个具名旗（输出）。
fn render() -> Module {
    let mut m = module_of("render");
    let mut params = BTreeMap::new();
    let mut corpus = param(ParamType::String, "语料文件");
    corpus.required = true;
    corpus.positional = Some(1);
    params.insert("corpus".to_string(), corpus);
    params.insert("out".to_string(), param(ParamType::String, "输出文件"));
    m.manifest.tools.insert(
        "report".to_string(),
        ToolDecl {
            command: "node report.js".to_string(),
            desc: "写一份报告".to_string(),
            params: Some(params),
            parallel: false,
        },
    );
    m
}

/// 运行需求：判据只认命令声明——手册与版本永远不要，none 命令跑完即回，services 命令要，模块工具调用要。
#[test]
fn runtime_need_follows_declaration() {
    assert!(!shell::needs_runtime(&["modules"], false, false));
    assert!(!shell::needs_runtime(&["work", "log"], false, false));
    assert!(shell::needs_runtime(&["single"], false, false));
    assert!(shell::needs_runtime(&["web", "--json"], false, false));
    // 手册与版本：后面跟什么都不用装运行时（只打印）。
    assert!(!shell::needs_runtime(&[], true, false));
    assert!(!shell::needs_runtime(&["--help"], false, false));
    assert!(!shell::needs_runtime(&["--version"], false, false));
    assert!(!shell::needs_runtime(&["render", "--help"], false, false));
    // 非命令词：无尾词只可能是模块手册或拼错的命令（都只打印）；有尾词按模块工具调用处理。
    assert!(!shell::needs_runtime(&["render"], false, false));
    assert!(shell::needs_runtime(&["render", "report"], false, false));
    // 没有任何词表 = 交互入口，要运行时。
    assert!(shell::needs_runtime(&[], false, false));
    // 单个短横的多字符词：这一趟只报用法错，但按最保守的运行时处理。
    assert!(shell::needs_runtime(&["-json"], false, false));
}

/// 清单：模块、版本、启用态、一句话说明都在，拒收原因单独列（校验不是挑选）。
#[test]
fn listing_shows_version_state_and_rejections() {
    let roster = Roster {
        modules: vec![render()],
        rejected: vec!["bad: 缺少 module.yaml".to_string()],
    };
    let text = shell::listing_text(&roster, &|id| id == "render");
    assert!(text.contains("render"), "{text}");
    assert!(text.contains("0.1.0"), "{text}");
    assert!(text.contains("启用"), "{text}");
    assert!(text.contains("（拒收）bad: 缺少 module.yaml"), "{text}");
    assert!(text.contains("用法：solomni <模块>"), "{text}");

    let stopped = shell::listing_text(&roster, &|_| false);
    assert!(stopped.contains("停用"), "{stopped}");

    let empty = shell::listing_text(
        &Roster {
            modules: Vec::new(),
            rejected: Vec::new(),
        },
        &|_| false,
    );
    assert!(empty.contains("没有可用模块"), "{empty}");
}

/// 模块手册：版本与说明一行、启用态、给 agent 的系统说明原文、工具表与用法。
#[test]
fn module_manual_carries_version_system_and_tools() {
    let m = render();
    let text = shell::manual_text(&m, true);
    assert!(text.starts_with("render 0.1.0"), "{text}");
    assert!(text.contains("状态：已启用"), "{text}");
    assert!(text.contains("给 agent 的系统说明："), "{text}");
    assert!(text.contains("  用法：render"), "{text}");
    assert!(text.contains("report"), "{text}");
    assert!(
        text.contains("用法：solomni render <工具>"),
        "要告诉用户怎么调工具：{text}"
    );

    let stopped = shell::manual_text(&m, false);
    assert!(stopped.contains("已停用"), "{stopped}");

    let value = shell::manual_json(&m, true);
    assert_eq!(value["id"], "render");
    assert_eq!(value["version"], "0.1.0");
    assert_eq!(value["enabled"], true);
    assert_eq!(value["system"], "用法：render");
    assert_eq!(value["tools"][0]["name"], "report");
    assert_eq!(value["tools"][0]["params"]["corpus"]["type"], "string");
}

/// 工具手册：用法串与参数表都由声明派生——命令行写法是 `render report`，不是动作 id。
#[test]
fn tool_manual_derives_usage_from_the_declaration() {
    let m = render();
    let decl = m.manifest.tools.get("report").expect("声明了 report");
    let text = shell::tool_text(&m, "report", decl);
    assert!(text.starts_with("render report · 0.1.0"), "{text}");
    assert!(
        text.contains("用法：solomni render report <corpus> [--out <string>]"),
        "{text}"
    );
    assert!(text.contains("--out <string>"), "参数表要列具名旗：{text}");
    assert!(
        text.contains("模块版本与系统说明：solomni render"),
        "{text}"
    );

    let value = shell::tool_json(&m, "report", decl);
    assert_eq!(value["module"], "render");
    assert_eq!(value["tool"], "report");
    assert_eq!(value["usage"], "render report <corpus> [--out <string>]");

    // 没有参数的工具有两种：只收整段 JSON，与声明了空表（参数：无）。
    let mut bare = module_of("bare");
    bare.manifest.tools.insert(
        "run".to_string(),
        ToolDecl {
            command: "run".to_string(),
            desc: "跑一次".to_string(),
            params: None,
            parallel: false,
        },
    );
    let decl = bare.manifest.tools.get("run").expect("声明了 run");
    assert!(
        shell::tool_text(&bare, "run", decl).contains("参数：整段 JSON"),
        "没声明参数就只收整段 JSON"
    );
}

/// 模块工具的词法：声明了参数就认具名旗与位置参数，没声明就只收整段 JSON。
#[test]
fn module_tools_parse_like_commands_or_take_one_payload() {
    let m = render();
    let decl = m.manifest.tools.get("report").expect("声明了 report");
    let op = shell::tool_op(&m, "report", decl);
    assert_eq!(op.id, "module.render.report", "动作面用全名");

    let args = shell::tool_args(decl, &op, "render report", &["a.jsonl", "--out", "r.html"])
        .expect("位置参数与具名旗等价");
    assert_eq!(args["corpus"], "a.jsonl");
    assert_eq!(args["out"], "r.html");

    let fault = shell::tool_args(decl, &op, "render report", &["a.jsonl", "--nope"])
        .expect_err("没声明的旗语要拒收");
    assert!(fault.contains("没有旗语"), "{fault}");
    assert!(fault.contains("render report"), "用法按命令行写法：{fault}");

    let missing =
        shell::tool_args(decl, &op, "render report", &[]).expect_err("缺必填位置参数要拒收");
    assert!(missing.contains("corpus"), "{missing}");

    // 没声明参数的工具：唯一的入口是整段 JSON。
    let mut bare = module_of("bare");
    bare.manifest.tools.insert(
        "run".to_string(),
        ToolDecl {
            command: "run".to_string(),
            desc: "跑一次".to_string(),
            params: None,
            parallel: false,
        },
    );
    let decl = bare.manifest.tools.get("run").expect("声明了 run");
    let op = shell::tool_op(&bare, "run", decl);
    assert_eq!(
        shell::tool_args(decl, &op, "bare run", &[]).expect("省略 = {}"),
        serde_json::json!({})
    );
    assert_eq!(
        shell::tool_args(decl, &op, "bare run", &["{\"n\":2}"]).expect("整段 JSON"),
        serde_json::json!({ "n": 2 })
    );
    let broken =
        shell::tool_args(decl, &op, "bare run", &["不是 JSON"]).expect_err("不是 JSON 要说清楚");
    assert!(broken.contains("整段 JSON"), "{broken}");
}
