//! 目的：`process` 的 Windows 后端——容器围栏准备、幂等收尾与 PowerShell shell。
//! 管：先授权可达范围再拉起守门进程；Job Object 收树；PowerShell 临时脚本与程序选择。
//! 不管：一次执行的公共轮询与回执（在门面）；围栏策略与文案。
//! 联动：由 `src/kernel/detail/process.rs` 按平台选中；围栏机制在 `confine`。

use super::*;
use crate::kernel::ports::{FenceCase, FenceVerdict, ShellRun};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Instant;

/// 目的：Windows 的围栏准备——用户授权写 ACL 时先授权，未授权时只提示一次并自检机制。
pub(super) fn prepare(ctx: PrepareCtx<'_>) -> Prepared {
    let tools = ctx.tools;
    let fence = ctx.fence;
    let mut prepared = false;
    let mut note: Option<String> = None;
    if tools.write_allowed.load(Ordering::Relaxed) {
        // 围栏准备单独计时：首调用逐条写 ACL 常常占掉整次耗时的绝大部分，回执要能分辨它。
        let fence_started = Instant::now();
        let prep = confine::prepare_fence(fence, ctx.command, &tools.home);
        let steps = prep.steps.clone();
        super::FENCE_PREP.with(|slot| {
            slot.set(Some(FenceTiming {
                total: fence_started.elapsed(),
                steps,
            }))
        });
        // 可选落点授不上只记事实（不牵动这次执行）：脚印打在 stderr，回执里不打扰模型。
        for n in &prep.notes {
            eprintln!("[围栏] {}（可选落点，本次照常执行）", n);
        }
        // 必要落点授不上：机制只把事实交给裁决端口；没有端口就按 fail-closed 拒绝。
        if prep.ok() {
            prepared = true;
        } else if let Some(blocked) = &prep.blocked {
            let Some(gate) = ctx.gate else {
                return Prepared {
                    stop: Some(RunOutcome {
                        ok: false,
                        output: tools.texts.fence_failed(),
                    }),
                    prepared: false,
                    unfenced_note: None,
                    home: None,
                };
            };
            let case = FenceCase {
                blocked,
                strict: ctx.strict,
                unfenced_possible: !fence.cwd.as_os_str().is_empty() && fence.cwd.is_dir(),
                agent: &fence.agent,
                ask: ctx.ask,
            };
            match gate.decide(&case) {
                FenceVerdict::Unfenced { note: n } => note = Some(n),
                FenceVerdict::Refused { output } => {
                    return Prepared {
                        stop: Some(RunOutcome { ok: false, output }),
                        prepared: false,
                        unfenced_note: None,
                        home: None,
                    };
                }
            }
        }
    } else if !tools.disclosed.swap(true, Ordering::Relaxed) {
        eprintln!(
            "[围栏] 容器围栏未启用（用户没授权在本机写权限）：外部工具按无围栏执行（能力等级见启动报告）。要启用：在设置里打开，或在 .home/settings.yaml 写 sandbox.policy.windows.write_acl: true"
        );
    }
    if !prepared
        && note.is_none()
        && let Some(outcome) =
            refuse_when_broken(&*tools.texts, confine::verify(fence, ctx.command))
    {
        return Prepared {
            stop: Some(outcome),
            prepared: false,
            unfenced_note: None,
            home: None,
        };
    }
    Prepared {
        stop: None,
        prepared,
        unfenced_note: note,
        home: Some(tools.home.clone()),
    }
}

impl ProcTools {
    /// 目的：强制 shell 用哪个 PowerShell（Windows 真机验 5.1 回退）；None = 自动。
    #[cfg(test)]
    pub(super) fn with_powershell(mut self, program: Option<&'static str>) -> ProcTools {
        self.shell_ps = program;
        self
    }
}

/// 目的：Windows 的进程树由守门进程的 Job Object 兜住，拉起前不需要额外配置。
pub(super) fn configure_spawn(_cmd: &mut std::process::Command) {}

/// 目的：Windows 杀树——守门进程一死，它 Job Object 里的整棵树随之消亡。
pub(super) fn kill_tree(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// 目的：Windows 的 shell——把脚本写成临时 `.ps1` 交给 PowerShell 执行。
pub(super) fn run_shell(tools: &ProcTools, run: &ShellRun<'_>) -> RunOutcome {
    let (name, file) = match tools.shell_script_file(run) {
        Ok(v) => v,
        Err(e) => {
            return RunOutcome {
                ok: false,
                output: e,
            };
        }
    };
    let command = format!(
        "{} -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File {}",
        tools.powershell_program(),
        name
    );
    // shell 的脚本不在命令行里，解释器基线看不到 node：Windows 容器里 node 需要跳过 realpath。
    let env = vec![(
        "NODE_OPTIONS".to_string(),
        confine::NODE_REALPATH_FLAGS.to_string(),
    )];
    let out = tools.run_inner(
        run.fence,
        &command,
        "",
        &env,
        run.path,
        run.ask,
        run.gate,
        RunPlan {
            timeout: run.timeout,
            strict: true,
            shell: true,
            cancel: run.cancel,
        },
    );
    let _ = std::fs::remove_file(&file);
    out
}

/// 目的：未授权时段遇到机制自检结论时的放行规矩——**只有本机装不上能降级**；
///   自检已确认机制有效却仍装不上 = 我们写错了，所以拒绝执行（命令不落进程）。
pub(crate) fn refuse_when_broken(
    texts: &dyn ProcessTexts,
    verdict: confine::FenceVerdict,
) -> Option<RunOutcome> {
    match verdict {
        confine::FenceVerdict::Broken(why) => {
            eprintln!(
                "[围栏] 容器围栏机制装不上（{}）：本次拒绝执行，不按无围栏跑",
                why
            );
            Some(RunOutcome {
                ok: false,
                output: texts.fence_failed(),
            })
        }
        confine::FenceVerdict::Enforced | confine::FenceVerdict::EnvUnavailable(_) => None,
    }
}

impl ProcTools {
    /// 目的：这次 shell 用哪个 PowerShell（shell 后端与测试共用）。
    pub(super) fn powershell_program(&self) -> &'static str {
        resolve_powershell(self)
    }

    /// 目的：把 shell 脚本写成临时 `.ps1`（shell 后端与测试共用）。
    pub(super) fn shell_script_file(
        &self,
        run: &ShellRun<'_>,
    ) -> Result<(String, PathBuf), String> {
        write_shell_script(run)
    }
}

/// 目的：这次 shell 用哪个 PowerShell：显式覆盖优先，否则 pwsh 7 优先、powershell 5.1 回退。
fn resolve_powershell(tools: &ProcTools) -> &'static str {
    if let Some(program) = tools.shell_ps {
        return program;
    }
    if crate::kernel::detail::host_probe::find_exe("pwsh").is_some() {
        "pwsh"
    } else {
        "powershell"
    }
}

/// 目的：Windows PowerShell 5.1 在容器里自动加载内核模块会带非终止错误；先按 Continue 载入常用模块，
///   再开 Stop（否则 Stop 会把加载错误变终止，Set-Location 之类就成了「命令不存在」）。
const PS_PRELUDE: &str = concat!(
    "try { Import-Module Microsoft.PowerShell.Management, Microsoft.PowerShell.Utility -ErrorAction SilentlyContinue } catch { }\n",
    "$ErrorActionPreference = 'Stop'\n",
    "try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch { }\n",
    "$OutputEncoding = [System.Text.Encoding]::UTF8\n",
    // 当前定位必须落在脚本所在目录（即工具进程的工作目录）：切不过去就如实报到 stderr。
    "try { Set-Location -LiteralPath $PSScriptRoot -ErrorAction Stop }\n",
    "catch {\n",
    "    if ((Get-Location).Path -ne $PSScriptRoot) {\n",
    "        [Console]::Error.WriteLine((\"[shell] 当前定位没能切到工作目录（{0}）：{1}\" -f $PSScriptRoot, $_.Exception.Message))\n",
    "    }\n",
    "}\n",
);

/// 目的：PowerShell 脚本的退出码尾巴——原生进程的码原样带出。
const PS_TAIL: &str = "\nif ($null -ne $LASTEXITCODE) { exit $LASTEXITCODE }\n";

/// 目的：把 shell 脚本写进 cwd 下的临时 .ps1（UTF-8 BOM + 错误即止 / 输出编码前导 + 退出码尾巴）；返回相对名与全路径。
/// 约束：create_new 防跟随同名符号链接；名字由本进程生成，不含模型内容。
fn write_shell_script(run: &ShellRun<'_>) -> Result<(String, PathBuf), String> {
    let dir = if !run.fence.cwd.as_os_str().is_empty() {
        run.fence.cwd.clone()
    } else {
        run.fence.private.clone()
    };
    if dir.as_os_str().is_empty() {
        return Err("shell 没有可写的私有沙箱（cwd 为空）".to_string());
    }
    let pid = std::process::id();
    for n in 0..100u32 {
        let name = format!(".solomni-shell-{}-{}.ps1", pid, n);
        let path = dir.join(&name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                use std::io::Write;
                // 未处理错误由包装器收成 exit 1：不依赖宿主进程对"未捕获终止错误"的收场路径。
                let body = format!(
                    "{}try {{\n{}\n}} catch {{\n    [Console]::Error.WriteLine(($_ | Out-String))\n    exit 1\n}}\n{}",
                    PS_PRELUDE, run.script, PS_TAIL
                );
                let mut bytes = vec![0xEF, 0xBB, 0xBF];
                bytes.extend_from_slice(body.as_bytes());
                if let Err(e) = f.write_all(&bytes) {
                    let _ = std::fs::remove_file(&path);
                    return Err(format!("写 shell 临时脚本失败：{}", e));
                }
                return Ok((name, path));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("建 shell 临时脚本失败：{}", e)),
        }
    }
    Err("建 shell 临时脚本失败：名字用完了".to_string())
}
