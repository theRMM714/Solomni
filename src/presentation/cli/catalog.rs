//! 目的：命令目录——手册、菜单、补全与「未知命令」建议都从这一张表生成。
//! 管：命令名 / 别名 / 参数形状 / 分组 / 一句话说明，以及帮助正文、补全候选与编辑距离建议。
//! 不管：命令怎么执行（`cli.rs`）、终端与配色怎么画（`term.rs`、`view.rs`）。
//! 联动：`cli.rs` 渲染 help 与提示；`term.rs` 的 Tab 补全取 `complete`；旗语正文取 `argv`。

/// 目的：一条命令的规格：名字、参数形状、别名、分组与一句话说明。
pub(crate) struct Cmd {
    pub(crate) name: &'static str,
    pub(crate) args: &'static str,
    pub(crate) aliases: &'static [&'static str],
    pub(crate) desc: &'static str,
    pub(crate) group: &'static str,
}

const GROUPS: [&str; 7] = ["会话", "转录", "注册处", "模块", "后台", "界面", "系统"];

/// 目的：全部会话内命令：`help` 与补全都读这一份，新增命令只改这里。
const COMMANDS: &[Cmd] = &[
    Cmd {
        name: "single",
        args: "<agent名…>",
        aliases: &[],
        desc: "发起单 agent 会话，进入转录提示符",
        group: "会话",
    },
    Cmd {
        name: "collab",
        args: "<agent名…|?>",
        aliases: &[],
        desc: "多 agent 协作，由同一个主持人收口",
        group: "会话",
    },
    Cmd {
        name: "proxy",
        args: "",
        aliases: &[],
        desc: "决定权整块交给核心（不看单席约束）",
        group: "会话",
    },
    Cmd {
        name: "rewind",
        args: "<会话> archive|delete <行id> | restore <标记id>",
        aliases: &[],
        desc: "回档：留档、删除或恢复到某条标记",
        group: "转录",
    },
    Cmd {
        name: "work",
        args: "<会话> log|show|restore <提交号> [--yes]",
        aliases: &[],
        desc: "共享区提交历史：看 head、看详情、非破坏性回滚",
        group: "转录",
    },
    Cmd {
        name: "diagnose",
        args: "<会话>",
        aliases: &[],
        desc: "只读诊断一条虚拟机档会话（进程 / CPU / 内存 / 挂载）",
        group: "转录",
    },
    Cmd {
        name: "provider",
        args: "list|add|rm|discover",
        aliases: &[],
        desc: "供应商登记与连通性测试",
        group: "注册处",
    },
    Cmd {
        name: "model",
        args: "list|add|rm",
        aliases: &[],
        desc: "模型登记与默认模型切换",
        group: "注册处",
    },
    Cmd {
        name: "core",
        args: "<模型id>",
        aliases: &[],
        desc: "设置核心默认模型",
        group: "注册处",
    },
    Cmd {
        name: "module",
        args: "<模块id.工具名> [json]",
        aliases: &[],
        desc: "不经 AI 直接调用模块工具（动作 id 来自动作目录）",
        group: "模块",
    },
    Cmd {
        name: "resident",
        args: "list|start|stop|enable|disable|call <模块id> <服务名> [操作名] [json]",
        aliases: &[],
        desc: "常驻服务：列清单、启停、调用操作",
        group: "后台",
    },
    Cmd {
        name: "secret",
        args: "",
        aliases: &[],
        desc: "列密钥声明（值不回显）",
        group: "后台",
    },
    Cmd {
        name: "rescan",
        args: "",
        aliases: &[],
        desc: "重扫模块与能力注册表",
        group: "后台",
    },
    Cmd {
        name: "web",
        args: "[端口]",
        aliases: &[],
        desc: "转入 Web 转录中心（Ctrl+C 回到这里）",
        group: "界面",
    },
    Cmd {
        name: "help",
        args: "[命令|--all]",
        aliases: &["/help", "?"],
        desc: "命令手册与旗语规范",
        group: "系统",
    },
    Cmd {
        name: "version",
        args: "",
        aliases: &[],
        desc: "版本与构建档位",
        group: "系统",
    },
    Cmd {
        name: "exit",
        args: "",
        aliases: &[],
        desc: "退出转录中心，回到系统 shell",
        group: "系统",
    },
];

/// 目的：非交互模式每轮重印的一行菜单（完整参数看 `help`）。
pub(crate) fn menu_text() -> String {
    let groups: Vec<String> = GROUPS
        .iter()
        .map(|group| {
            COMMANDS
                .iter()
                .filter(|c| c.group == *group)
                .map(|c| c.name)
                .collect::<Vec<_>>()
                .join("/")
        })
        .filter(|row| !row.is_empty())
        .collect();
    format!(
        "命令：{}（help 看用法与旗语；help <命令> 看单条；Tab 补全）",
        groups.join(" · ")
    )
}

/// 目的：完整手册正文：用法、旗语表、按分组的命令表与语法规则。
/// 参数：`all` 为真时连内部旗语一起列。
pub(crate) fn help_text(all: bool) -> String {
    let mut out = String::new();
    out.push_str("Solomni —— 终端转录中心\n\n");
    out.push_str("用法：solomni [全局旗语]\n在转录中心里输入：<命令> [参数]（下表；别名可当简写）\n\n全局旗语\n");
    for row in super::argv::flag_rows(all) {
        out.push_str(&row);
        out.push('\n');
    }
    out.push_str("\n命令（在转录中心里输入；别名也可用）\n");
    for group in GROUPS {
        let rows: Vec<&Cmd> = COMMANDS.iter().filter(|c| c.group == group).collect();
        if rows.is_empty() {
            continue;
        }
        out.push_str(&format!("  {}\n", group));
        for cmd in rows {
            let head = if cmd.args.is_empty() {
                cmd.name.to_string()
            } else {
                format!("{} {}", cmd.name, cmd.args)
            };
            let alias = if cmd.aliases.is_empty() {
                String::new()
            } else {
                format!("（别名 {}）", cmd.aliases.join(" / "))
            };
            let used = super::width::cols(&head);
            if used <= 40 {
                let mut line = format!("    {}", head);
                line.push_str(&" ".repeat(44 - used));
                line.push_str(cmd.desc);
                line.push_str(&alias);
                out.push_str(&line);
                out.push('\n');
            } else {
                out.push_str(&format!("    {}\n      {}{}\n", head, cmd.desc, alias));
            }
        }
    }
    out.push_str("\n语法规则\n");
    out.push_str("  1 旗语只出现在 solomni 主程序层；命令内部只用子命令与位置参数。\n");
    out.push_str("  2 短旗只留 -h、-v；其余一律长旗，且 --x v 与 --x=v 等价。\n");
    out.push_str("  3 布尔旗不带值；关掉一个开关用对应的 --no-xxx，不写 --xxx=false。\n");
    out.push_str("  4 `--` 之后全部当位置参数（参数里有空格或连字符时用）。\n");
    out.push_str("  5 未知旗语直接报错，编辑距离 ≤2 时给最近的一条建议。\n");
    out
}

/// 目的：单条命令的说明（`help <命令>`）：用法、说明与别名。
pub(crate) fn detail(name: &str) -> Option<String> {
    let name = name.trim().trim_start_matches('/');
    let cmd = COMMANDS
        .iter()
        .find(|c| c.name == name || c.aliases.contains(&name))?;
    let usage = if cmd.args.is_empty() {
        cmd.name.to_string()
    } else {
        format!("{} {}", cmd.name, cmd.args)
    };
    let mut out = format!("{}  {}\n  {}", usage, cmd.desc, cmd.group);
    if !cmd.aliases.is_empty() {
        out.push_str(&format!("；别名 {}", cmd.aliases.join(" / ")));
    }
    Some(out)
}

/// 目的：Tab 补全候选：命令名与别名里以输入为前缀的那些；输入为空时给全部命令名。
pub(crate) fn complete(prefix: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for cmd in COMMANDS {
        for name in std::iter::once(cmd.name).chain(cmd.aliases.iter().copied()) {
            if name.starts_with(prefix) && !out.iter().any(|c| c == name) {
                out.push(name.to_string());
            }
        }
    }
    out.sort();
    out
}

/// 目的：未知命令的「你是不是想…」建议：编辑距离 ≤2 时给最近的一条。
pub(crate) fn suggest(word: &str) -> Option<&'static str> {
    let mut best: Option<(&'static str, usize)> = None;
    for cmd in COMMANDS {
        for name in std::iter::once(cmd.name).chain(cmd.aliases.iter().copied()) {
            let d = distance(word, name);
            if d <= 2 && best.is_none_or(|(_, bd)| d < bd) {
                best = Some((name, d));
            }
        }
    }
    best.map(|(name, _)| name)
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
