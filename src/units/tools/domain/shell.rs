//! 目的：执行类系统工具 `shell` 的纯规则：参数契约、命令长度、timeout 解析与回执说明的派生。
//! 管：一次 shell 调用的入参到 `ShellCall` 的解析；命令过长 / timeout 越界的事实与文案；从回执说明（稳定码）与测量值派生结构化事实 `ShellFacts`。
//! 不管：进程拉起、围栏安装、guest 执行与输出截断（在 kernel 的 ProcessRunner / vm 的 guest）；围栏派生在 domain/fence.rs。
//! 联动：由 collab 的成员循环在派发 `shell` 前调用；声明在 systools/tools.yaml，经 tools::api 导出。

use crate::units::tools::domain::schema::ToolSchema;
use crate::units::tools::domain::systool::{SHELL, arg_fault_text};
use crate::kernel::api::note::Note;
use crate::kernel::api::note::codes;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// 目的：单条 shell 命令的字符上限；超过就要求模型先把脚本写进沙箱再执行（命令不拼进命令行）。
pub const MAX_COMMAND_CHARS: usize = 8_000;

/// 目的：一次已校验的 shell 调用——脚本、本次超时、要放的宿主回环端口。授予是固定的（私有沙箱 rw、模块目录与主机只读根 ro、网络按档位），
///   调用参数里唯一的旋钮是 `local_ports`：它不扩大文件系统授予，只决定容器内哪几个宿主回环端口可连。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellCall {
    /// 目的：要执行的命令原文（模型写的本环境 shell 语法）。
    pub script: String,
    /// 目的：本次执行的超时；到点连根杀整棵进程树。
    pub timeout: Duration,
    /// 目的：要放行的宿主回环端口（空 = 不碰回环，走会话档位）。
    pub local_ports: Vec<u16>,
}

/// 目的：单次调用能要几个回环端口——一个中继进程要 bind 这么多个监听，多到几十个就没有"测试一个服务"的形状了。
pub const MAX_LOCAL_PORTS: usize = 8;

/// 目的：按工具声明校验并解析一次 shell 调用；参数不符 / 命令过长都如实报事实。
/// 参数：`schema` = systools/tools.yaml 里的 shell 声明；`args_json` = 模型给的参数对象。
/// 返回：已补缺省（timeout_secs=120）的调用。
/// 错误：参数不是 JSON、缺必填 / 类型不符（含不认识的参数）、命令超过 `MAX_COMMAND_CHARS`。
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
    let secs = args
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(120);
    let local_ports = parse_local_ports(&args)?;
    Ok(ShellCall {
        script,
        timeout: Duration::from_secs(secs),
        local_ports,
    })
}

/// 目的：把 `local_ports` 摊成去重后的端口清单；形状不符 / 越界 / 太多都如实报事实。
/// 约束：数组元素形状由 JSON Schema 的 `items: {}` 放过，逐元素校验在这里做（schema.check 只管"是不是数组"）。
fn parse_local_ports(args: &serde_json::Value) -> Result<Vec<u16>, String> {
    let Some(arr) = args.get("local_ports").and_then(|v| v.as_array()) else {
        return Ok(Vec::new());
    };
    if arr.len() > MAX_LOCAL_PORTS {
        return Err(format!(
            "local_ports 最多 {} 个端口（这次给了 {} 个）：一个中继进程绑这么多监听没有意义，请只列测试真正要连的端口。",
            MAX_LOCAL_PORTS,
            arr.len()
        ));
    }
    let mut out: Vec<u16> = Vec::with_capacity(arr.len());
    for item in arr {
        let Some(n) = item.as_u64() else {
            return Err(format!(
                "local_ports 的成员必须是端口号（整数 1-65535），这次给了 {}：这条调用没有执行。",
                item
            ));
        };
        if n == 0 || n > u64::from(u16::MAX) {
            return Err(format!(
                "local_ports 里的 {} 不是合法端口（1-65535）：这条调用没有执行。",
                n
            ));
        }
        let port = u16::try_from(n).unwrap_or_default();
        if !out.contains(&port) {
            out.push(port);
        }
    }
    Ok(out)
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

/// 目的：从回执说明里认状态——壳执行真的收场了才会有 `shell.exit` / `shell.signal` / `shell.timeout` / `shell.cancelled`。
pub fn status(notes: &[Note]) -> Option<ShellStatus> {
    notes.iter().find_map(|note| match note.code.as_str() {
        codes::SHELL_EXIT => Some(ShellStatus::Exit),
        codes::SHELL_SIGNAL => Some(ShellStatus::Signal),
        codes::SHELL_TIMEOUT => Some(ShellStatus::Timeout),
        codes::SHELL_CANCELLED => Some(ShellStatus::Stopped),
        _ => None,
    })
}

/// 目的：从回执说明里取退出码；超时 / 停止 / 拿不到时为 None。
pub fn exit_code(notes: &[Note]) -> Option<i32> {
    notes.iter().find_map(|note| {
        if note.code != codes::SHELL_EXIT {
            return None;
        }
        note.args.get("code").and_then(|code| code.parse().ok())
    })
}

/// 目的：这条回执带执行层的收尾说明（说明守门进程已经走到跑命令这一步）。
/// 约束：参数拒绝、严格模式拒绝与起进程失败都没有收尾说明；围栏没装上也只剩 `fence.failed`。
pub fn did_run(notes: &[Note]) -> bool {
    status(notes).is_some()
}

/// 目的：落点与耗时的说明（模型与回放读同一份；回执渲染时排在正文与收尾说明之后）。
/// 约束：围栏准备（逐条写 ACL）到一秒才单独交代，不足一秒不追加——它是「耗时」里的一段。
pub fn place_notes(cwd: &str, secs: u64, fence_prep_secs: Option<u64>) -> Vec<Note> {
    let mut notes = vec![
        Note::info(codes::SHELL_PLACE)
            .arg("cwd", cwd)
            .arg("secs", secs),
    ];
    if let Some(prep) = fence_prep_secs.filter(|secs| *secs >= 1) {
        notes.push(Note::info(codes::SHELL_PLACE_PREP).arg("prep", prep));
    }
    notes
}

/// 目的：共享区里的相对路径脚本被直接执行（EACCES）时，给一条"用解释器跑"的说明。
/// 参数：`script` = 本次 shell 脚本；`output` = 进程原始输出。
/// 返回：要追加的说明；不该提示时给 None。
/// 约束：只在"输出里有 Permission denied + 首 token 是相对路径"时提示——共享盘（cifs 合成权限位）不承载执行位；
///   这只是建议，不声称失败原因，所以判据读进程原始输出不会造成误报定性。
pub fn shared_exec_note(script: &str, output: &str) -> Option<Note> {
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
    Some(Note::info(codes::SHELL_SHARED_EXEC).arg("script", token))
}
/// 目的：一次 shell 调用的结构化事实——落点 / 耗时 / 围栏准备 / 退出码 / 状态 / 是否截断；
///   实况卡片、会话回放与测试读同一份，不各自解析回执文本。
/// 约束：六个字段都能缺省（形状稳定）；时间单位统一毫秒；status 词表同 ShellStatus。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellFacts {
    /// 目的：本次执行的落点（呈现写法，不是机器路径）。
    #[serde(default)]
    pub cwd: String,
    /// 目的：本次执行的总耗时（毫秒），含围栏准备。
    #[serde(default)]
    pub elapsed_ms: u64,
    /// 目的：本机档围栏准备耗时（毫秒）：逐条写 ACL 常常是首调用的大头；虚拟机档为 None。
    #[serde(default)]
    pub fence_prep_ms: Option<u64>,
    /// 目的：退出码；超时 / 停止 / 拿不到时为 None。
    #[serde(default)]
    pub exit_code: Option<i32>,
    /// 目的：状态行分类（exit / timeout / stopped / signal）；没有状态行时为 None。
    #[serde(default)]
    pub status: Option<String>,
    /// 目的：这次回执是否被 head+tail 截断过（模型只看到开头与结尾）。
    #[serde(default)]
    pub truncated: bool,
}

impl ShellFacts {
    /// 目的：只认秒的呈现与状态栏用的整秒耗时。
    pub fn elapsed_secs(&self) -> u64 {
        self.elapsed_ms / 1000
    }

    /// 目的：围栏准备到一秒才单独交代（与回执行「不足一秒不追加」同一口径）。
    pub fn fence_prep_secs(&self) -> Option<u64> {
        self.fence_prep_ms
            .filter(|ms| *ms >= 1000)
            .map(|ms| ms / 1000)
    }
}

/// 目的：把执行层已经算好的计时与回执说明整理成一份结构化事实（不在文本里再解析一遍）。
/// 参数：cwd = 呈现落点；elapsed = 本次墙钟耗时；fence_prep = 同线程交棒来的围栏准备耗时（虚拟机档为 None）；
///   notes = 回执说明（退出码 / 状态 / 截断都从稳定码取）。
pub fn facts(
    cwd: &str,
    elapsed: Duration,
    fence_prep: Option<Duration>,
    notes: &[Note],
) -> ShellFacts {
    ShellFacts {
        cwd: cwd.to_string(),
        elapsed_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
        fence_prep_ms: fence_prep.map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
        exit_code: exit_code(notes),
        status: status(notes).map(|s| s.as_str().to_string()),
        truncated: truncated(notes),
    }
}

/// 目的：这条回执是否被截断——只看预算写下的稳定码，不认文案。
pub fn truncated(notes: &[Note]) -> bool {
    notes
        .iter()
        .any(|note| note.code == codes::RESULT_TRUNCATED)
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
  timeout_secs: { type: integer, min: 1, max: 600, default: 120 }
"#,
        )
        .expect("声明必须能解析")
    }

    #[test]
    fn parses_defaults() {
        let call = parse(&schema(), r#"{"command":"echo hi"}"#).expect("合法调用");
        assert_eq!(call.script, "echo hi");
        assert_eq!(call.timeout, Duration::from_secs(120));
        let call = parse(&schema(), r#"{"command":"echo hi","timeout_secs":5}"#).expect("合法调用");
        assert_eq!(call.timeout, Duration::from_secs(5));
    }

    #[test]
    fn rejects_range_knobs_and_overlong_command() {
        // 授予固定：没有 scope 这类范围旋钮，不认识的参数按参数错误拒绝。
        assert!(parse(&schema(), r#"{"command":"x","scope":"external"}"#).is_err());
        let long = "x".repeat(MAX_COMMAND_CHARS + 1);
        let err = parse(&schema(), &format!(r#"{{"command":"{}"}}"#, long)).unwrap_err();
        assert!(err.contains("命令太长"), "{}", err);
        assert!(parse(&schema(), r#"{"command":""}"#).is_err());
        assert!(parse(&schema(), r#"{"command":"x","timeout_secs":0}"#).is_err());
    }

    #[test]
    fn shared_exec_note_only_for_relative_entries_denied_by_permissions() {
        let denied = "[stderr]\n/bin/sh: 1: ./deploy.sh: Permission denied\n";
        let note = shared_exec_note("./deploy.sh --yes", denied).expect("相对路径脚本被拒要提示");
        assert_eq!(note.code, codes::SHELL_SHARED_EXEC);
        assert_eq!(
            note.args.get("script").map(String::as_str),
            Some("./deploy.sh")
        );
        assert_eq!(
            note.severity,
            crate::kernel::api::note::Severity::Info,
            "这只是建议，不是失败定性"
        );
        assert!(shared_exec_note("tools/build deploy", denied).is_some());
        assert_eq!(
            shared_exec_note("/usr/bin/foo", denied),
            None,
            "绝对路径不是共享区入口"
        );
        assert_eq!(
            shared_exec_note("python tools/x.py", denied),
            None,
            "裸名（解释器）不是这条提示的对象"
        );
        assert_eq!(
            shared_exec_note("./deploy.sh", "[stderr]\nboom\n"),
            None,
            "不是权限拒绝就不提示"
        );
    }

    #[test]
    fn place_notes_carry_the_location_and_only_name_fence_prep_from_one_second() {
        let notes = place_notes("C:\\work\\a b", 12, None);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].code, codes::SHELL_PLACE);
        assert_eq!(
            notes[0].args.get("cwd").map(String::as_str),
            Some("C:\\work\\a b")
        );
        assert_eq!(notes[0].args.get("secs").map(String::as_str), Some("12"));
        // 围栏准备不足一秒不交代（与回执行「不足一秒不追加」同一口径），到一秒才追加一条。
        assert_eq!(place_notes("C:\\work", 3, Some(0)).len(), 1);
        let notes = place_notes("C:\\work", 239, Some(238));
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[1].code, codes::SHELL_PLACE_PREP);
        assert_eq!(notes[1].args.get("prep").map(String::as_str), Some("238"));
    }

    #[test]
    fn did_run_only_when_the_execution_layer_wrote_a_closing_note() {
        assert!(did_run(&[Note::info(codes::SHELL_EXIT).arg("code", 0)]));
        assert!(did_run(&[Note::error(codes::SHELL_TIMEOUT).arg("secs", 5)]));
        assert!(!did_run(&[Note::error(codes::FENCE_FAILED)]));
        assert!(!did_run(&[Note::error(codes::RUN_SPAWN_FAILED)]));
        assert!(!did_run(&[Note::info(codes::SHELL_PLACE).arg("cwd", "/w")]));
    }

    #[test]
    fn facts_carries_the_measured_timing_and_the_closing_notes() {
        let notes = vec![
            Note::info(codes::SHELL_EXIT).arg("code", 0),
            Note::info(codes::SHELL_PLACE)
                .arg("cwd", "C:\\work")
                .arg("secs", 331),
            Note::info(codes::SHELL_PLACE_PREP).arg("prep", 327),
        ];
        let f = facts(
            "C:\\work",
            Duration::from_millis(331_400),
            Some(Duration::from_millis(327_000)),
            &notes,
        );
        assert_eq!(f.cwd, "C:\\work");
        assert_eq!(f.elapsed_ms, 331_400);
        assert_eq!(f.fence_prep_ms, Some(327_000));
        assert_eq!(f.elapsed_secs(), 331);
        assert_eq!(f.fence_prep_secs(), Some(327));
        assert_eq!(f.exit_code, Some(0));
        assert_eq!(f.status.as_deref(), Some("exit"));
        assert!(!f.truncated);
        // 围栏准备不足一秒不给回放看（与回执说明同一口径）。
        let quick = facts(
            "C:\\work",
            Duration::from_millis(400),
            Some(Duration::from_millis(400)),
            &[],
        );
        assert_eq!(quick.elapsed_secs(), 0);
        assert_eq!(quick.fence_prep_secs(), None);
        assert_eq!(quick.exit_code, None, "没有收尾说明就没有退出码");
        assert_eq!(quick.status, None);
    }

    #[test]
    fn truncated_reads_the_budget_note() {
        assert!(truncated(&[Note::warning(codes::RESULT_TRUNCATED)]));
        assert!(!truncated(&[Note::info(codes::SHELL_EXIT).arg("code", 0)]));
        assert!(!truncated(&[Note::info(codes::RESULT_TRUNCATED_READ)]));
    }

    #[test]
    fn facts_round_trips_through_json_and_tolerates_missing_fields() {
        let f = facts(
            "/w/a",
            Duration::from_millis(7_900),
            None,
            &[Note::info(codes::SHELL_EXIT).arg("code", 0)],
        );
        let json = serde_json::to_value(&f).expect("可序列化");
        assert_eq!(
            serde_json::from_value::<ShellFacts>(json).expect("可反序列化"),
            f
        );
        // 六个字段都能缺省：形状稳定，缺字段的行不失败。
        let empty: ShellFacts = serde_json::from_value(serde_json::json!({})).expect("缺省要能读");
        assert_eq!(empty, ShellFacts::default());
        assert_eq!(empty.elapsed_secs(), 0);
        assert_eq!(empty.fence_prep_secs(), None);
    }

    #[test]
    fn status_and_exit_code_read_the_closing_notes() {
        assert_eq!(
            status(&[Note::info(codes::SHELL_EXIT).arg("code", 7)]),
            Some(ShellStatus::Exit)
        );
        assert_eq!(
            status(&[Note::error(codes::SHELL_SIGNAL)]),
            Some(ShellStatus::Signal)
        );
        assert_eq!(
            status(&[Note::error(codes::SHELL_TIMEOUT).arg("secs", 2)]),
            Some(ShellStatus::Timeout)
        );
        assert_eq!(
            status(&[Note::warning(codes::SHELL_CANCELLED)]),
            Some(ShellStatus::Stopped)
        );
        assert_eq!(status(&[]), None);
        assert_eq!(status(&[Note::error(codes::FENCE_FAILED)]), None);
        assert_eq!(
            exit_code(&[Note::info(codes::SHELL_PLACE).arg("cwd", "C:\\a")]),
            None,
            "只有落点说明不算状态"
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
