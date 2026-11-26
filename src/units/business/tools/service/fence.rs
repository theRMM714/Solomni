//! 目的：工具层的围栏裁决——必要落点授不上时怎么处置（问用户、无围栏跑一次、拒绝）。
//! 管：`ToolFenceGate`；裁决卡、回执文案与停会话。
//! 不管：围栏描述符与落点判据（在 `src/kernel/contracts/fence.rs`）、卡片形状（在 `src/units/business/tools/domain/fence.rs`）。
//! 联动：`ToolsService` 持它并作为 `FenceGate` 传给 `ProcessRunner`；kernel 只原样转交裁决结果。

use crate::kernel::api::fence::FenceBlocked;
use crate::kernel::api::types::AskOutcome;
use crate::kernel::ports::{FenceCase, FenceGate, FenceVerdict, ask_user};
use crate::units::business::tools::domain::fence::{OPT_FENCE_UNFENCED, fence_ask};

/// 目的：工具层的围栏裁决——把"必要落点授不上"按用户裁决变成"无围栏跑一次"或"不执行"。
/// 约束：strict 不提供无围栏选项；这一趟没人答就按拒绝收场（fail-closed）；文案逐字是模型看到的回执契约。
pub struct ToolFenceGate;

impl FenceGate for ToolFenceGate {
    #[cfg_attr(
        not(windows),
        allow(
            dead_code,
            reason = "围栏裁决只在 Windows 有生产调用点；非 Windows 保留同一实现以便测试与跨平台编译"
        )
    )]
    fn decide(&self, case: &FenceCase<'_>) -> FenceVerdict {
        let blocked = case.blocked;
        if case.strict {
            return FenceVerdict::Refused {
                output: strict_refusal(blocked),
            };
        }
        let why = blocked.line();
        eprintln!("[围栏] 必要落点授不上（{}）：本次不许按无围栏跑", why);
        let Some(request) = fence_ask(blocked, case.agent, case.unfenced_possible) else {
            match case.ask {
                Some(ask) => ask.halt(&why),
                None => eprintln!("[围栏] 这一趟没有可回答的前端：不停会话，只如实拒绝这次调用"),
            }
            return FenceVerdict::Refused {
                output: refusal(blocked, &AskOutcome::NoOptions),
            };
        };
        let outcome = ask_user(case.ask, &request);
        match outcome.decided() {
            Some(OPT_FENCE_UNFENCED) => FenceVerdict::Unfenced {
                note: unfenced_note(blocked),
            },
            _ => FenceVerdict::Refused {
                output: refusal(blocked, &outcome),
            },
        }
    }
}

/// 目的：这一环授不上的**哪个目录**：没有具体目录的一环（容器身份 / 授权台账）如实说没有。
fn where_text(blocked: &FenceBlocked) -> String {
    if blocked.path.as_os_str().is_empty() {
        "没有具体目录".to_string()
    } else {
        blocked.path.to_string_lossy().into_owned()
    }
}

fn refused_text(blocked: &FenceBlocked) -> String {
    format!(
        "[围栏] 这次调用**没有执行**：{}（{}）授不上——{}。怎么补：{}",
        blocked.what,
        where_text(blocked),
        blocked.why,
        blocked.fix
    )
}

/// 目的：不执行时的回执——哪一环、哪个目录、缺什么前提、怎么补；用户拒绝与"没人答"分别追加一句。
fn refusal(blocked: &FenceBlocked, outcome: &AskOutcome) -> String {
    let path = where_text(blocked);
    let part = blocked.what;
    let mut output = refused_text(blocked);
    let extra = match outcome {
        AskOutcome::Chosen(_) => Some(
            "用户没有放行这次工具调用：它**没有执行**。请换一种做法，或请用户调整会话权限（ask / full）。"
                .to_string(),
        ),
        AskOutcome::NoAnswer => Some(format!(
            "[围栏] 这一问没人能答（没有可回答的前端）：{}（{}）授不上——这次调用**没有执行**。",
            part, path
        )),
        AskOutcome::Stopped | AskOutcome::NoOptions => None,
    };
    if let Some(line) = extra {
        output.push('\n');
        output.push_str(&line);
    }
    output
}

/// 目的：shell 严格模式下必要落点授不上：不发起裁决、不提供"无围栏跑一次"，直接失败。
#[cfg_attr(
    not(windows),
    allow(
        dead_code,
        reason = "围栏裁决只在 Windows 有生产调用点；非 Windows 保留同一实现以便测试与跨平台编译"
    )
)]
fn strict_refusal(blocked: &FenceBlocked) -> String {
    let mut output = refused_text(blocked);
    output.push('\n');
    output.push_str("严格模式：shell 要求围栏真强制，这次调用没有执行。");
    output
}

/// 目的：无围栏跑一次时回执里的如实标注（模型下一轮看得到"这次没有容器那层强制"）。
#[cfg_attr(
    not(windows),
    allow(
        dead_code,
        reason = "围栏裁决只在 Windows 有生产调用点；非 Windows 保留同一实现以便测试与跨平台编译"
    )
)]
fn unfenced_note(blocked: &FenceBlocked) -> String {
    format!(
        "[围栏] 这次调用按你的裁决**无围栏**执行（没有容器那层强制）：{}（{}）授不上——{}",
        blocked.what,
        where_text(blocked),
        blocked.why
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::api::types::Ask;
    use crate::kernel::ports::AskUser;

    /// 目的：没有可回答的前端（这一趟没人能答）：如实拒绝这次调用。
    struct Silent;

    impl AskUser for Silent {
        fn ask(&self, _ask: &Ask) -> AskOutcome {
            AskOutcome::NoAnswer
        }

        fn halt(&self, _why: &str) {}
    }

    /// 目的：**没人答不等于放行**：这一趟没放行就按拒绝收场（fail-closed），绝不悄悄按无围栏跑。
    #[test]
    fn a_question_nobody_answers_never_means_unfenced() {
        let blocked = FenceBlocked {
            what: "模块目录",
            path: std::path::PathBuf::from("C:/mods/a"),
            why: "写 DACL 失败（错误码 5）".to_string(),
            fix: "以管理员身份跑一次 setup",
        };
        let case = FenceCase {
            blocked: &blocked,
            strict: false,
            unfenced_possible: true,
            agent: "甲",
            ask: Some(&Silent),
        };
        match ToolFenceGate.decide(&case) {
            FenceVerdict::Refused { output } => {
                assert!(output.contains("没人能答"), "要说清为什么没跑：{}", output);
                assert!(
                    output.contains("没有执行"),
                    "要明确这次调用没执行：{}",
                    output
                );
            }
            FenceVerdict::Unfenced { .. } => panic!("没人答不能当成放行"),
        }
    }
}
