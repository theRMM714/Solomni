//! 目的：终端转录中心：解析命令 → 用入站能力面 → 渲染事件流。
//! 管：命令分发、启动页事实、退出或转入 Web 的去向。
//! 不管：业务决策与状态（都在各能力内部）；终端 I/O 与配色（`cli/term.rs`、`cli/theme.rs`）。
//! 联动：能力面来自 `src/capabilities/*/api.rs`；事件词汇与 `src/presentation/web/` 并列共用；模块声明见 `src/presentation.rs`。

use crate::capabilities::registry::api::{ModelView, ProviderView};
use crate::capabilities::runtime::api::Output;
use crate::capabilities::runtime::api::{Acted, ActionCall, Caller};
use crate::capabilities::runtime::api::{DecisionCard, DecisionWaiter, SessionEvent, Tier};
use crate::capabilities::session::api::AgentMeta;
use crate::capabilities::system::api::System;
use crate::capabilities::tools::api::Command;
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
        let mut parts = line.splitn(2, ' ');
        let cmd = parts.next().unwrap_or("").to_ascii_lowercase();
        let arg = parts.next().unwrap_or("").trim().to_string();
        // 启动相命令（命令表里 `phase: boot` 的那些）：与命令行同一个执行入口，跑完回提示符。
        if let Some(command) = catalog::boot(&cmd) {
            run_boot(&faces, system, command, &arg);
            continue;
        }
        match cmd.as_str() {
            "single" => single_flow(&faces, &arg),
            "collab" => collab_flow(&faces, &arg),
            "proxy" => proxy_flow(&faces),
            "provider" => provider_flow(&faces, &arg),
            "model" => model_flow(&faces, &arg),
            "core" => core_flow(&faces, &arg),
            // 回档：留档（标记+折叠，可恢复）/ 删除（真的截掉）/ 恢复（删掉该标记及其后）。
            "rewind" => rewind_cmd(&faces, &arg),
            // 共享区提交历史：看 head / 列提交 / 看详情 / 非破坏性回滚。
            "work" => work_cmd(&faces, &arg),
            // 常驻服务：列清单 / 启停开关 / 调用操作（都走统一动作面）。
            "resident" => resident_cmd(&faces, &arg),
            // 隐秘字段：列声明（置值随设置面接入）。
            "secret" => secret_cmd(&faces),
            // 直接用模块工具（不经 AI）：清单与动作 id 都来自核心的动作目录。
            "module" => module_cmd(&faces, &arg),
            // 只读诊断一条虚拟机档会话的 guest（进程 / CPU / 内存 / 负载 / 挂载）。
            "diagnose" => diagnose_cmd(&faces, &arg),
            "rescan" => print_roster(&faces),
            "help" | "/help" | "?" => help_cmd(&arg),
            "version" => term::line(&catalog::version_text()),
            // 转入 Web 转录中心（唯一入口：启动参数用 --web，会话内用 web）。
            "web" => {
                let port = arg.parse::<u16>().unwrap_or(web_default_port);
                return CliExit::Web(port);
            }
            "exit" => break,
            "" => continue,
            _ => match catalog::suggest(&cmd) {
                Some(near) => term::line(&format!(
                    "[提示] 未知命令 {}；你是不是想用 {}？（help 看全部命令）",
                    cmd, near
                )),
                None => term::line(&format!(
                    "[提示] 未知命令 {}（help 看全部命令；转 Web 用 web；退出用 exit）",
                    cmd
                )),
            },
        }
    }
    term::line("再见。");
    CliExit::Exit
}

/// 目的：转录中心里跑一条启动相命令（`--doctor` / `--gc` / `--provision` …）：与命令行同一条执行路径。
/// 参数：`command` 来自命令表，`tail` 是命令名之后的原文（与命令名之间只按空格切，不再做语法）。
/// 返回：这条命令的退出码——转录中心不因它退出。
/// 约束：转录中心里 `--json` 没有意义（终端要人读），`--root` 只能在启动时给（这个进程的根已定）。
pub(crate) fn run_boot(faces: &Faces, system: &dyn System, command: &Command, tail: &str) -> i32 {
    let mut operands: Vec<String> = Vec::new();
    for word in tail.split_whitespace() {
        let name = word.split_once('=').map_or(word, |(n, _)| n);
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
            _ => operands.push(word.to_string()),
        }
    }
    let writes = !sessions_running(faces);
    let mut ask = TerminalAsk;
    let code = boot::run_asked(&command.name, &operands, system, false, &mut ask, writes);
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

/// 目的：打印手册：无参数给全量，`--all` 连内部旗语一起列，带命令名只讲那一条。
fn help_cmd(arg: &str) {
    let arg = arg.trim();
    if arg.is_empty() {
        term::line(&catalog::help_text(false));
    } else if arg == "--all" || arg == "-a" {
        term::line(&catalog::help_text(true));
    } else {
        match catalog::detail(arg) {
            Some(text) => term::line(&text),
            None => match catalog::suggest(arg) {
                Some(near) => {
                    term::line(&format!("[提示] 没有命令 {}；你是不是想查 {}？", arg, near))
                }
                None => term::line(&format!("[提示] 没有命令 {}；help 看全部命令", arg)),
            },
        }
    }
}

/// 目的：把用户写的模块工具名规整成动作 id——`<模块id>.<工具名>` 与完整的 `module.<模块id>.<工具名>` 都收。
pub(crate) fn module_action_id(arg: &str) -> String {
    let a = arg.trim();
    if a.starts_with("module.") {
        a.to_string()
    } else {
        format!("module.{}", a)
    }
}

pub(crate) mod argv;
pub(crate) mod boot;
pub(crate) mod catalog;
mod commands;
mod flows;
mod gate;
mod input;
mod term;
mod theme;
mod view;
mod width;

use commands::*;
use flows::*;
use gate::*;
use view::*;

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

    #[test]
    fn help_cmd_prints_manual_branches() {
        let (_, lines) = term::capture(|| help_cmd(""));
        assert!(lines.iter().any(|l| l.contains("命令")), "无参数给全量手册");

        let (_, lines) = term::capture(|| help_cmd("--all"));
        assert!(
            lines.iter().any(|l| l.contains("--probe-tools")),
            "--all 展开内部旗语"
        );

        let (_, lines) = term::capture(|| help_cmd("single"));
        assert!(lines.iter().any(|l| l.contains("single")));

        let (_, lines) = term::capture(|| help_cmd("singel"));
        assert!(lines.iter().any(|l| l.contains("你是不是想查 single")));

        let (_, lines) = term::capture(|| help_cmd("zzzz"));
        assert!(lines.iter().any(|l| l.contains("没有命令 zzzz")));
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

        let (code, _) = term::capture(|| run_boot(&faces, &system, doctor, ""));
        assert_eq!(code, 0, "自检只读，成功");

        // --json 在终端没有意义：提示一句、照常执行，不塞一份 JSON 进转录区。
        let (code, lines) = term::capture(|| run_boot(&faces, &system, doctor, "--json"));
        assert_eq!(code, 0);
        assert!(
            lines.iter().any(|l| l.contains("--json 没有意义")),
            "{lines:?}"
        );

        // --root 只能在启动时给：直接拒绝，退出码 2。
        let (code, lines) = term::capture(|| run_boot(&faces, &system, doctor, "--root work"));
        assert_eq!(code, 2);
        assert!(
            lines.iter().any(|l| l.contains("--root 只能在启动时给")),
            "{lines:?}"
        );

        // 用法不对时，退出码要配一句解释，而不是只丢一个数字。
        let (code, lines) = term::capture(|| run_boot(&faces, &system, doctor, "--nope"));
        assert_eq!(code, 2);
        assert!(lines.iter().any(|l| l.contains("[退出码 2]")), "{lines:?}");

        // 便携维护：状态模式跑得通（回执自己排版），未知模式仍退 2 并给一句解释。
        let setup = catalog::boot("--setup").expect("命令表里有 --setup");
        let (code, _) = term::capture(|| run_boot(&faces, &system, setup, "status"));
        assert_eq!(code, 0);
        let (code, lines) = term::capture(|| run_boot(&faces, &system, setup, "bogus"));
        assert_eq!(code, 2);
        assert!(lines.iter().any(|l| l.contains("[退出码 2]")), "{lines:?}");

        // 非门槛类的失败也要有解释（退出码 1）：同一句兜底文案。
        let failing = crate::tests::boot::Fake::failing("storage");
        let storage = catalog::boot("--storage").expect("命令表里有 --storage");
        let (code, lines) = term::capture(|| run_boot(&faces, &failing, storage, ""));
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
        let (exit, lines) = term::capture(|| run(faces, 49181, true, false, &system));
        assert!(matches!(exit, CliExit::Exit), "exit 之后才离开转录中心");
        assert!(
            lines.iter().any(|l| l.contains("[退出码 2]")),
            "用法不对要配一句解释：{lines:?}"
        );
        assert!(
            !lines.iter().any(|l| l.contains("[退出码 0]")),
            "成功不留退出码：{lines:?}"
        );

        // 问话没人答（输入到尽头）：按没点头算，不该把维护命令一直吊在确认上。
        let (handle2, ops2) = crate::tests::ops_with(vec![], vec![]);
        term::feed(&["--gc"]);
        let (exit, _) = term::capture(|| run(ops2, 49181, true, false, &system));
        assert!(matches!(exit, CliExit::Exit));
        drop(handle2);
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
