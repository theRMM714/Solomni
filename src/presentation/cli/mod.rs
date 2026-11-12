//! 终端转录中心：解析命令 → 用入站能力面 → 渲染事件流。
//! 只做解析与渲染，不做业务决策；Web 前端与它并列，共用同一能力面与事件词汇。

use crate::capabilities::conductor::api::{Acted, ActionCall, Caller};
use crate::capabilities::conductor::api::{
    AgentInstance, DecisionCard, DecisionWaiter, SessionEvent, Tier,
};
use crate::capabilities::conductor::api::{Ops, Output};
use crate::capabilities::registry::api::{ModelView, ProviderView};
use std::io::Write;

/// 目的：离开转录中心时的去向：退出，或转入 Web 转录中心（端口）。
pub enum CliExit {
    Exit,
    Web(u16),
}

pub fn run(ops: Ops, web_default_port: u16) -> CliExit {
    println!("Solomni 核心编排者（转录中心）");
    print_roster(&ops);

    loop {
        print_menu(&ops);
        print!("> ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
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
            // 转入 Web 转录中心：接受 webui / -webUI（启动参数也这么写），可选端口。
            "webui" | "-webui" | "web" | "-web" => {
                let port = arg.parse::<u16>().unwrap_or(web_default_port);
                return CliExit::Web(port);
            }
            "exit" => break,
            "" => continue,
            _ => println!("[提示] 未知命令 {}（Web 界面用 webui；退出用 exit）", cmd),
        }
    }
    println!("再见。");
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

/// 直接用模块工具（不经 AI）：`module` 列清单，`module <模块id>.<工具名> [json 参数]` 跑一次。
/// 清单来自核心的动作目录（`module.<模块id>.<工具名>`），所以与 Web 看到的是同一份事实。
mod commands;
mod flows;
mod gate;
mod view;

use commands::*;
use flows::*;
use gate::*;
use view::*;

// 测试直接点名的助手：只在测试构建里外露，源码内部按子模块取。
#[cfg(test)]
pub(crate) use flows::eof_stop;
#[cfg(test)]
pub(crate) use gate::{
    create_session_action, pick_agents, pick_option, split_names, waiting_lines, NO_AGENTS,
};
