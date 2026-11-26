//! 目的：命令目录的**投影**——手册、菜单、补全与「未知命令」建议都从同一条声明（`Op`）渲染。
//! 管：帮助正文、单条说明、补全候选与编辑距离建议的排版。
//! 不管：命令有哪些（在 `systools/commands.yaml`，投影见 `units/business/tools/domain/op.rs`）；
//!   命令怎么执行（转录相在 `cli.rs`，启动相在 `boot.rs`）；终端与配色怎么画（`term.rs`、`view.rs`）。
//! 联动：`cli.rs` 渲染 help 与提示；`term.rs` 的 Tab 补全取 `complete`；旗语正文取 `argv`。

use super::MenuRow;
use crate::units::business::catalog::modules::api::ParamType;
use crate::units::business::tools::api::{
    CommandBook, Executor, Op, OpBook, Phase, complete_op, op_usage,
};
use std::sync::OnceLock;

/// 目的：产品内置的命令表；表非法是编译期就该发现的事，这里让它在第一次使用时炸出来。
fn book() -> &'static CommandBook {
    CommandBook::builtin().unwrap_or_else(|e| panic!("命令表非法（systools/commands.yaml）：{}", e))
}

/// 目的：命令表投影成的 op 书——手册、补全、解析与分派都只认这一份声明。
pub(crate) fn ops() -> &'static OpBook {
    static OPS: OnceLock<OpBook> = OnceLock::new();
    OPS.get_or_init(|| OpBook::from_commands(book()))
}

/// 目的：按名字（也认 `--名字`）找一条**启动相**命令：转录中心据此决定要不要交给启动相执行。
/// 返回：None 表示这不是启动相命令，仍按转录相的命令分发。
pub(crate) fn boot(name: &str) -> Option<&'static Op> {
    ops().find(name).filter(|op| op.phase == Phase::Boot)
}

/// 目的：非交互模式每轮重印的一行菜单（只有转录相命令；启动相命令在 `help` 里，完整参数看 `help <命令>`）。
pub(crate) fn menu_text() -> String {
    let groups: Vec<String> = groups(Phase::Core)
        .iter()
        .map(|(group, list)| {
            let names: Vec<String> = list.iter().map(|op| op.id.clone()).collect();
            format!("{}：{}", group, names.join("/"))
        })
        .collect();
    format!(
        "命令：{}（help 看用法与旗语；help <命令> 看单条；Tab 补全）",
        groups.join(" · ")
    )
}

/// 目的：完整手册正文：用法、全局旗语、启动命令、转录中心里的命令与语法规则。
/// 参数：`all` 为真时连内部旗语一起列。
pub(crate) fn help_text(all: bool) -> String {
    let mut out = String::new();
    out.push_str("Solomni —— 终端转录中心\n\n");
    out.push_str("用法：solomni [全局旗语] [启动命令]\n");
    out.push_str("      带启动命令：直接执行完退出（单条参数用 solomni <命令> --help 看）\n");
    out.push_str("      不带：进转录中心，在提示符里输入下表的命令（启动相的写法带 -- 前缀）\n\n");
    out.push_str("全局旗语\n");
    for row in super::argv::flag_rows(all) {
        out.push_str(&row);
        out.push('\n');
    }
    out.push_str("\n启动命令（命令行直接执行；转录中心里也能跑，跑完回提示符）\n");
    push_groups(&mut out, Phase::Boot);
    out.push_str("\n转录中心里的命令（别名可当简写）\n");
    push_groups(&mut out, Phase::Core);
    out.push_str("\n语法规则\n");
    out.push_str("  1 旗语只出现在 solomni 主程序层；命令内部只用子命令与位置参数。\n");
    out.push_str("  2 短旗只留 -h、-v；其余一律长旗，且 --x v 与 --x=v 等价。\n");
    out.push_str("  3 布尔旗不带值；关掉一个开关用对应的 --no-xxx，不写 --xxx=false。\n");
    out.push_str("  4 `--` 之后全部当位置参数（参数里有空格或连字符时用）。\n");
    out.push_str("  5 未知旗语直接报错，编辑距离 ≤2 时给最近的一条建议。\n");
    out.push_str("  6 全局旗语写在启动命令前或后都算；命令自己的旗语按顺序接在它后面。\n");
    out
}

/// 目的：某一相的命令按分组归类（组序与命令序都按声明来，不另维护一份清单）。
fn groups(phase: Phase) -> Vec<(String, Vec<&'static Op>)> {
    let mut out: Vec<(String, Vec<&'static Op>)> = Vec::new();
    for op in ops().ops().iter().filter(|op| op.phase == phase) {
        match out.iter_mut().find(|(group, _)| *group == op.group) {
            Some((_, list)) => list.push(op),
            None => out.push((op.group.clone(), vec![op])),
        }
    }
    out
}

/// 目的：把某一相的命令按分组写进手册；组内一条命令一行，写法过宽就折到第二行。
fn push_groups(out: &mut String, phase: Phase) {
    for (group, list) in groups(phase) {
        out.push_str(&format!("  {}\n", group));
        for op in list {
            let alias = if op.aliases.is_empty() {
                String::new()
            } else {
                format!("（别名 {}）", op.aliases.join(" / "))
            };
            out.push_str(&command_line(&entry_usage(op), &op.brief, &alias));
        }
    }
}

/// 目的：手册里的写法：启动相加 `--` 前缀（命令行上的调用形式），转录相就是命令词本身。
pub(crate) fn entry_usage(op: &Op) -> String {
    let usage = op_usage(op);
    match op.phase {
        Phase::Boot => format!("--{}", usage),
        Phase::Core => usage,
    }
}

/// 目的：一条命令的手册行：写法、说明与别名；写法不超过 40 列就与说明同行。
fn command_line(head: &str, brief: &str, tail: &str) -> String {
    let used = super::width::cols(head);
    if used <= 40 {
        let mut line = format!("    {}", head);
        line.push_str(&" ".repeat(44 - used));
        line.push_str(brief);
        line.push_str(tail);
        line.push('\n');
        line
    } else {
        format!("    {}\n      {}{}\n", head, brief, tail)
    }
}

/// 目的：单条命令的说明（`help <命令>`）：写法、说明、分组与相、自己的参数与确认门槛。
pub(crate) fn detail(raw: &str) -> Option<String> {
    let op = ops().find(raw)?;
    let mut out = format!("{}  {}\n", entry_usage(op), op.brief);
    // 相由执行者决定：能在装配核心之前跑的才是启动命令。
    let phase = match op.executor {
        Executor::System => "启动命令",
        Executor::Core | Executor::Ui => "转录中心",
    };
    out.push_str(&format!("  {} · {}", op.group, phase));
    if !op.aliases.is_empty() {
        out.push_str(&format!("；别名 {}", op.aliases.join(" / ")));
    }
    out.push('\n');
    let params: Vec<String> = op
        .schema
        .params
        .as_ref()
        .map(|params| {
            params
                .iter()
                .filter(|(_, p)| p.positional.is_none() && !p.rest)
                .map(|(name, p)| param_row(name, p))
                .collect()
        })
        .unwrap_or_default();
    if !params.is_empty() {
        out.push_str("  旗语：\n");
        for row in params {
            out.push_str(&row);
        }
    }
    if op.confirm {
        out.push_str("  确认：有写操作，需 --yes 才算数（缺省只预览或只报状态）\n");
    }
    if op.phase == Phase::Boot {
        out.push_str("  退出码：0 成功；1 执行失败；2 用法或确认门槛不对\n");
        out.push_str("  转录中心：提示符里同样能跑（写操作会就地一问），跑完回提示符\n");
    }
    if let Some(json) = &op.json {
        out.push_str(&format!(
            "  等价 JSON：加 --json 得到这次调用的机器回执 {}\n",
            json
        ));
    }
    Some(out)
}

/// 目的：一行旗语说明（名字、取值提示、说明），从参数声明派生。
pub(crate) fn param_row(
    name: &str,
    p: &crate::units::business::catalog::modules::api::Param,
) -> String {
    let spec = if p.ty == ParamType::Boolean {
        format!("--{}", name)
    } else {
        let hint = match &p.values {
            Some(vals) => vals.join("|"),
            None => format!("<{}>", p.ty.name()),
        };
        format!("--{} {}", name, hint)
    };
    let used = super::width::cols(&spec);
    let pad = if used < 22 { 22 - used } else { 2 };
    format!("    {}{}{}\n", spec, " ".repeat(pad), p.desc)
}

/// 目的：Tab 补全候选：只对命令生效（`/` 开头），消息文本不给候选。
/// 参数：`prefix` 是当前正在输入的整行（含前导 `/`）；不是命令时返回空。
/// 约束：候选一律带前导 `/`——命令与消息的区别只有前缀，补全也不能自己换一套写法。
pub(crate) fn complete(prefix: &str) -> Vec<String> {
    let Some(rest) = prefix.strip_prefix('/') else {
        return Vec::new();
    };
    // 已经写全一个命令词 + 空格：补全从这个命令自己的声明里出（旗语与枚举值）。
    if let Some((head, tail)) = rest.split_once(' ')
        && let Some(op) = ops().find(head)
    {
        return complete_op(op, tail.trim_start());
    }
    let mut out: Vec<String> = Vec::new();
    for op in ops().ops() {
        let names = std::iter::once(&op.id).chain(op.aliases.iter());
        for name in names {
            let name = match op.phase {
                Phase::Boot => flag_form(name),
                Phase::Core => name.clone(),
            };
            if name.starts_with(rest) && !out.contains(&name) {
                out.push(format!("/{}", name));
            }
        }
    }
    out.sort();
    out
}

/// 目的：输入框下方的实时菜单：`/` 起头且还没敲空格时，按前缀列出转录相命令。
/// 参数：`prefix` 是当前整行（含前导 `/`）；不是命令写法时返回空。
/// 约束：顺序按命令声明，不排序；别名也能命中，但一行只出现一次（展示的是命令词本身）。
pub(crate) fn menu(prefix: &str) -> Vec<MenuRow> {
    let Some(rest) = prefix.strip_prefix('/') else {
        return Vec::new();
    };
    if rest.contains(' ') {
        return Vec::new();
    }
    ops()
        .ops()
        .iter()
        .filter(|op| op.phase == Phase::Core)
        .filter(|op| {
            op.id.starts_with(rest) || op.aliases.iter().any(|alias| alias.starts_with(rest))
        })
        .map(|op| MenuRow {
            name: format!("/{}", op.id),
            brief: op.brief.clone(),
        })
        .collect()
}

/// 目的：启动相命令在转录中心与命令行上的写法：长旗形式 `--名字`（别名里的前导 `/` 不算）。
fn flag_form(name: &str) -> String {
    format!("--{}", name.trim_start_matches('/'))
}

/// 目的：未知命令的「你是不是想…」建议：编辑距离 ≤2 时给最近的一条（两相的命令都算）。
pub(crate) fn suggest(word: &str) -> Option<String> {
    let mut best: Option<(String, usize)> = None;
    for op in ops().ops() {
        for name in std::iter::once(op.id.as_str()).chain(op.aliases.iter().map(String::as_str)) {
            let name = match op.phase {
                Phase::Boot => flag_form(name),
                Phase::Core => name.to_string(),
            };
            consider(word, &name, &mut best);
        }
    }
    best.map(|(name, _)| name)
}

/// 目的：比一条候选：编辑距离 ≤2 且比当前最近更近，才替换。
fn consider(word: &str, name: &str, best: &mut Option<(String, usize)>) {
    let d = super::distance(word, name);
    if d <= 2 && best.as_ref().is_none_or(|(_, bd)| d < *bd) {
        *best = Some((name.to_string(), d));
    }
}

/// 目的：版本行：包版本 + 构建档位（debug / release 由编译期决定）。
pub(crate) fn version_text() -> String {
    let build = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    format!("solomni {} （{} 构建）", env!("CARGO_PKG_VERSION"), build)
}
