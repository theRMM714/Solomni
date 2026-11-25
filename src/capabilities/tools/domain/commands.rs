//! 目的：命令面——产品对外暴露的每一条命令都声明在 `systools/commands.yaml`（转录相 `core` 与启动相 `boot` 共用一张表）。
//! 管：读表、校验表的骨架（名字与别名唯一、分组与说明不缺）、投影成 `Op`。
//! 不管：写法（由参数声明派生，见 `op.rs`）、手册怎么排版（`presentation/cli/catalog.rs`）、命令怎么执行（各相自己的分派）。
//! 联动：`op.rs` 把这张表投影成 op 书；`presentation/cli` 的手册与补全、`src/tests/commands.rs` 的覆盖门禁都读 op 书。

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

/// 目的：这条命令要不要常驻运行时（存储自动任务、常驻服务、视图盘与围栏授权）。
/// 约束：只改一次性命令的启动路径，不改命令语义；判据只有这一处，不另立名单。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Runtime {
    /// 跑完即回：不占视图盘、不起后台任务、不拉起常驻服务、免围栏授权（只读或只动产品自己的 .home）。
    #[default]
    None,
    /// 要常驻运行时：进会话/生成、Web、常驻服务、模块工具调用。
    Services,
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
    /// 目的：手册里的一行说明。
    pub brief: String,
    /// 目的：这条命令有写操作，需要 `--yes` 才算数（缺省只预览或只报状态）。
    #[serde(default)]
    pub confirm: bool,
    /// 目的：`--json` 时的机器回执字段（给手册用；回执本身由呈现层排版）。
    #[serde(default)]
    pub json: Option<String>,
    /// 目的：参数契约（与动作面同一个形状）：位置序号、可选值、类型、缺省值都写在这里。
    ///   约束：写法（usage）、手册、补全与校验全部从它派生，不再手写字符串。
    #[serde(default)]
    pub params: Option<
        std::collections::BTreeMap<String, crate::capabilities::catalog::modules::api::Param>,
    >,
    /// 目的：最后一步落在哪（core / system / ui）：声明与解析共用，只有执行入口不同。
    #[serde(default)]
    pub executor: super::op::Executor,
    /// 目的：要不要常驻运行时（缺省 none）：分派点据此决定占不占视图盘、起不起后台任务与常驻服务。
    #[serde(default)]
    pub runtime: Runtime,
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

    /// 目的：文件顺序的全部命令（手册与投影都读它）。
    pub fn commands(&self) -> &[Command] {
        &self.commands
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
            for name in std::iter::once(&cmd.name).chain(cmd.aliases.iter()) {
                if seen.contains(name) {
                    out.push(format!("名字 {} 重复声明", name));
                } else {
                    seen.push(name.clone());
                }
            }
        }
        out
    }
}
