//! 目的：执行类系统工具 `shell` 的纯规则：参数契约、命令长度、scope / timeout 解析与失败回执的 scope 提示。
//! 管：一次 shell 调用的入参到 `ShellCall` 的解析；命令过长 / scope 非法 / timeout 越界的事实与文案。
//! 不管：进程拉起、围栏安装、guest 执行与输出截断（在 kernel 的 ProcessRunner / vm 的 guest）；围栏派生在 domain/fence.rs。
//! 联动：由 collab 的成员循环在派发 `shell` 前调用；声明在 systools/tools.yaml，经 tools::api 导出。

use crate::capabilities::permission::api::{CallScope, Decision};
use crate::capabilities::tools::domain::schema::ToolSchema;
use crate::capabilities::tools::domain::systool::{arg_fault_text, SHELL};
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

/// 目的：从 shell 回执里取退出码（host 与 guest 的统一标记）；超时 / 停止 / 拿不到时为 None。
pub fn exit_code(output: &str) -> Option<i32> {
    output.lines().rev().find_map(|l| {
        l.trim()
            .strip_prefix("[shell] 退出码 ")
            .and_then(|s| s.trim().parse().ok())
    })
}

/// 目的：sandbox shell 的按次临时容器 profile 名与租约名（Windows W1 用；其它平台只是日志标识）。
/// 约束：同一进程内单调递增 + pid，避免并发调用撞名；agent / lease 都带同一个后缀。
pub fn ephemeral_names(agent: &str, session: &str) -> (String, String) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let tag = format!("{}-{}", std::process::id(), n);
    (
        format!("{}#shell-{}", agent, tag),
        format!("{}#shell-{}", session, tag),
    )
}

/// 目的：guest Linux 里跑 shell 的脚本包装：子 shell 接住用户脚本，打印退出码并原样退出。
/// 约束：用户脚本里的 exit 只退出子 shell，包装仍能打印退出码；命令的 stdin 不提供交互。
pub fn guest_script(script: &str) -> String {
    format!(
        "( {}\n); __solomni_shell_code=$?\nprintf '[shell] 退出码 %s\\n' \"$__solomni_shell_code\"\nexit $__solomni_shell_code\n",
        script
    )
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
        assert!(parse(&schema(), r#"{"command":"x","scope":"root"}"#)
            .unwrap_err()
            .contains("scope 只认"));
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
}
