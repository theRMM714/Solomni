//! 目的：终端转录中心：解析命令 → 用入站面 → 渲染事件流。
//! 管：命令分发、启动页事实、退出或转入 Web 的去向。
//! 不管：业务决策与状态（都在各单元内部）；终端 I/O 与配色（`cli/term.rs`、`cli/theme.rs`）。
//! 联动：入站面来自 `src/units/*/api.rs`；事件词汇与 `src/presentation/web/` 并列共用；模块声明见 `src/presentation.rs`。

use crate::presentation::faces::Faces;
use crate::units::business::runtime::api::{Acted, ActionCall, Caller};
use crate::units::business::system::api::System;
use crate::units::business::tools::api::Op;
use crate::units::business::transcript::api::{DecisionCard, DecisionWaiter, SessionEvent};
use crate::units::business::turn::turn::api::Output;
use crate::units::pure::exec_model::api::Tier;
use crate::units::pure::registry_model::api::AgentMeta;
use crate::units::pure::registry_model::api::ModelView;
use crate::units::pure::registry_model::api::ProviderView;

/// 目的：输入框下方命令列表的一行：补全用的写法（含前导 `/`）与一句说明。
/// 约束：行类型放在这一层——`catalog` 只投影命令表，`term` 只管画，两边都不认识对方。
pub(crate) struct MenuRow {
    pub name: String,
    pub brief: String,
}

/// 目的：启动目录当外部工作区的结论：能当就记规范化后的根，不能当就记一句原因。
/// 约束：组合根在启动时算一次（`startup_workspace`），主循环与一次性命令共用；会话建好后不再变。
///   两个字段都空 = 明确不绑（`--no-workspace`），会话用产品内 work/，不必再解释。
#[derive(Debug, Clone, Default)]
pub(crate) struct Startup {
    pub(crate) workspace: Option<std::path::PathBuf>,
    pub(crate) skip: Option<String>,
}

/// 目的：把启动目录收成外部工作区——默认「在哪个目录启动就把哪个目录当工作区」。
/// 返回：可用的根，或一句为什么用不了（呈现层在建会话时原样说明，会话回退到产品内 work/）。
/// 约束：判定在后端（`WorkOps::ambient_workspace`，与建会话同一把尺子），呈现层不自己判路径；
///   `off`（`--no-workspace`）时连试探都不做；路径不是 UTF-8 时不绑（会话参数是 JSON 字符串）。
pub(crate) fn startup_workspace(faces: &Faces, off: bool) -> Startup {
    if off {
        return Startup::default();
    }
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            return Startup {
                workspace: None,
                skip: Some(format!("取不到启动目录（{}）", e)),
            };
        }
    };
    let tier = faces
        .registry
        .settings()
        .map(|s| s.tier)
        .unwrap_or(Tier::Host);
    match faces.work.ambient_workspace(&cwd, tier) {
        Ok(root) if root.to_str().is_some() => Startup {
            workspace: Some(root),
            skip: None,
        },
        Ok(_) => Startup {
            workspace: None,
            skip: Some("启动目录不是 UTF-8 路径，写不进会话参数".to_string()),
        },
        Err(why) => Startup {
            workspace: None,
            skip: Some(why),
        },
    }
}

/// 目的：离开转录中心时的去向：退出，或转入 Web 转录中心（端口）。
pub enum CliExit {
    Exit,
    Web(u16),
}

/// 目的：转录中心主循环：交互模式用固定输入框，非交互模式保持逐行输入（输出不变）。
/// 参数：`web_default_port` 是会话内 `web` 命令不带端口时的默认值；`plain` 为真则强制纯文本；`anim` 为假则生成态不轮流刷帧；
///   `startup` 是启动目录当外部工作区的结论（组合根算一次，这里只用不判）；`system` 是启动相命令的执行面
///   （与命令行共用同一份系统维护单元，见 `run_boot`）。
pub fn run(
    faces: Faces,
    web_default_port: u16,
    plain: bool,
    anim: bool,
    startup: Startup,
    system: &dyn System,
) -> CliExit {
    let interactive = term::init(plain, anim);
    // 干活时没人读键盘，Ctrl+C 得由进程级处理器接住：第一次停这一轮，没人取走再按一次才放行默认终止。
    if interactive && let Err(e) = crate::presentation::signal::install() {
        term::line(&term::ink(
            theme::Ink::Dim,
            &format!(
                "[提示] 装不上 Ctrl+C 处理器（{}）：Ctrl+C 会直接结束进程",
                e
            ),
        ));
    }
    term::set_completer(catalog::complete);
    term::set_menu(catalog::menu);
    if interactive {
        term::banner(&startup_tagline(), &startup_facts(&faces));
    } else {
        println!("Solomni 核心编排者（转录中心）");
    }
    if !interactive {
        print_roster(&faces);
    }
    if interactive {
        term::line(&term::ink(
            theme::Ink::Dim,
            "输入 help 看命令手册与旗语规范 · web 转 Web 界面 · exit 退出",
        ));
    }
    term::status(&startup_status(&faces));

    // 这一趟的呈现选项与工作区结论都是启动时定下的，整轮共用一份。
    let opts = shell::Opts {
        json: false,
        help: false,
        version: false,
        interactive: true,
        web_port: web_default_port,
        workspace: startup.workspace,
        workspace_skip: startup.skip,
    };
    loop {
        if !interactive {
            print_menu(&faces);
        }
        let label = if interactive { "转录中心" } else { ">" };
        // Ctrl+C 与 EOF 都离开主循环：交互模式下两者都是「不在键盘前 / 不想再敲」。
        let line = match term::ask_command(label) {
            term::Answer::Line(line) => line,
            term::Answer::Interrupt | term::Answer::Eof => break,
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        // 前缀即类型：/ 开头是命令（与命令行同一处判定），其余都是消息（顶层没有会话就开一条核心代理）。
        // // 转义成字面的一个斜杠（消息本身以斜杠开头时才用得上）。
        let outcome = if let Some(literal) = line.strip_prefix("//") {
            proxy_flow(&faces, system, Some(literal.to_string()), &opts)
        } else if let Some(command) = line.strip_prefix('/') {
            let command = command.trim();
            if command.is_empty() {
                term::line(&term::ink(
                    theme::Ink::Dim,
                    "[提示] / 后面写命令（/help 看全部命令）",
                ));
                continue;
            }
            shell::run_line(&faces, system, command, &opts)
        } else {
            // 顶层没有会话：纯文本直接开一条核心代理会话，这句话就是第一句。
            proxy_flow(&faces, system, Some(line.clone()), &opts)
        };
        match outcome {
            shell::Outcome::Done(_) => continue,
            shell::Outcome::Web(port) => return CliExit::Web(port),
            shell::Outcome::Exit => break,
        }
    }
    term::line("再见。");
    CliExit::Exit
}

/// 目的：转录中心里跑一条启动相命令（`--doctor` / `--gc` / `--provision` …）：与命令行同一条执行路径。
/// 参数：`op` 是命令表里那一条（决定收哪些旗语），`tail` 是命令名之后已经切好的词（交给共享解析器）。
/// 返回：这条命令的退出码——转录中心不因它退出。
/// 约束：转录中心里 `--json` 没有意义（终端要人读），`--root` 只能在启动时给（这个进程的根已定）。
pub(crate) fn run_boot(faces: &Faces, system: &dyn System, op: &Op, tail: &[&str]) -> i32 {
    let mut operands: Vec<String> = Vec::new();
    for word in tail {
        let name = word.split_once('=').map_or(*word, |(n, _)| n);
        match name {
            "--json" => {
                term::line(&term::ink(
                    theme::Ink::Dim,
                    "[提示] 转录中心里 --json 没有意义（那是给脚本的回执），这次忽略",
                ));
                continue;
            }
            "--root" => {
                term::line(&term::ink(
                    theme::Ink::Dim,
                    "[提示] --root 只能在启动时给（这个进程的根目录已经定了）",
                ));
                return 2;
            }
            _ => operands.push((*word).to_string()),
        }
    }
    let writes = !sessions_running(faces);
    let mut ask = TerminalAsk;
    let code = boot::run_asked(op, &operands, system, false, &mut ask, writes);
    if code != 0 {
        term::line(&term::ink(
            theme::Ink::Dim,
            &format!("[退出码 {}] {}", code, exit_hint(code)),
        ));
    }
    code
}

/// 目的：退出码的一句话解释（转录中心里没有人会去看退出码表）。
fn exit_hint(code: i32) -> &'static str {
    match code {
        2 => "用法不对或还差一次确认（help <命令> 看写法）",
        _ => "执行失败（上面是原因）",
    }
}

/// 目的：转录中心里的确认：以提示符就地问，只有明确的 yes 才算点头（读不到输入算没点头）。
struct TerminalAsk;

impl boot::Ask for TerminalAsk {
    fn anyone(&self) -> bool {
        true
    }

    fn confirm(&mut self, question: &str) -> bool {
        match term::ask(question) {
            term::Answer::Line(line) => matches!(
                line.trim().to_ascii_lowercase().as_str(),
                "y" | "yes" | "是"
            ),
            // Ctrl+C / 输入到尽头都算没点头。
            term::Answer::Eof | term::Answer::Interrupt => false,
        }
    }
}

/// 目的：有没有会话此刻在生成：写操作（回收 / 还原 / 补齐）在有会话在跑时只给预览。
/// 约束：读不到会话清单按「没在跑」处理——宁可多跑一次确认问话，也不让维护命令卡在只读上。
fn sessions_running(faces: &Faces) -> bool {
    faces
        .work
        .session_views(&[])
        .map(|views| views.iter().any(|v| v.running))
        .unwrap_or(false)
}

pub(crate) mod argv;
pub(crate) mod boot;
pub(crate) mod catalog;
mod commands;
mod flows;
mod gate;
mod input;
pub(crate) mod shell;
mod term;
mod theme;
mod view;
mod width;

use commands::*;
use flows::*;
use gate::*;
use view::*;

/// 目的：两个词的编辑距离（只用于旗语与名字的建议，长度都短）。
///   共用方：旗语建议（`src/presentation/cli/argv.rs`）与名字建议（`src/presentation/cli/catalog.rs`）。
pub(crate) fn distance(a: &str, b: &str) -> usize {
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

// 测试直接点名的助手：只在测试构建里外露，源码内部按子模块取。
#[cfg(test)]
pub(crate) use flows::eof_stop;
#[cfg(test)]
pub(crate) use gate::{
    NO_AGENTS, SessionAsk, create_session_action, pick_agents, pick_option, spin_frames,
    split_names, waiting_lines,
};
#[cfg(test)]
pub(crate) use input::{Editor, Key, Step, clear_history};
#[cfg(test)]
pub(crate) use width::{cols, cut, wrap};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presentation::cli::term;
    use std::io::IsTerminal;

    /// 目的：手册的三个分支都由目录渲染，不再有另一份手写文案。
    #[test]
    fn manual_comes_from_the_catalog() {
        assert!(catalog::help_text(false).contains("转录中心里的命令"));
        assert!(
            catalog::help_text(true).contains("--probe-tools"),
            "--all 展开内部旗语"
        );
        assert!(
            catalog::detail("single")
                .expect("命令表里有 single")
                .contains("single"),
        );
        assert!(catalog::detail("zzzz").is_none(), "没有的命令不给正文");
    }

    #[test]
    fn repl_runs_to_completion_when_stdin_is_closed() {
        if std::io::stdin().is_terminal() {
            return;
        }
        let (handle, faces) = crate::tests::ops_with(vec![], vec![]);
        let system = crate::tests::boot::Fake::default();
        let exit = run(faces, 49181, true, false, Startup::default(), &system);
        assert!(matches!(exit, CliExit::Exit));
        drop(handle);
    }

    /// 目的：转录中心里能跑启动相命令——同一条执行路径、同一个退出码，跑完只留一行提示。
    #[test]
    fn boot_commands_run_from_the_transcript_centre() {
        let (handle, faces) = crate::tests::ops_with(vec![], vec![]);
        let system = crate::tests::boot::Fake::default();
        let doctor = catalog::boot("--doctor").expect("命令表里有 --doctor");
        assert!(catalog::boot("single").is_none(), "转录相命令不走启动相");

        let (code, _) = term::capture(|| run_boot(&faces, &system, doctor, &[]));
        assert_eq!(code, 0, "自检只读，成功");

        // --json 在终端没有意义：提示一句、照常执行，不塞一份 JSON 进转录区。
        let (code, lines) = term::capture(|| run_boot(&faces, &system, doctor, &["--json"]));
        assert_eq!(code, 0);
        assert!(
            lines.iter().any(|l| l.contains("--json 没有意义")),
            "{lines:?}"
        );

        // --root 只能在启动时给：直接拒绝，退出码 2。
        let (code, lines) =
            term::capture(|| run_boot(&faces, &system, doctor, &["--root", "work"]));
        assert_eq!(code, 2);
        assert!(
            lines.iter().any(|l| l.contains("--root 只能在启动时给")),
            "{lines:?}"
        );

        // 用法不对时，退出码要配一句解释，而不是只丢一个数字。
        let (code, lines) = term::capture(|| run_boot(&faces, &system, doctor, &["--nope"]));
        assert_eq!(code, 2);
        assert!(lines.iter().any(|l| l.contains("[退出码 2]")), "{lines:?}");

        // 便携维护：状态模式跑得通（回执自己排版），未知模式仍退 2 并给一句解释。
        let setup = catalog::boot("--setup").expect("命令表里有 --setup");
        let (code, _) = term::capture(|| run_boot(&faces, &system, setup, &["status"]));
        assert_eq!(code, 0);
        let (code, lines) = term::capture(|| run_boot(&faces, &system, setup, &["bogus"]));
        assert_eq!(code, 2);
        assert!(lines.iter().any(|l| l.contains("[退出码 2]")), "{lines:?}");

        // 非门槛类的失败也要有解释（退出码 1）：同一句兜底文案。
        let failing = crate::tests::boot::Fake::failing("storage");
        let storage = catalog::boot("--storage").expect("命令表里有 --storage");
        let (code, lines) = term::capture(|| run_boot(&faces, &failing, storage, &[]));
        assert_eq!(code, 1);
        assert!(lines.iter().any(|l| l.contains("[退出码 1]")), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("执行失败")), "{lines:?}");
        drop(handle);
    }

    /// 目的：启动目录默认收成外部工作区；`--no-workspace` 连试探都不做。
    /// 约束：两种结果都必须是明确的——要么绑上一个根，要么给一句为什么没绑（不许沉默地回退）。
    #[test]
    fn startup_workspace_binds_the_launch_directory_unless_turned_off() {
        let (handle, faces) = crate::tests::ops_with(vec![], vec![]);
        let off = startup_workspace(&faces, true);
        assert!(off.workspace.is_none(), "--no-workspace 不绑");
        assert!(off.skip.is_none(), "明确不绑，就没有「绑不上」的理由要解释");

        let on = startup_workspace(&faces, false);
        assert!(
            on.workspace.is_some() != on.skip.is_some(),
            "要么绑上启动目录，要么说清为什么没绑：{:?} / {:?}",
            on.workspace,
            on.skip
        );
        drop(handle);
    }

    /// 目的：提示符里直接敲启动相旗语也走同一条执行路径——跑完回提示符，转录中心不退出。
    /// 约束：写操作就地一问；摇头只算预览（退出码归零），点头才真的带上确认。
    #[test]
    fn repl_runs_boot_commands_from_the_prompt() {
        let (handle, faces) = crate::tests::ops_with(vec![], vec![]);
        let system = crate::tests::boot::Fake::default();
        term::feed(&["/--setup bogus", "/--gc", "n", "/--gc", "y", "/exit"]);
        let (exit, _) =
            term::capture(|| run(faces, 49181, true, false, Startup::default(), &system));
        assert!(matches!(exit, CliExit::Exit), "exit 之后才离开转录中心");

        // 问话没人答（输入到尽头）：按没点头算，不该把维护命令一直吊在确认上。
        let (handle2, ops2) = crate::tests::ops_with(vec![], vec![]);
        term::feed(&["/--gc"]);
        let (exit, _) =
            term::capture(|| run(ops2, 49181, true, false, Startup::default(), &system));
        assert!(matches!(exit, CliExit::Exit));
        drop(handle2);
        drop(handle);
    }

    /// 目的：一行输入走的是与命令行同一个分派——版本、清单这类纯界面命令跑完回提示符。
    #[test]
    fn repl_dispatches_lines_through_the_shared_shell() {
        let (handle, faces) = crate::tests::ops_with(vec![], vec![]);
        let system = crate::tests::boot::Fake::default();
        term::feed(&["/version", "/modules", "nope", "/exit"]);
        let (exit, _) =
            term::capture(|| run(faces, 49181, true, false, Startup::default(), &system));
        assert!(
            matches!(exit, CliExit::Exit),
            "未知命令只让那一行退 2，不结束转录中心"
        );
        drop(handle);
    }

    /// 目的：单破折号的多字符词不是旗语——-help 要说清楚，而不是当成工具名或参数。
    #[test]
    fn single_dash_words_are_rejected_as_short_flags() {
        let (handle, faces) = crate::tests::ops_with(vec![], vec![]);
        let system = crate::tests::boot::Fake::default();
        let opts = shell::Opts {
            json: false,
            help: false,
            version: false,
            interactive: true,
            web_port: 49181,
            workspace: None,
            workspace_skip: None,
        };
        let (code, lines) =
            term::capture(
                || match shell::run_line(&faces, &system, "work -help", &opts) {
                    shell::Outcome::Done(code) => code,
                    _ => -1,
                },
            );
        assert_eq!(code, 2, "用法错误退 2");
        // 提示走 stdout（终端里直接看到），这里只钉退出码。
        let _ = lines;
        drop(handle);
    }

    /// 目的：前缀即类型——裸文本开一条核心代理会话，空行不离开，// 把斜杠还给消息，Ctrl+C 离开会话回顶层。
    #[test]
    fn bare_text_starts_a_proxy_session_and_double_slash_escapes() {
        let (handle, faces) = crate::tests::ops_with(vec![], vec![]);
        let system = crate::tests::boot::Fake::default();
        term::feed_answers(vec![
            term::Answer::Line("你好".to_string()),
            term::Answer::Line(String::new()),
            term::Answer::Line("//这一句以斜杠开头".to_string()),
            term::Answer::Interrupt,
            term::Answer::Line("/exit".to_string()),
        ]);
        let (exit, lines) =
            term::capture(|| run(faces, 49181, true, false, Startup::default(), &system));
        assert!(matches!(exit, CliExit::Exit), "只有 /exit 离开进程");
        assert!(
            lines.iter().any(|l| l.contains("空行不发送任何东西")),
            "空行只是提示，不再结束会话：{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.contains("已离开这条会话")),
            "会话里 Ctrl+C 是离开提示符、不是退出产品：{lines:?}"
        );
        drop(handle);
    }

    /// 目的：转录中心的确认只认明确的 yes——n / 空 / 读不到输入都算没点头。
    #[test]
    fn terminal_ask_only_takes_an_explicit_yes() {
        for (answer, expect) in [
            ("y", true),
            ("yes", true),
            ("是", true),
            ("n", false),
            ("", false),
        ] {
            term::feed(&[answer]);
            let mut ask = TerminalAsk;
            assert!(boot::Ask::anyone(&ask), "转录中心里有人可问");
            assert_eq!(
                boot::Ask::confirm(&mut ask, "确认？[y/N]"),
                expect,
                "回答 {answer:?}"
            );
        }
        // 输入到尽头（没人答）：同样不算点头。
        term::feed(&[]);
        let mut ask = TerminalAsk;
        assert!(!boot::Ask::confirm(&mut ask, "确认？[y/N]"));
    }
}
