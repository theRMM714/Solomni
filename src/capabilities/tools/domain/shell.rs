//! 目的：执行类系统工具 `shell` 的纯规则：参数契约、命令长度、scope / timeout 解析与失败回执的 scope 提示。
//! 管：一次 shell 调用的入参到 `ShellCall` 的解析；命令过长 / scope 非法 / timeout 越界的事实与文案。
//! 不管：进程拉起、围栏安装、guest 执行与输出截断（在 kernel 的 ProcessRunner / vm 的 guest）；围栏派生在 domain/fence.rs。
//! 联动：由 collab 的成员循环在派发 `shell` 前调用；声明在 systools/tools.yaml，经 tools::api 导出。

use crate::capabilities::permission::api::{CallScope, Decision};
use crate::capabilities::tools::domain::schema::ToolSchema;
use crate::capabilities::tools::domain::systool::{SHELL, arg_fault_text};
use crate::kernel::api::ToolOutcome;
use std::time::Duration;

/// 目的：单条 shell 命令的字符上限；超过就要求模型先把脚本写进沙箱再执行（命令不拼进命令行）。
pub const MAX_COMMAND_CHARS: usize = 8_000;

/// 目的：一次已校验的 shell 调用——脚本、scope 请求与本次超时。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellCall {
    /// 目的：要执行的命令原文（模型写的本环境 shell 语法）。
    pub script: String,
    /// 目的：本次调用的 reach 请求；缺省 sandbox。
    pub scope: CallScope,
    /// 目的：本次执行的超时；到点连根杀整棵进程树。
    pub timeout: Duration,
}

/// 目的：按工具声明校验并解析一次 shell 调用；参数不符 / 命令过长 / scope 非法都如实报事实。
/// 参数：`schema` = systools/tools.yaml 里的 shell 声明；`args_json` = 模型给的参数对象。
/// 返回：已补缺省（scope=sandbox、timeout_secs=120）的调用。
/// 错误：参数不是 JSON、缺必填 / 类型不符、命令超过 `MAX_COMMAND_CHARS`、scope 不是 sandbox / external。
pub fn parse(schema: &ToolSchema, args_json: &str) -> Result<ShellCall, String> {
    let mut args: serde_json::Value =
        serde_json::from_str(args_json).map_err(|e| format!("参数不是合法 JSON：{}", e))?;
    if let Err(fault) = schema.check(&args) {
        return Err(arg_fault_text(SHELL, schema, &fault));
    }
    schema.apply_defaults(&mut args);
    let script = args
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let count = script.chars().count();
    if count > MAX_COMMAND_CHARS {
        return Err(format!(
            "命令太长（{} 字符，上限 {}）：请先用 write 把脚本写进你的沙箱，再执行那个脚本（这条命令没有执行）。",
            count, MAX_COMMAND_CHARS
        ));
    }
    let scope = CallScope::from_request(args.get("scope").and_then(|v| v.as_str()))?;
    let secs = args
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(120);
    Ok(ShellCall {
        script,
        scope,
        timeout: Duration::from_secs(secs),
    })
}

/// 目的：给一次失败的 sandbox shell 回执补 scope 提示——只在围栏边界说明出现时补，不猜别的失败原因。
/// 参数：`external` = 当前档位对 external scope 的处置（据此说“会先问 / 直接执行 / 需先切档”）。
/// 约束：不自动重试、不自动升级 scope；回执仍是失败。
pub fn with_scope_hint(outcome: ToolOutcome, scope: CallScope, external: Decision) -> ToolOutcome {
    if outcome.ok
        || scope != CallScope::Sandbox
        || !outcome
            .output
            .contains(crate::kernel::api::FENCE_BOUNDARY_MARK)
    {
        return outcome;
    }
    let hint = match external {
        Decision::Deny => {
            "本次 scope=sandbox 只能访问你的私有沙箱；要访问沙箱外，请让用户把会话档位调到「外部可达（逐次确认）」或更高。"
        }
        Decision::Ask => {
            "本次 scope=sandbox 只能访问你的私有沙箱；需要访问沙箱外，请用 scope=external 重试（档位会先问你）。"
        }
        Decision::Allow => {
            "本次 scope=sandbox 只能访问你的私有沙箱；需要访问沙箱外，请用 scope=external 重试（全可达档会直接执行，仍受全局上限）。"
        }
    };
    let mut output = outcome.output;
    output.push_str("\n[scope] ");
    output.push_str(hint);
    ToolOutcome { ok: false, output }
}

/// 目的：shell 回执的状态行分类——退出码之外的三种收尾（超时 / 用户停止 / 被信号结束）。
/// 约束：与退出码互补；状态行是「上一行是状态、末行是 cwd·耗时」契约的一半。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellStatus {
    /// 目的：正常返回了退出码。
    Exit,
    /// 目的：到点被连根杀树。
    Timeout,
    /// 目的：用户停止。
    Stopped,
    /// 目的：被信号结束，拿不到退出码。
    Signal,
}

impl ShellStatus {
    /// 目的：给状态栏 / JSON 用的稳定取值（exit / timeout / stopped / signal）。
    pub fn as_str(self) -> &'static str {
        match self {
            ShellStatus::Exit => "exit",
            ShellStatus::Timeout => "timeout",
            ShellStatus::Stopped => "stopped",
            ShellStatus::Signal => "signal",
        }
    }
}

/// 目的：从 shell 回执里取状态（取最后一行的 `[shell] ` 收尾行）；没有收尾行（没执行过）给 None。
/// 约束：只认本模块与 ProcessTexts 产出的形状；命令自己的输出伪造同形行时，更靠后的真行会取代它。
pub fn status(output: &str) -> Option<ShellStatus> {
    output.lines().rev().find_map(|line| {
        let rest = line.trim().strip_prefix("[shell] ")?;
        if rest.starts_with("退出码未知") {
            return Some(ShellStatus::Signal);
        }
        if rest.starts_with("退出码 ") {
            return Some(ShellStatus::Exit);
        }
        if rest.starts_with("超时（") {
            return Some(ShellStatus::Timeout);
        }
        if rest.starts_with("用户停止") {
            return Some(ShellStatus::Stopped);
        }
        None
    })
}

/// 目的：从 shell 回执里取退出码（host 与 guest 的统一标记）；超时 / 停止 / 拿不到时为 None。
pub fn exit_code(output: &str) -> Option<i32> {
    output.lines().rev().find_map(|l| {
        l.trim()
            .strip_prefix("[shell] 退出码 ")
            .and_then(|s| s.trim().parse().ok())
    })
}

/// 目的：shell 回执末尾的「落点 + 本次耗时」行：模型与回放读同一行。
/// 约束：形状是稳定的（`[shell] cwd <路径> · 耗时 <秒> 秒`），前缀与既有字段一字不动；
///   本机档围栏准备（逐条写 ACL）到一秒时只在行尾追加「（其中围栏准备 <秒> 秒）」，不足一秒不追加；
///   追加段不参与解析（见 `cwd_elapsed`）。
pub fn cwd_elapsed_line(cwd: &str, secs: u64, fence_prep_secs: Option<u64>) -> String {
    let mut line = format!("[shell] cwd {} · 耗时 {} 秒", cwd, secs);
    // 围栏准备是「耗时」里的一段：首调用写 ACL 常常就是大头，到一秒才值得单独交代。
    if let Some(prep) = fence_prep_secs.filter(|secs| *secs >= 1) {
        line.push_str(&format!("（其中围栏准备 {} 秒）", prep));
    }
    line
}

/// 目的：从 shell 回执里取（cwd, 耗时秒数）；没有这一行 = None（本次之前的转录）。
/// 约束：只认本模块产出的形状；命令自己的输出伪造同形行时，更靠后的真行会取代它。
pub fn cwd_elapsed(output: &str) -> Option<(String, u64)> {
    output.lines().rev().find_map(|line| {
        let rest = line.trim().strip_prefix("[shell] cwd ")?;
        let (cwd, tail) = rest.rsplit_once(" · 耗时 ")?;
        // 尾段可能带「（其中围栏准备 M 秒）」：只取「秒」前的数字，追加段不参与解析。
        let secs = tail.split(" 秒").next()?.parse().ok()?;
        if cwd.is_empty() {
            return None;
        }
        Some((cwd.to_string(), secs))
    })
}

/// 目的：共享区里的相对路径脚本被直接执行（EACCES）时，给一句"用解释器跑"的提示。
/// 参数：`script` = 本次 shell 脚本；`output` = 执行回执。
/// 返回：要追加的一行；不该提示时给 None。
/// 约束：只在"非零 + Permission denied + 首 token 是相对路径"时提示——共享盘（cifs 合成权限位）不承载执行位。
pub fn shared_exec_hint(script: &str, output: &str) -> Option<String> {
    if !output.contains("Permission denied") {
        return None;
    }
    let first = script.lines().find(|l| !l.trim().is_empty())?.trim();
    let token = first.split_whitespace().next()?;
    let path_like = token.contains('/') || token.contains('\\');
    let absolute = token.starts_with('/') || token.starts_with('\\') || token.contains(":\\");
    if !path_like || absolute {
        return None;
    }
    Some(format!(
        "\n[shell] 共享区不承载执行位：{} 不能直接执行，用解释器跑（例如 bash {} 或 sh {}）",
        token, token, token
    ))
}

/// 目的：这条回执带执行层的 `[shell]` 收尾行（说明守门进程已经走到跑命令这一步）。
/// 约束：参数拒绝、严格模式拒绝与起进程失败都没有这一行；经过守门进程的围栏失败会带 111 的收尾行。
pub fn did_run(output: &str) -> bool {
    output
        .lines()
        .any(|l| l.trim_start().starts_with("[shell] "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> ToolSchema {
        yaml_serde::from_str(
            r#"
desc: shell
params:
  command: { type: string, required: true, non_empty: true }
  scope: { type: string, default: sandbox }
  timeout_secs: { type: integer, min: 1, max: 600, default: 120 }
"#,
        )
        .expect("声明必须能解析")
    }

    #[test]
    fn parses_defaults_and_scope() {
        let call = parse(&schema(), r#"{"command":"echo hi"}"#).expect("合法调用");
        assert_eq!(call.script, "echo hi");
        assert_eq!(call.scope, CallScope::Sandbox);
        assert_eq!(call.timeout, Duration::from_secs(120));
        let call = parse(
            &schema(),
            r#"{"command":"echo hi","scope":"external","timeout_secs":5}"#,
        )
        .expect("合法调用");
        assert_eq!(call.scope, CallScope::External);
        assert_eq!(call.timeout, Duration::from_secs(5));
    }

    #[test]
    fn rejects_unknown_scope_and_overlong_command() {
        assert!(
            parse(&schema(), r#"{"command":"x","scope":"root"}"#)
                .unwrap_err()
                .contains("scope 只认")
        );
        let long = "x".repeat(MAX_COMMAND_CHARS + 1);
        let err = parse(&schema(), &format!(r#"{{"command":"{}"}}"#, long)).unwrap_err();
        assert!(err.contains("命令太长"), "{}", err);
        assert!(parse(&schema(), r#"{"command":""}"#).is_err());
        assert!(parse(&schema(), r#"{"command":"x","timeout_secs":0}"#).is_err());
    }

    #[test]
    fn scope_hint_only_on_a_fenced_sandbox_failure() {
        let failed = ToolOutcome {
            ok: false,
            output: format!(
                "[围栏] 工具进程只可达 x；{}。",
                crate::kernel::api::FENCE_BOUNDARY_MARK
            ),
        };
        let hinted = with_scope_hint(failed.clone(), CallScope::Sandbox, Decision::Ask);
        assert!(hinted.output.contains("[scope]") && hinted.output.contains("scope=external"));
        let untouched = with_scope_hint(failed.clone(), CallScope::External, Decision::Ask);
        assert_eq!(untouched, failed);
        let ordinary = ToolOutcome {
            ok: false,
            output: "编译失败".to_string(),
        };
        assert_eq!(
            with_scope_hint(ordinary.clone(), CallScope::Sandbox, Decision::Ask),
            ordinary
        );
    }

    #[test]
    fn shared_exec_hint_only_for_relative_entries_denied_by_permissions() {
        let denied = "[stderr]\n/bin/sh: 1: ./deploy.sh: Permission denied\n";
        assert!(
            shared_exec_hint("./deploy.sh --yes", denied)
                .expect("相对路径脚本被拒要提示")
                .contains("bash ./deploy.sh")
        );
        assert!(shared_exec_hint("tools/build deploy", denied).is_some());
        assert_eq!(
            shared_exec_hint("/usr/bin/foo", denied),
            None,
            "绝对路径不是共享区入口"
        );
        assert_eq!(
            shared_exec_hint("python tools/x.py", denied),
            None,
            "裸名（解释器）不是这条提示的对象"
        );
        assert_eq!(
            shared_exec_hint("./deploy.sh", "[stderr]\nboom\n"),
            None,
            "不是权限拒绝就不提示"
        );
    }

    #[test]
    fn cwd_and_elapsed_line_round_trips_and_is_stable() {
        let line = cwd_elapsed_line("C:\\work\\a b", 12, None);
        assert_eq!(line, "[shell] cwd C:\\work\\a b · 耗时 12 秒");
        assert_eq!(
            cwd_elapsed(&format!("[shell] 退出码 0\n{}", line)),
            Some(("C:\\work\\a b".to_string(), 12))
        );
        assert_eq!(cwd_elapsed("没有这一行"), None);
        assert_eq!(
            cwd_elapsed("[shell] cwd C:\\a · 耗时 3 秒 · 耗时 4 秒"),
            Some(("C:\\a · 耗时 3 秒".to_string(), 4)),
            "路径里出现同样的分隔符时取最后一个"
        );
    }

    #[test]
    fn fence_prep_is_only_named_in_the_tail_line_from_one_second() {
        // 冻结契约：前缀与既有字段不变；围栏准备不足一秒不追加，到一秒才追加一段。
        assert_eq!(
            cwd_elapsed_line("C:\\work", 3, Some(0)),
            "[shell] cwd C:\\work · 耗时 3 秒",
            "不足一秒不追加"
        );
        let line = cwd_elapsed_line("C:\\work", 239, Some(238));
        assert_eq!(
            line,
            "[shell] cwd C:\\work · 耗时 239 秒（其中围栏准备 238 秒）"
        );
        assert_eq!(
            cwd_elapsed(&format!("[shell] 退出码 0\n{}", line)),
            Some(("C:\\work".to_string(), 239)),
            "追加段不参与解析"
        );
    }

    #[test]
    fn did_run_only_when_the_execution_layer_wrote_a_shell_tail() {
        assert!(did_run("[shell] 退出码 0"));
        assert!(did_run("[shell] 超时（5 秒）"));
        assert!(!did_run("[围栏] 这次调用没有执行"));
        assert!(!did_run("工具进程启动失败：找不到"));
    }

    #[test]
    fn status_reads_the_tail_line_and_ignores_unexecuted_receipts() {
        assert_eq!(status("[shell] 退出码 7"), Some(ShellStatus::Exit));
        assert_eq!(
            status("[shell] 退出码未知（被信号结束）"),
            Some(ShellStatus::Signal)
        );
        assert_eq!(
            status("[shell] 超时（2 秒）：工具进程与它拉起的整棵进程树已被终止"),
            Some(ShellStatus::Timeout)
        );
        assert_eq!(
            status("[shell] 用户停止：工具进程与它拉起的整棵进程树已被终止"),
            Some(ShellStatus::Stopped)
        );
        assert_eq!(status("没有收尾行"), None);
        assert_eq!(status("[围栏] 这次调用没有执行"), None);
        assert_eq!(
            status("[shell] cwd C:\\a · 耗时 3 秒"),
            None,
            "只有 cwd 行不算状态行"
        );
        assert_eq!(
            [
                ShellStatus::Exit.as_str(),
                ShellStatus::Timeout.as_str(),
                ShellStatus::Stopped.as_str(),
                ShellStatus::Signal.as_str(),
            ],
            ["exit", "timeout", "stopped", "signal"]
        );
    }
}
