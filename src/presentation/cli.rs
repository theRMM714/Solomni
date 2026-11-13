//! 目的：终端转录中心：解析命令 → 用入站能力面 → 渲染事件流。
//! 管：命令分发、启动页事实、退出或转入 Web 的去向。
//! 不管：业务决策与状态（都在各能力内部）；终端 I/O 与配色（`cli/term.rs`、`cli/theme.rs`）。
//! 联动：能力面来自 `src/capabilities/*/api.rs`；事件词汇与 `src/presentation/web/` 并列共用；模块声明见 `src/presentation.rs`。

use crate::capabilities::conductor::api::{Acted, ActionCall, Caller};
use crate::capabilities::conductor::api::{
    AgentInstance, DecisionCard, DecisionWaiter, SessionEvent, Tier,
};
use crate::capabilities::conductor::api::{Ops, Output};
use crate::capabilities::registry::api::{ModelView, ProviderView};

/// 目的：离开转录中心时的去向：退出，或转入 Web 转录中心（端口）。
pub enum CliExit {
    Exit,
    Web(u16),
}

/// 目的：转录中心主循环：交互模式用固定输入框，非交互模式保持逐行输入（输出不变）。
/// 参数：`web_default_port` 是会话内 `web` 命令不带端口时的默认值；`plain` 为真则强制纯文本；`anim` 为假则生成态不轮流刷帧。
pub fn run(ops: Ops, web_default_port: u16, plain: bool, anim: bool) -> CliExit {
    let interactive = term::init(plain, anim);
    term::set_completer(catalog::complete);
    if interactive {
        term::banner(&startup_tagline(), &startup_facts(&ops));
    } else {
        println!("Solomni 核心编排者（转录中心）");
    }
    if !interactive {
        print_roster(&ops);
    }
    if interactive {
        term::line(&term::ink(
            theme::Ink::Dim,
            "输入 help 看命令手册与旗语规范 · web 转 Web 界面 · exit 退出",
        ));
    }
    term::status(&startup_status(&ops));

    loop {
        if !interactive {
            print_menu(&ops);
        }
        let label = if interactive { "转录中心" } else { ">" };
        let Some(line) = term::ask(label) else {
            break;
        };
        let line = line.trim().to_string();
        let mut parts = line.splitn(2, ' ');
        let cmd = parts.next().unwrap_or("").to_ascii_lowercase();
        let arg = parts.next().unwrap_or("").trim().to_string();
        match cmd.as_str() {
            "single" => single_flow(&ops, &arg),
            "collab" => collab_flow(&ops, &arg),
            "proxy" => proxy_flow(&ops),
            "provider" => provider_flow(&ops, &arg),
            "model" => model_flow(&ops, &arg),
            "core" => core_flow(&ops, &arg),
            // 回档：留档（标记+折叠，可恢复）/ 删除（真的截掉）/ 恢复（删掉该标记及其后）。
            "rewind" => rewind_cmd(&ops, &arg),
            // 共享区提交历史：看 head / 列提交 / 看详情 / 非破坏性回滚。
            "work" => work_cmd(&ops, &arg),
            // 常驻服务：列清单 / 启停开关 / 调用操作（都走统一动作面）。
            "resident" => resident_cmd(&ops, &arg),
            // 隐秘字段：列声明（置值随设置面接入）。
            "secret" => secret_cmd(&ops),
            // 直接用模块工具（不经 AI）：清单与动作 id 都来自核心的动作目录。
            "module" => module_cmd(&ops, &arg),
            // 只读诊断一条虚拟机档会话的 guest（进程 / CPU / 内存 / 负载 / 挂载）。
            "diagnose" => diagnose_cmd(&ops, &arg),
            "rescan" => print_roster(&ops),
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
        let (handle, ops) = crate::tests::ops_with(vec![], vec![]);
        let exit = run(ops, 49181, true, false);
        assert!(matches!(exit, CliExit::Exit));
        drop(handle);
    }
}
