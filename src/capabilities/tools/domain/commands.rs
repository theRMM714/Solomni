//! 目的：命令面——产品对外暴露的每一条命令都声明在 `systools/commands.yaml`（转录相 `core` 与启动相 `boot` 共用一张表）。
//! 管：读表、校验表（名字与别名唯一、分组与写法自洽）、按名查找、按相与分组取清单。
//! 不管：手册怎么排版（`presentation/cli/catalog.rs`）、命令怎么执行（`entry/args.rs` 与 `presentation/cli.rs` 各自分派）。
//! 联动：`presentation/cli` 的手册与补全、`src/entry/args.rs` 的启动相识别、`src/tests/commands.rs` 的覆盖门禁。

use serde::Deserialize;
use std::sync::OnceLock;

/// 目的：编译进来的命令表——手册与解析在任何产品根下都可用，不依赖磁盘上的 yaml 是否就位。
pub const BUILTIN: &str = include_str!("../../../../systools/commands.yaml");

/// 目的：命令在哪一相执行。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// 装配核心之前直接执行，跑完退出（用长旗语形式调用：`--gc`）。
    Boot,
    /// 转录中心里输入（`single <agent名…>`）。
    Core,
}

/// 目的：命令自己的一条旗语（只给手册与启动相识别用；参数类型与校验仍归工具表）。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandFlag {
    pub name: String,
    /// 目的：占位符（空 = 布尔旗），如 `<guest>`。
    #[serde(default)]
    pub value: String,
    pub desc: String,
}

/// 目的：一条命令的声明。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub group: String,
    pub phase: Phase,
    /// 目的：位置参数写法（空 = 不收位置参数），如 `<agent名…>`。
    #[serde(default)]
    pub usage: String,
    /// 目的：手册里的一行说明。
    pub brief: String,
    /// 目的：这条命令有写操作，需要 `--yes` 才算数（缺省只预览或只报状态）。
    #[serde(default)]
    pub confirm: bool,
    #[serde(default)]
    pub flags: Vec<CommandFlag>,
}

impl Command {
    /// 目的：这条命令的调用写法：启动相加长旗前缀，转录相就是名字本身；带上位置参数写法。
    pub fn invocation(&self) -> String {
        let head = match self.phase {
            Phase::Boot => format!("--{}", self.name),
            Phase::Core => self.name.clone(),
        };
        if self.usage.is_empty() {
            head
        } else {
            format!("{} {}", head, self.usage)
        }
    }
}

/// 目的：整张命令表（文件顺序即手册顺序；分组按首次出现排）。
#[derive(Debug, Clone, Default)]
pub struct CommandBook {
    commands: Vec<Command>,
}

/// 文件形状：只有一张 `commands` 清单。
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    commands: Vec<Command>,
}

impl CommandBook {
    /// 目的：从 yaml 文本建表，并当场校验（结构错与内容错都如实报错，不静默放过）。
    pub fn from_yaml(text: &str) -> Result<CommandBook, String> {
        let file: File = yaml_serde::from_str(text)
            .map_err(|e| format!("命令表非法（systools/commands.yaml）：{}", e))?;
        let book = CommandBook {
            commands: file.commands,
        };
        let problems = book.problems();
        if problems.is_empty() {
            Ok(book)
        } else {
            Err(format!(
                "命令表自相矛盾（systools/commands.yaml）：{}",
                problems.join("；")
            ))
        }
    }

    /// 目的：产品内置的命令表（编译期带入同一份文件，解析一次后复用）。
    pub fn builtin() -> Result<&'static CommandBook, String> {
        static ONCE: OnceLock<Result<CommandBook, String>> = OnceLock::new();
        match ONCE.get_or_init(|| CommandBook::from_yaml(BUILTIN)) {
            Ok(book) => Ok(book),
            Err(e) => Err(e.clone()),
        }
    }

    /// 目的：按名字或别名查一条命令；接受前导 `--`（启动相写法）与前导 `/`（`/help`）。
    pub fn find(&self, raw: &str) -> Option<&Command> {
        let name = raw.trim();
        let name = name.strip_prefix("--").unwrap_or(name);
        let name = name.strip_prefix('/').unwrap_or(name);
        self.commands
            .iter()
            .find(|c| c.name == name || c.aliases.iter().any(|a| a.trim_start_matches('/') == name))
    }

    /// 目的：某一相里的分组，按首次出现排序。
    pub fn groups(&self, phase: Phase) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for cmd in self.commands.iter().filter(|c| c.phase == phase) {
            if !out.contains(&cmd.group.as_str()) {
                out.push(&cmd.group);
            }
        }
        out
    }

    /// 目的：某一相、某一组里的命令（文件顺序）。
    pub fn in_group(&self, phase: Phase, group: &str) -> Vec<&Command> {
        self.commands
            .iter()
            .filter(|c| c.phase == phase && c.group == group)
            .collect()
    }

    /// 目的：某一相的全部命令。
    pub fn phase(&self, phase: Phase) -> Vec<&Command> {
        self.commands.iter().filter(|c| c.phase == phase).collect()
    }

    /// 目的：表自身的矛盾（空 = 一切正常）：装配期自检与覆盖门禁读它。
    pub fn problems(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        for cmd in &self.commands {
            if cmd.name.trim().is_empty() {
                out.push("有条命令没写名字".to_string());
                continue;
            }
            if cmd.group.trim().is_empty() {
                out.push(format!("命令 {} 没写分组", cmd.name));
            }
            if cmd.brief.trim().is_empty() {
                out.push(format!("命令 {} 没写一句话说明", cmd.name));
            }
            if cmd.usage.contains('\n') {
                out.push(format!("命令 {} 的写法不该换行", cmd.name));
            }
            for name in std::iter::once(&cmd.name).chain(cmd.aliases.iter()) {
                if seen.contains(name) {
                    out.push(format!("名字 {} 重复声明", name));
                } else {
                    seen.push(name.clone());
                }
            }
            let mut flags: Vec<&str> = Vec::new();
            for flag in &cmd.flags {
                if !flag.name.starts_with("--") {
                    out.push(format!("命令 {} 的旗语 {} 要长旗形式", cmd.name, flag.name));
                }
                if flags.contains(&flag.name.as_str()) {
                    out.push(format!("命令 {} 的旗语 {} 重复", cmd.name, flag.name));
                }
                flags.push(&flag.name);
            }
        }
        out
    }
}
