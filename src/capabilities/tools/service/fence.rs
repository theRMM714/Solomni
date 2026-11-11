//! 目的：工具层的围栏裁决——必要落点授不上时怎么处置（问用户、无围栏跑一次、拒绝）。
//! 管：`ToolFenceGate`；裁决卡、回执文案与停会话。
//! 不管：围栏描述符与落点判据（在 `src/kernel/contracts/fence.rs`）、卡片形状（在 `src/capabilities/tools/domain/fence.rs`）。
//! 联动：`ToolsService` 持它并作为 `FenceGate` 传给 `ProcessRunner`；kernel 只原样转交裁决结果。

use crate::capabilities::tools::domain::fence::{fence_ask, OPT_FENCE_UNFENCED};
use crate::kernel::api::{AskOutcome, FenceBlocked};
use crate::kernel::ports::{ask_user, FenceCase, FenceGate, FenceVerdict};

/// 目的：工具层的围栏裁决——把"必要落点授不上"按用户裁决变成"无围栏跑一次"或"不执行"。
/// 约束：strict 不提供无围栏选项；构不出选项时停会话并拒绝；文案逐字是模型看到的回执契约。
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
        blocked.part.label(),
        where_text(blocked),
        blocked.why,
        blocked.part.fix()
    )
}

/// 目的：不执行时的回执——哪一环、哪个目录、缺什么前提、怎么补；用户拒绝与"没人答"分别追加一句。
fn refusal(blocked: &FenceBlocked, outcome: &AskOutcome) -> String {
    let path = where_text(blocked);
    let part = blocked.part.label();
    let mut output = refused_text(blocked);
    let extra = match outcome {
        AskOutcome::Chosen(_) => Some(
            "用户没有放行这次工具调用：它**没有执行**。请换一种做法，或请用户调整会话权限（ask / full）。"
                .to_string(),
        ),
        AskOutcome::Defaulted(id) => Some(format!(
            "[围栏] 这一问没人能答（没有可回答的前端）：{}（{}）授不上——按这一问声明的默认项「{}」收场。",
            part, path, id
        )),
        AskOutcome::NoAnswer => Some(format!(
            "[围栏] 这一问没人能答（没有可回答的前端），也没有声明默认项：{}（{}）授不上——这次调用**没有执行**。",
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
        blocked.part.label(),
        where_text(blocked),
        blocked.why
    )
}
