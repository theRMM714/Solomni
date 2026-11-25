//! 目的：回执的**结构化说明**——稳定码 + 轻重分级 + 类型化参数；码是契约，文案在边缘按语言渲染。
//! 管：轻重三档（`Severity`）、说明形状（`Note`）、跨单元共用的码表（`codes`）与按说明汇总分级。
//! 不管：具体文案（在 `locales/<lang>.ftl`，由 `kernel::detail::i18n` 渲染）；谁展示（模型通道 / 界面 / 日志各自决定）。
//! 联动：`ToolOutcome` 与 `RunOutcome` 都带它；工具执行层、会话引擎与 Web/CLI 按 `code` 取目录文案。

use std::collections::BTreeMap;

/// 目的：一条回执的轻重——界面按它分图标与颜色，不靠读文案猜。
/// 约束：序即优先级（`Info < Warning < Error`）；一次回执的分级取全部说明里最重的那条。
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// 目的：完成且可用——缺省值（没有说明时按它算）。
    #[default]
    Info,
    /// 目的：完成但需留意（非零退出、输出截断、围栏收窄、用户主动停止）。
    Warning,
    /// 目的：没完成（未执行、启动失败、超时、被信号终止）。
    Error,
}

/// 目的：一条回执说明——`code` 是稳定契约，`severity` 是轻重，`args` 是它的类型化参数。
/// 约束：`code` 只写 ASCII 点分标识（认 `codes` 里的常量），不写任何自然语言；缺目录项时渲染出码本身。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Note {
    /// 目的：稳定标识（`shell.exit` 这类）；改文案不改它。
    pub code: String,
    /// 目的：这条说明的轻重。
    pub severity: Severity,
    /// 目的：参数——键是模板里的占位名，值是数据（数字、路径、原样错误文本），不是文案。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub args: BTreeMap<String, String>,
}

impl Note {
    /// 目的：按码与轻重造一条空参说明。
    pub fn new(code: &str, severity: Severity) -> Note {
        Note {
            code: code.to_string(),
            severity,
            args: BTreeMap::new(),
        }
    }

    /// 目的：一条「完成且可用」的说明。
    pub fn info(code: &str) -> Note {
        Note::new(code, Severity::Info)
    }

    /// 目的：一条「完成但需留意」的说明。
    pub fn warning(code: &str) -> Note {
        Note::new(code, Severity::Warning)
    }

    /// 目的：一条「没完成」的说明。
    pub fn error(code: &str) -> Note {
        Note::new(code, Severity::Error)
    }

    /// 目的：补一个模板参数（同名覆盖）。
    pub fn arg(mut self, key: &str, value: impl ToString) -> Note {
        self.args.insert(key.to_string(), value.to_string());
        self
    }
}

/// 目的：按一组说明汇总分级——没有说明时是 `Info`，否则取最重的一条。
pub fn severity_of(notes: &[Note]) -> Severity {
    notes
        .iter()
        .map(|n| n.severity)
        .max()
        .unwrap_or(Severity::Info)
}

/// 目的：跨单元共用的回执码表——稳定 ASCII 点分标识，任何一档都不许在这里写自然语言。
/// 约束：`locales/<lang>.ftl` 每个码一条；Web 按 `ALL` 导出目录（缺一条由测试点名）。
pub mod codes {
    /// 目的：shell 正常退出（`code` = 退出码；0 是 info，非零是 warning）。
    pub const SHELL_EXIT: &str = "shell.exit";
    /// 目的：shell 被信号结束，拿不到退出码。
    pub const SHELL_SIGNAL: &str = "shell.signal";
    /// 目的：shell 到点被杀（`secs` = 本次上限秒数）。
    pub const SHELL_TIMEOUT: &str = "shell.timeout";
    /// 目的：shell 被用户停止。
    pub const SHELL_CANCELLED: &str = "shell.cancelled";
    /// 目的：本次执行的落点与耗时（`cwd` / `secs`）。
    pub const SHELL_PLACE: &str = "shell.place";
    /// 目的：本机档围栏准备的耗时（`prep` = 秒）。
    pub const SHELL_PLACE_PREP: &str = "shell.place.prep";
    /// 目的：共享区不承载执行位（`script` = 被拒的命令首词）。
    pub const SHELL_SHARED_EXEC: &str = "shell.shared-exec";
    /// 目的：模块工具到点被杀（非 shell 通道）。
    pub const RUN_TIMEOUT: &str = "run.timeout";
    /// 目的：工具进程没起来（`error` = 机制原样错误）。
    pub const RUN_SPAWN_FAILED: &str = "run.spawn-failed";
    /// 目的：围栏机制没装上，命令没有执行。
    pub const FENCE_FAILED: &str = "fence.failed";
    /// 目的：回执被 head+tail 预算截断（`total` / `head` / `tail` 字符数）。
    pub const RESULT_TRUNCATED: &str = "result.truncated";
    /// 目的：截断后怎么接着取——执行类（stdout 不是文件）。
    pub const RESULT_TRUNCATED_COMMAND: &str = "result.truncated.command";
    /// 目的：截断后怎么接着取——文件类（能按 offset/limit 续读）。
    pub const RESULT_TRUNCATED_READ: &str = "result.truncated.read";
    /// 目的：本批预算按剩余上下文收紧过，截断后要继续就得先压缩会话（`limit` = 本次字符上限）。
    pub const RESULT_TRUNCATED_CONTEXT: &str = "result.truncated.context";
    /// 目的：三档分级的中文标签（CLI 归成「成功 / 警告 / 失败」时用；界面用图标与颜色）。
    pub const LABEL_OK: &str = "cli.ok";
    /// 目的：见 `LABEL_OK`。
    pub const LABEL_WARNING: &str = "cli.warning";
    /// 目的：见 `LABEL_OK`。
    pub const LABEL_ERROR: &str = "cli.error";

    /// 目的：全部码（目录完整性检查与 Web 导出按它遍历）。
    pub const ALL: &[&str] = &[
        SHELL_EXIT,
        SHELL_SIGNAL,
        SHELL_TIMEOUT,
        SHELL_CANCELLED,
        SHELL_PLACE,
        SHELL_PLACE_PREP,
        SHELL_SHARED_EXEC,
        RUN_TIMEOUT,
        RUN_SPAWN_FAILED,
        FENCE_FAILED,
        RESULT_TRUNCATED,
        RESULT_TRUNCATED_COMMAND,
        RESULT_TRUNCATED_READ,
        RESULT_TRUNCATED_CONTEXT,
        LABEL_OK,
        LABEL_WARNING,
        LABEL_ERROR,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 目的：分级的序就是优先级——一次回执取最重的那条。
    #[test]
    fn severity_order_is_the_priority() {
        assert!(Severity::Info < Severity::Warning && Severity::Warning < Severity::Error);
        let notes = vec![
            Note::info(codes::SHELL_PLACE),
            Note::warning(codes::SHELL_EXIT),
        ];
        assert_eq!(severity_of(&notes), Severity::Warning);
        assert_eq!(severity_of(&[]), Severity::Info);
    }

    /// 目的：码表只写 ASCII 点分标识；它是对外契约，不是文案。
    #[test]
    fn codes_are_stable_ascii_identifiers() {
        for code in codes::ALL {
            assert!(
                code.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-'),
                "码必须是 ASCII 点分标识：{}",
                code
            );
        }
        let mut sorted = codes::ALL.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), codes::ALL.len(), "码表不能重复");
    }

    /// 目的：参数是数据——数字与路径原样进去，渲染时才代入模板。
    #[test]
    fn args_carry_data_not_text() {
        let note = Note::warning(codes::SHELL_EXIT).arg("code", 3);
        assert_eq!(note.args.get("code").map(String::as_str), Some("3"));
        assert_eq!(
            serde_json::to_value(&note).expect("可序列化"),
            serde_json::json!({"code": "shell.exit", "severity": "warning", "args": {"code": "3"}})
        );
    }
}
