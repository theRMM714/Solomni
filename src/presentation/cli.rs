//! 目的：终端转录中心：解析命令 → 用入站能力面 → 渲染事件流。
//! 管：命令分发、启动页事实、退出或转入 Web 的去向。
//! 不管：业务决策与状态（都在各能力内部）；终端 I/O 与配色（`cli/term.rs`、`cli/theme.rs`）。
//! 联动：能力面来自 `src/capabilities/*/api.rs`；事件词汇与 `src/presentation/web/` 并列共用；模块声明见 `src/presentation.rs`。

use crate::capabilities::catalog::registry::api::AgentMeta;
use crate::capabilities::catalog::registry::api::{ModelView, ProviderView};
use crate::capabilities::runtime::api::Output;
use crate::capabilities::runtime::api::{Acted, ActionCall, Caller};
use crate::capabilities::runtime::api::{DecisionCard, DecisionWaiter, SessionEvent, Tier};
use crate::capabilities::system::api::System;
use crate::capabilities::tools::api::Op;
use crate::presentation::faces::Faces;

/// 目的：离开转录中心时的去向：退出，或转入 Web 转录中心（端口）。
pub enum CliExit {
    Exit,
    Web(u16),
}

/// 目的：转录中心主循环：交互模式用固定输入框，非交互模式保持逐行输入（输出不变）。
/// 参数：`web_default_port` 是会话内 `web` 命令不带端口时的默认值；`plain` 为真则强制纯文本；`anim` 为假则生成态不轮流刷帧；
///   `system` 是启动相命令的执行面（与命令行共用同一份系统维护能力，见 `run_boot`）。
pub fn run(
    faces: Faces,
    web_default_port: u16,
    plain: bool,
    anim: bool,
    system: &dyn System,
) -> CliExit {
    let interactive = term::init(plain, anim);
    term::set_completer(catalog::complete);
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

    loop {
        if !interactive {
            print_menu(&faces);
        }
        let label = if interactive { "转录中心" } else { ">" };
        let Some(line) = term::ask(label) else {
            break;
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        // 一行输入的归属（命令 / 模块工具 / 都不认识）只在 shell 里判定一次，命令行也走同一处。
        let opts = shell::Opts {
            json: false,
            help: false,
            version: false,
            interactive: true,
            web_port: web_default_port,
        };
        match shell::run_line(&faces, system, &line, &opts) {
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
            Some(line) => matches!(
                line.trim().to_ascii_lowercase().as_str(),
                "y" | "yes" | "是"
            ),
            None => false,
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
    NO_AGENTS, create_session_action, pick_agents, pick_option, spin_frames, split_names,
    waiting_lines,
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
        let exit = run(faces, 49181, true, false, &system);
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

    /// 目的：提示符里直接敲启动相旗语也走同一条执行路径——跑完回提示符，转录中心不退出。
    /// 约束：写操作就地一问；摇头只算预览（退出码归零），点头才真的带上确认。
    #[test]
    fn repl_runs_boot_commands_from_the_prompt() {
        let (handle, faces) = crate::tests::ops_with(vec![], vec![]);
        let system = crate::tests::boot::Fake::default();
        term::feed(&["--setup bogus", "--gc", "n", "--gc", "y", "exit"]);
        let (exit, _) = term::capture(|| run(faces, 49181, true, false, &system));
        assert!(matches!(exit, CliExit::Exit), "exit 之后才离开转录中心");

        // 问话没人答（输入到尽头）：按没点头算，不该把维护命令一直吊在确认上。
        let (handle2, ops2) = crate::tests::ops_with(vec![], vec![]);
        term::feed(&["--gc"]);
        let (exit, _) = term::capture(|| run(ops2, 49181, true, false, &system));
        assert!(matches!(exit, CliExit::Exit));
        drop(handle2);
        drop(handle);
    }

    /// 目的：一行输入走的是与命令行同一个分派——版本、清单这类纯界面命令跑完回提示符。
    #[test]
    fn repl_dispatches_lines_through_the_shared_shell() {
        let (handle, faces) = crate::tests::ops_with(vec![], vec![]);
        let system = crate::tests::boot::Fake::default();
        term::feed(&["version", "modules", "nope", "exit"]);
        let (exit, _) = term::capture(|| run(faces, 49181, true, false, &system));
        assert!(
            matches!(exit, CliExit::Exit),
            "未知命令只让那一行退 2，不结束转录中心"
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
