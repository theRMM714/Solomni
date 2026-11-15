//! 目的：命令目录的**投影**——手册、菜单、补全与「未知命令」建议都从 `systools/commands.yaml` 那一张表渲染。
//! 管：帮助正文、单条说明、补全候选与编辑距离建议的排版。
//! 不管：命令有哪些（在 `systools/commands.yaml`，解析见 `capabilities/tools/domain/commands.rs`）；
//!   命令怎么执行（`cli.rs` 与启动相的 `entry/args.rs`）；终端与配色怎么画（`term.rs`、`view.rs`）。
//! 联动：`cli.rs` 渲染 help 与提示；`term.rs` 的 Tab 补全取 `complete`；旗语正文取 `argv`。

use crate::capabilities::tools::api::{CommandBook, Phase};

/// 目的：产品内置的命令表；表非法是编译期就该发现的事，这里让它在第一次使用时炸出来。
fn book() -> &'static CommandBook {
    CommandBook::builtin().unwrap_or_else(|e| panic!("命令表非法（systools/commands.yaml）：{}", e))
}

/// 目的：非交互模式每轮重印的一行菜单（只有转录相命令；完整参数看 `help`）。
pub(crate) fn menu_text() -> String {
    let book = book();
    let groups: Vec<String> = book
        .groups(Phase::Core)
        .iter()
        .map(|group| {
            let names: Vec<String> = book
                .in_group(Phase::Core, group)
                .iter()
                .map(|c| c.name.clone())
                .collect();
            names.join("/")
        })
        .filter(|row| !row.is_empty())
        .collect();
    format!(
        "命令：{}（help 看用法与旗语；help <命令> 看单条；Tab 补全）",
        groups.join(" · ")
    )
}

/// 目的：完整手册正文：用法、全局旗语、启动命令、转录中心里的命令与语法规则。
/// 参数：`all` 为真时连内部旗语一起列。
pub(crate) fn help_text(all: bool) -> String {
    let book = book();
    let mut out = String::new();
    out.push_str("Solomni —— 终端转录中心\n\n");
    out.push_str("用法：solomni [全局旗语] [启动命令]\n");
    out.push_str("      带启动命令：直接执行完退出（单条参数用 solomni <命令> --help 看）\n");
    out.push_str("      不带：进转录中心，在提示符里输入下表的命令\n\n");
    out.push_str("全局旗语\n");
    for row in super::argv::flag_rows(all) {
        out.push_str(&row);
        out.push('\n');
    }
    out.push_str("\n启动命令（直接执行后退出；不进转录中心）\n");
    push_groups(&mut out, book, Phase::Boot);
    out.push_str("\n转录中心里的命令（别名可当简写）\n");
    push_groups(&mut out, book, Phase::Core);
    out.push_str("\n语法规则\n");
    out.push_str("  1 旗语只出现在 solomni 主程序层；命令内部只用子命令与位置参数。\n");
    out.push_str("  2 短旗只留 -h、-v；其余一律长旗，且 --x v 与 --x=v 等价。\n");
    out.push_str("  3 布尔旗不带值；关掉一个开关用对应的 --no-xxx，不写 --xxx=false。\n");
    out.push_str("  4 `--` 之后全部当位置参数（参数里有空格或连字符时用）。\n");
    out.push_str("  5 未知旗语直接报错，编辑距离 ≤2 时给最近的一条建议。\n");
    out
}

/// 目的：把某一相的命令按分组写进手册；组内一条命令一行，写法过宽就折到第二行。
fn push_groups(out: &mut String, book: &CommandBook, phase: Phase) {
    for group in book.groups(phase) {
        out.push_str(&format!("  {}\n", group));
        for cmd in book.in_group(phase, group) {
            let alias = if cmd.aliases.is_empty() {
                String::new()
            } else {
                format!("（别名 {}）", cmd.aliases.join(" / "))
            };
            out.push_str(&command_line(&cmd.invocation(), &cmd.brief, &alias));
        }
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

/// 目的：单条命令的说明（`help <命令>`）：写法、说明、分组与相、自己的旗语与确认门槛。
pub(crate) fn detail(raw: &str) -> Option<String> {
    let cmd = book().find(raw)?;
    let mut out = format!("{}  {}\n", cmd.invocation(), cmd.brief);
    let phase = match cmd.phase {
        Phase::Boot => "启动命令",
        Phase::Core => "转录中心",
    };
    out.push_str(&format!("  {} · {}", cmd.group, phase));
    if !cmd.aliases.is_empty() {
        out.push_str(&format!("；别名 {}", cmd.aliases.join(" / ")));
    }
    if !cmd.flags.is_empty() {
        out.push_str("\n  旗语：\n");
        for flag in &cmd.flags {
            let spec = if flag.value.is_empty() {
                flag.name.clone()
            } else {
                format!("{} {}", flag.name, flag.value)
            };
            let used = super::width::cols(&spec);
            out.push_str(&format!("    {}", spec));
            let pad = if used < 22 { 22 - used } else { 2 };
            out.push_str(&" ".repeat(pad));
            out.push_str(&flag.desc);
            out.push('\n');
        }
    }
    if cmd.confirm {
        out.push_str("  确认：有写操作，需 --yes 才算数（缺省只预览或只报状态）\n");
    }
    Some(out)
}

/// 目的：Tab 补全候选：转录相命令名与别名里以输入为前缀的那些；输入为空时给全部命令名。
pub(crate) fn complete(prefix: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for cmd in book().phase(Phase::Core) {
        for name in std::iter::once(&cmd.name).chain(cmd.aliases.iter()) {
            if name.starts_with(prefix) && !out.iter().any(|c| c == name) {
                out.push(name.clone());
            }
        }
    }
    out.sort();
    out
}

/// 目的：未知命令的「你是不是想…」建议：编辑距离 ≤2 时给最近的一条（只看转录相命令）。
pub(crate) fn suggest(word: &str) -> Option<String> {
    let mut best: Option<(&str, usize)> = None;
    for cmd in book().phase(Phase::Core) {
        for name in std::iter::once(cmd.name.as_str()).chain(cmd.aliases.iter().map(String::as_str))
        {
            let d = distance(word, name);
            if d <= 2 && best.is_none_or(|(_, bd)| d < bd) {
                best = Some((name, d));
            }
        }
    }
    best.map(|(name, _)| name.to_string())
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

fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}
