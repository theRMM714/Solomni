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
pub fn run(ops: Ops, web_default_port: u16) -> CliExit {
    let interactive = term::init(false);
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
            "输入 help 看全量命令与模块清单 · webui 转 Web 界面 · exit 退出",
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
            "help" | "/help" | "?" => print_menu(&ops),
            // 转入 Web 转录中心：接受 webui / -webUI（启动参数也这么写），可选端口。
            "webui" | "-webui" | "web" | "-web" => {
                let port = arg.parse::<u16>().unwrap_or(web_default_port);
                return CliExit::Web(port);
            }
            "exit" => break,
            "" => continue,
            _ => term::line(&format!(
                "[提示] 未知命令 {}（Web 界面用 webui；退出用 exit）",
                cmd
            )),
        }
    }
    term::line("再见。");
    CliExit::Exit
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
    NO_AGENTS, create_session_action, pick_agents, pick_option, split_names, waiting_lines,
};
#[cfg(test)]
pub(crate) use input::{Editor, Key, Step, clear_history};
#[cfg(test)]
pub(crate) use width::{cols, cut, wrap};
