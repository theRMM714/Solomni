//! 目的：启动门：围栏能力与授权、缺项报告、Windows 首次引导、围栏核对与 Web 概览共用的一份事实。
//! 管：围栏现状的判定、缺项原因、人机问答与授权写回；`FenceReport` 是唯一一份事实。
//! 不管：各探针的实现（`entry/probes/`）、能力内部机制（`crate::kernel::detail::confine`）。
//! 联动：`src/main.rs` 在装配后调用；`entry/web.rs` 读 `FenceReport`；`docs/` 的围栏说明。

use std::io::{BufRead, Write};

/// 目的：环境变量优先于设置里的写 ACL 授权；只认 1/0，其它值不覆盖。
pub(crate) fn authorize(env: Option<&str>, setting: bool) -> bool {
    match env {
        Some("1") => true,
        Some("0") => false,
        _ => setting,
    }
}

/// 目的：一项强制要求没满足的**原因**——未授权能靠授权修好，能力缺失不能。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MissingCause {
    Unsupported,
    Unauthorized,
}

impl MissingCause {
    pub(crate) fn label(self) -> &'static str {
        match self {
            MissingCause::Unsupported => "本机能力不可用",
            MissingCause::Unauthorized => "Windows 容器围栏未授权（本机能力可用）",
        }
    }
}

/// 目的：一项“要求但没满足”的围栏与原因；启动门据此决定能不能修。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MissingPart {
    pub(crate) name: &'static str,
    pub(crate) cause: MissingCause,
}

/// 目的：把“要求但没满足”的项按原因拆开。授权只影响 Windows 的路径级围栏； Linux / macOS 的围栏在守门进程里自足，未授权不等于无围栏。
pub(crate) fn missing_parts(
    requirements: &crate::kernel::api::SandboxRequirements,
    cap: &crate::kernel::detail::confine::Capability,
    authorized: bool,
) -> Vec<MissingPart> {
    let paths_ok = !cfg!(windows) || authorized;
    let mut out = Vec::new();
    if requirements.filesystem {
        if !cap.fs {
            out.push(MissingPart {
                name: "文件系统围栏",
                cause: MissingCause::Unsupported,
            });
        } else if !paths_ok {
            out.push(MissingPart {
                name: "文件系统围栏",
                cause: MissingCause::Unauthorized,
            });
        }
    }
    if requirements.network {
        if !cap.net {
            out.push(MissingPart {
                name: "网络围栏",
                cause: MissingCause::Unsupported,
            });
        } else if !paths_ok {
            out.push(MissingPart {
                name: "网络围栏",
                cause: MissingCause::Unauthorized,
            });
        }
    }
    if requirements.process_tree && !cap.tree {
        out.push(MissingPart {
            name: "进程树围栏",
            cause: MissingCause::Unsupported,
        });
    }
    // 环境白名单由进程机制自足（组合根传 env_ok=true），没有“能力缺失”这一路。
    out
}

/// 目的：把指定原因的缺失项拼成人类可读清单。
pub(crate) fn missing_names(parts: &[MissingPart], cause: MissingCause) -> String {
    parts
        .iter()
        .filter(|p| p.cause == cause)
        .map(|p| p.name)
        .collect::<Vec<_>>()
        .join("、")
}

/// 目的：启动报告先把「本机能力」与「本次实际」分开说清，再让启动门提问。
pub(crate) fn print_fence_report(report: &FenceReport, read_only_roots: usize) {
    let usable = |ok: bool| if ok { "可用" } else { "不可用" };
    println!(
        "[围栏] 本机能力：文件系统={} 断网={} 进程树={}（{}）",
        usable(report.cap.fs),
        usable(report.cap.net),
        usable(report.cap.tree),
        report.cap.note
    );
    println!(
        "[围栏] 本次实际：文件系统={} 断网={} 进程树={}；容器授权={}；只读根={} 个（{}）",
        usable(report.fs),
        usable(report.net),
        usable(report.cap.tree),
        fence_auth_label(report.write_allowed),
        read_only_roots,
        fence_auth_note(report.write_allowed)
    );
}

/// 目的：可注入读写的 y/n 提问——启动门与测试用同一套默认值（空行 = 默认项）。
pub(crate) fn ask_yes_no<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    question: &str,
    default_yes: bool,
) -> bool {
    let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
    let _ = write!(writer, "{} {} ", question, hint);
    let _ = writer.flush();
    let mut answer = String::new();
    if reader.read_line(&mut answer).is_err() {
        return default_yes;
    }
    match answer.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => true,
        "n" | "no" => false,
        _ => default_yes,
    }
}

/// 目的：把 Windows 的写 ACL 授权落进设置；调用方负责在成功后整体重启。
pub(crate) fn write_acl_setting(
    registry: &mut dyn crate::capabilities::registry::api::Registry,
    value: bool,
) -> Result<(), String> {
    let mut app = registry.app_settings();
    app.sandbox.policy.windows.write_acl = value;
    registry.set_app_settings(app)
}

/// 目的：能力缺失时的持久收口——把四项强制等级都降为 best_effort。 只降“要求等级”，不关闭已有的机制：装得上时仍照常强制，只是缺了不再拒绝启动。
pub(crate) fn lower_enforce_to_best_effort(
    registry: &mut dyn crate::capabilities::registry::api::Registry,
) -> Result<(), String> {
    use crate::capabilities::registry::api::SandboxEnforceLevel::BestEffort;
    let mut app = registry.app_settings();
    app.sandbox.policy.enforce.filesystem = BestEffort;
    app.sandbox.policy.enforce.network = BestEffort;
    app.sandbox.policy.enforce.process_tree = BestEffort;
    app.sandbox.policy.enforce.env_whitelist = BestEffort;
    registry.set_app_settings(app)
}

/// 目的：首次启动（没有 settings.yaml）时先给一次授权机会；只 Windows、只 TTY、无环境变量覆盖。 返回 true = 已写盘，调用方应以 75 整体重启；false = 没写（用户暂不授权/不适用），继续走启动门。
#[allow(clippy::too_many_arguments)]
pub(crate) fn maybe_onboard_windows<R: BufRead, W: Write>(
    missing: &[MissingPart],
    settings_exists: bool,
    tty: bool,
    fence_write_env: Option<&str>,
    forced_degraded: Option<&str>,
    cap: &crate::kernel::detail::confine::Capability,
    registry: &mut dyn crate::capabilities::registry::api::Registry,
    reader: &mut R,
    writer: &mut W,
) -> Result<bool, String> {
    let needs_authorization = missing
        .iter()
        .any(|p| p.cause == MissingCause::Unauthorized);
    let has_unsupported = missing.iter().any(|p| p.cause == MissingCause::Unsupported);
    if !cfg!(windows)
        || !needs_authorization
        || has_unsupported
        || settings_exists
        || !tty
        || fence_write_env.is_some()
        || forced_degraded.is_some()
        || !cap.fs
        || !cap.net
    {
        return Ok(false);
    }
    let _ = writeln!(
        writer,
        "[围栏] Windows 的容器围栏需要授权 Solomni 在工具执行时给 agent 沙箱目录写 ACL。"
    );
    let _ = writeln!(
        writer,
        "[围栏] 授权会记进台账，可用 --fence-clean 撤销；程序默认不写。"
    );
    if !ask_yes_no(reader, writer, "[围栏] 现在授权并启用容器围栏？", true) {
        return Ok(false);
    }
    write_acl_setting(registry, true)?;
    Ok(true)
}

/// 目的：启动门的处置结论——继续（已同意降级）、写盘后重启、或退出。
pub(crate) enum GateOutcome {
    Proceed,
    Restart,
    Exit(i32),
}

/// 目的：环境不满足时的唯一出路：本次降级 / 重设 / 退出；不允许默认进入会话。
/// 约束：没有可回答的前端、或测试注入强制严格时，一律退出（不启动 CLI / Web）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_gate<R: BufRead, W: Write>(
    missing: &[MissingPart],
    ask_allowed: bool,
    tty: bool,
    forced_degraded: Option<&str>,
    fence_write_env: Option<&str>,
    registry: &mut dyn crate::capabilities::registry::api::Registry,
    reader: &mut R,
    writer: &mut W,
) -> GateOutcome {
    if forced_degraded == Some("0") {
        let _ = writeln!(
            writer,
            "[沙箱] SOLOMNI_SANDBOX_DEGRADED=0：按全局策略拒绝执行，不进入会话。"
        );
        return GateOutcome::Exit(1);
    }
    if forced_degraded == Some("1") {
        let _ = writeln!(
            writer,
            "[沙箱] SOLOMNI_SANDBOX_DEGRADED=1 不能覆盖 on_unavailable=refuse。"
        );
        return GateOutcome::Exit(1);
    }
    if !tty {
        let _ = writeln!(
            writer,
            "[沙箱] 没有可回答的前端：不进入会话（fail-closed）。"
        );
        let _ = writeln!(
            writer,
            "[沙箱] 要启用：在 .home/settings.yaml 写 sandbox.policy.windows.write_acl: true，或设置 SOLOMNI_FENCE_WRITE=1。"
        );
        return GateOutcome::Exit(1);
    }

    if ask_allowed {
        let _ = writeln!(writer, "[沙箱] 本次无围栏运行？（不落盘，重启后重新询问）");
        if ask_yes_no(reader, writer, "[沙箱] 继续？", false) {
            return GateOutcome::Proceed;
        }
    } else {
        let _ = writeln!(writer, "[沙箱] 全局策略不允许本次降级。");
    }

    if !ask_yes_no(reader, writer, "[沙箱] 现在重新设置？", true) {
        let _ = writeln!(writer, "[沙箱] 已退出，未进入会话。");
        return GateOutcome::Exit(0);
    }

    let unsupported = missing.iter().any(|p| p.cause == MissingCause::Unsupported);
    if unsupported {
        let _ = writeln!(
            writer,
            "[沙箱] 本机能力不足：{}。",
            missing_names(missing, MissingCause::Unsupported)
        );
        let _ = writeln!(
            writer,
            "[沙箱] 重新设置只能把强制等级降为 best_effort（持久接受无强制）。"
        );
        if !ask_yes_no(reader, writer, "[沙箱] 继续？", false) {
            let _ = writeln!(writer, "[沙箱] 已退出，未进入会话。");
            return GateOutcome::Exit(0);
        }
        return match lower_enforce_to_best_effort(registry) {
            Ok(()) => GateOutcome::Restart,
            Err(e) => {
                let _ = writeln!(writer, "[沙箱] 写入设置失败：{}", e);
                GateOutcome::Exit(1)
            }
        };
    }

    if fence_write_env.is_some() {
        let _ = writeln!(
            writer,
            "[沙箱] SOLOMNI_FENCE_WRITE 正在覆盖设置；改环境变量后重启，或退出。"
        );
        let _ = writeln!(writer, "[沙箱] 已退出，未进入会话。");
        return GateOutcome::Exit(0);
    }

    let _ = writeln!(
        writer,
        "[沙箱] 需要授权 Windows 写目录 ACL 才能启用容器围栏。"
    );
    if !ask_yes_no(reader, writer, "[沙箱] 现在授权并启用？", true) {
        let _ = writeln!(writer, "[沙箱] 已退出，未进入会话。");
        return GateOutcome::Exit(0);
    }
    match write_acl_setting(registry, true) {
        Ok(()) => GateOutcome::Restart,
        Err(e) => {
            let _ = writeln!(writer, "[沙箱] 写入设置失败：{}", e);
            GateOutcome::Exit(1)
        }
    }
}

/// 目的：启动报告与 Web 概览共用的一份围栏事实：本机能力 + 本次实际（授权后的路径级围栏）。
/// 约束：授权只影响 Windows 的目录 ACL；其它平台由 `Capability::effective_paths` 判成与本机能力相同。
pub(crate) struct FenceReport {
    pub(crate) cap: crate::kernel::detail::confine::Capability,
    pub(crate) write_allowed: bool,
    pub(crate) fs: bool,
    pub(crate) net: bool,
    pub(crate) sandbox_mode: std::sync::Arc<crate::kernel::api::SandboxMode>,
}

impl FenceReport {
    /// 目的：按本机能力与本次授权算出同一份事实；启动报告与 Web 不再各编一套。
    pub(crate) fn new(
        cap: crate::kernel::detail::confine::Capability,
        write_allowed: bool,
        sandbox_mode: std::sync::Arc<crate::kernel::api::SandboxMode>,
    ) -> Self {
        let (fs, net) = cap.effective_paths(write_allowed);
        Self {
            cap,
            write_allowed,
            fs,
            net,
            sandbox_mode,
        }
    }
}

/// 目的：本次授权状态的人类可读标签——Unix 没有外层写 ACL 这一步，不能显示成"未授权"。
pub(crate) fn fence_auth_label(write_allowed: bool) -> &'static str {
    #[cfg(windows)]
    {
        if write_allowed {
            "已授权"
        } else {
            "未授权"
        }
    }
    #[cfg(not(windows))]
    {
        let _ = write_allowed;
        "不适用"
    }
}

/// 目的：本次授权影响的说明——只讲 Windows 的目录 ACL，避免让 Unix 读者以为还有一步授权没做。
pub(crate) fn fence_auth_note(write_allowed: bool) -> &'static str {
    #[cfg(windows)]
    {
        if write_allowed {
            "容器围栏按本机能力装，写入的权限项逐条记台账"
        } else {
            "不写本机任何权限项，容器围栏不装，只放行进程树、资源上限与环境白名单；要启用：设置里打开，或 .home/settings.yaml 写 sandbox.policy.windows.write_acl: true"
        }
    }
    #[cfg(not(windows))]
    {
        let _ = write_allowed;
        "sandbox.policy.windows.write_acl 只影响 Windows 的目录 ACL；本平台围栏在守门进程里自足，与授权无关"
    }
}
