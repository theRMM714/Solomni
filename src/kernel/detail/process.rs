//! 目的：工具执行适配器——把本能力放行的工具命令拉进围栏里跑（实现 kernel 共享的 ProcessRunner 端口）。
//! 管：外层拉起守门进程（本程序的 --fence-run 模式）；由它把围栏与环境白名单装进真正的工具进程；stdin 送参、stdout/stderr 截获、超时连根杀树、输出截断。
//! 不管：命令行与可达范围的策略（由调用方派生后传入）；命令来自 module.yaml，JSON 参数走 stdin（不进命令行）。
//! 联动：围栏机制在 confine；ProcessRunner 端口形状见 src/kernel/ports.rs。

use crate::kernel::api::{FenceSpec, RunOutcome};
use crate::kernel::detail::confine;
use crate::kernel::ports::FenceGate;
#[cfg(windows)]
use crate::kernel::ports::{FenceCase, FenceVerdict};
use crate::kernel::ports::{ProcessRunner, ProcessTexts};
use std::path::PathBuf;
use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant};

pub struct ProcTools {
    /// 目的：守门进程用的可执行文件（组合根注入当前程序路径）。
    pub exe: PathBuf,
    /// 目的：产品私有区（`.home/`）：围栏授权台账落在这里，供 `--fence-clean` 精确回收。
    ///   只有 Windows 的容器围栏需要写目录 ACL，所以下面这三项只在本平台存在。
    #[cfg(windows)]
    pub home: PathBuf,
    /// 目的：是否允许在本机写权限（由组合根按设置与 `SOLOMNI_FENCE_WRITE` 注入；默认否）。
    #[cfg(windows)]
    pub write_allowed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 目的：「未授权」的提示只出一次，不刷屏。
    #[cfg(windows)]
    pub disclosed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 目的：工具回执里那些收尾标记的文案（生产用本文件的 `CodeProcessTexts`；它们随 [工具结果] 进模型上下文）。
    ///   **共享一份**（调用方给出的 `Arc`）：这里不再各存一份深拷贝。
    pub texts: std::sync::Arc<dyn ProcessTexts>,
    /// 目的：单次工具执行的超时（到时连根杀掉整棵树，ok = false）。
    pub timeout: Duration,
    /// 目的：回传给模型/轨迹的输出上限（字符数）。
    pub max_output_chars: usize,
    /// 目的：全局沙箱要求与本进程是否已同意降级。
    pub mode: std::sync::Arc<crate::kernel::api::SandboxMode>,
    /// 目的：强制使用的 PowerShell 程序（pwsh / powershell）；None = 自动检测。Windows 专用。
    #[cfg(windows)]
    pub shell_ps: Option<&'static str>,
}

/// 目的：进程机制自己的收尾文案（不再经提示词册；它们随 [工具结果] 进模型上下文）。
pub struct CodeProcessTexts;

impl ProcessTexts for CodeProcessTexts {
    fn stderr_header(&self) -> String {
        "[stderr]".to_string()
    }
    fn timeout(&self) -> String {
        "[超时] 工具进程与它拉起的整棵进程树已被终止".to_string()
    }
    fn fence_failed(&self) -> String {
        "[围栏] 工具进程没有进入围栏：命令未执行（当前能力等级见启动报告）".to_string()
    }
    fn truncated(&self, chars: &str, limit: &str) -> String {
        format!("[截断] 输出共 {} 字符，仅保留前 {}", chars, limit)
    }
    fn truncated_middle(&self, chars: &str, head: &str, tail: &str) -> String {
        format!(
            "[截断] 输出共 {} 字符；保留开头 {} 与结尾 {} 字符（按完整行取整）",
            chars, head, tail
        )
    }
    fn shell_exit(&self, code: Option<i32>) -> String {
        match code {
            Some(c) => format!("[shell] 退出码 {}", c),
            None => "[shell] 退出码未知（被信号结束）".to_string(),
        }
    }
    fn shell_timeout(&self, secs: u64) -> String {
        format!(
            "[shell] 超时（{} 秒）：工具进程与它拉起的整棵进程树已被终止",
            secs
        )
    }
    fn shell_cancelled(&self) -> String {
        "[shell] 用户停止：工具进程与它拉起的整棵进程树已被终止".to_string()
    }
}

impl ProcTools {
    /// 目的：组合根注入：当前可执行文件（守门进程就是它自己）、进程收尾文案、产品私有区与写权限开关。
    pub fn new(
        exe: PathBuf,
        texts: std::sync::Arc<dyn ProcessTexts>,
        home: PathBuf,
        write_allowed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> ProcTools {
        // 其它平台没有容器围栏这一步：家目录与写权限开关不参与装配，显式忽略以免被误读成漏用。
        #[cfg(not(windows))]
        let _ = (&home, &write_allowed);
        ProcTools {
            exe,
            #[cfg(windows)]
            home,
            #[cfg(windows)]
            write_allowed,
            #[cfg(windows)]
            disclosed: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            texts,
            timeout: Duration::from_secs(30),
            max_output_chars: 16_000,
            mode: std::sync::Arc::new(crate::kernel::api::SandboxMode::permissive()),
            #[cfg(windows)]
            shell_ps: None,
        }
    }

    /// 目的：组合根注入全局沙箱模式；缺省是“无要求”的测试替身。
    pub fn with_mode(mut self, mode: std::sync::Arc<crate::kernel::api::SandboxMode>) -> ProcTools {
        self.mode = mode;
        self
    }

    /// 目的：强制 shell 用哪个 PowerShell（Windows 真机验 5.1 回退）；None = 自动。
    #[cfg(all(windows, test))]
    pub fn with_powershell(mut self, program: Option<&'static str>) -> ProcTools {
        self.shell_ps = program;
        self
    }
}

impl ProcessRunner for ProcTools {
    // 提问与裁决端口只在 Windows 的容器围栏那一路用；unix 侧没有 prepare_fence，参数如实闲置。
    #[cfg_attr(not(windows), allow(unused_variables))]
    fn run(
        &self,
        fence: &FenceSpec,
        command: &str,
        args_json: &str,
        env: &[(String, String)],
        path: Option<&std::ffi::OsStr>,
        ask: Option<&dyn crate::kernel::ports::AskUser>,
        gate: Option<&dyn FenceGate>,
    ) -> RunOutcome {
        self.run_inner(
            fence,
            command,
            args_json,
            env,
            path,
            ask,
            gate,
            RunPlan {
                timeout: self.timeout,
                strict: false,
                shell: false,
                cancel: None,
            },
        )
    }

    fn run_shell(&self, run: &crate::kernel::ports::ShellRun<'_>) -> RunOutcome {
        self.run_shell_impl(run)
    }
}

/// 目的：一次进程执行的机制选项——模块工具用默认超时与降级策略；shell 用按次超时、严格围栏与取消。
struct RunPlan<'a> {
    timeout: Duration,
    strict: bool,
    shell: bool,
    cancel: Option<&'a std::sync::atomic::AtomicBool>,
}

impl ProcTools {
    /// 目的：一次进程执行的公共体（准备围栏、拉起守门进程、送 stdin、等待/取消/超时、拼回执）。
    /// 参数：`plan` 区分模块工具与 shell；strict = 围栏必须真强制，由裁决端口按该事实处理。
    #[allow(clippy::too_many_arguments)]
    #[cfg_attr(not(windows), allow(unused_variables))]
    fn run_inner(
        &self,
        fence: &FenceSpec,
        command: &str,
        stdin_payload: &str,
        env: &[(String, String)],
        path: Option<&std::ffi::OsStr>,
        ask: Option<&dyn crate::kernel::ports::AskUser>,
        gate: Option<&dyn FenceGate>,
        plan: RunPlan<'_>,
    ) -> RunOutcome {
        // Windows：容器围栏要先把「可达范围」授权给容器 SID。
        // **默认不写本机任何权限项**：只有用户显式授权（设置里的 sandbox.policy.windows.write_acl，或环境变量 SOLOMNI_FENCE_WRITE=1）才做。
        #[cfg(windows)]
        let (prepared, unfenced_note) = {
            use std::sync::atomic::Ordering;
            let mut prepared = false;
            let mut note: Option<String> = None;
            if self.write_allowed.load(Ordering::Relaxed) {
                let prep = confine::prepare_fence(fence, command, &self.home);
                // 可选落点授不上只记事实（不牵动这次执行）：脚印打在 stderr，回执里不打扰模型。
                for n in &prep.notes {
                    eprintln!("[围栏] {}（可选落点，本次照常执行）", n);
                }
                // 必要落点授不上：机制只把事实交给裁决端口；没有端口就按 fail-closed 拒绝。
                if prep.ok() {
                    prepared = true;
                } else if let Some(blocked) = &prep.blocked {
                    let Some(gate) = gate else {
                        return RunOutcome {
                            ok: false,
                            output: self.texts.fence_failed(),
                        };
                    };
                    let case = FenceCase {
                        blocked,
                        strict: plan.strict,
                        unfenced_possible: !fence.cwd.as_os_str().is_empty() && fence.cwd.is_dir(),
                        agent: &fence.agent,
                        ask,
                    };
                    match gate.decide(&case) {
                        FenceVerdict::Unfenced { note: n } => note = Some(n),
                        FenceVerdict::Refused { output } => {
                            return RunOutcome { ok: false, output }
                        }
                    }
                }
            } else if !self.disclosed.swap(true, Ordering::Relaxed) {
                eprintln!(
                    "[围栏] 容器围栏未启用（用户没授权在本机写权限）：外部工具按无围栏执行（能力等级见启动报告）。要启用：在设置里打开，或在 .home/settings.yaml 写 sandbox.policy.windows.write_acl: true"
                );
            }
            if !prepared && note.is_none() {
                // 未授权时段先问机制：环境不允许就如实降级，我们写错了就拒绝执行（见 refuse_when_broken）。
                if let Some(outcome) =
                    refuse_when_broken(&*self.texts, confine::verify(fence, command))
                {
                    return outcome;
                }
            }
            (prepared, note)
        };
        // 其它平台没有容器围栏，也就没有"要先授权"这一步。
        #[cfg(not(windows))]
        let (prepared, unfenced_note): (bool, Option<String>) = (false, None);
        // 容器 profile 的台账落点：只有 Windows 的守门进程会写它（外层不知道 profile 建成了没有）。
        #[cfg(windows)]
        let home = Some(self.home.clone());
        #[cfg(not(windows))]
        let home: Option<std::path::PathBuf> = None;
        // 全局沙箱要求：机制不可用且本进程未同意降级时，工具**不执行**。
        // shell 严格模式不看本进程降级：它的围栏要求恒为真，由守门进程按 FENCE_FAILED 拒绝。
        if !plan.strict && unfenced_note.is_none() {
            if let Err(why) = self.mode.check(fence, prepared) {
                return RunOutcome {
                    ok: false,
                    output: why,
                };
            }
        }
        let policy = if plan.strict {
            crate::kernel::api::FenceRunPolicy {
                require_fs: true,
                require_net: true,
                allow_degraded: false,
            }
        } else {
            crate::kernel::api::FenceRunPolicy {
                require_fs: self.mode.requirements.filesystem,
                require_net: self.mode.requirements.network,
                allow_degraded: self.mode.degraded(),
            }
        };
        let job = confine::FenceJob {
            spec: fence.clone(),
            prepared,
            home,
            policy,
        };
        let mut cmd = confine::launcher(&self.exe, &job, command);
        cmd.current_dir(&fence.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // 环境白名单：不继承父进程环境（密钥与无关凭据不进工具进程）；HOME/TEMP 落进该 agent 的私有沙箱。
        cmd.env_clear();
        for (k, v) in confine::fence_env(fence, command) {
            cmd.env(k, v);
        }
        // 本机档运行包的生效 PATH：只覆盖这一趟，不改进程全局。
        if let Some(p) = path {
            cmd.env("PATH", p);
        }
        // 该模块隐私字段的注入项：只走环境，不进命令行（值也不进提示词 / 转录 / 日志）。
        for (k, v) in env {
            cmd.env(k, v);
        }
        // 独立进程组：Unix 上超时/停止能杀整棵树；Windows 侧由守门进程的 Job Object 兜住。
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return RunOutcome {
                    ok: false,
                    output: format!("工具进程启动失败：{}", e),
                }
            }
        };
        // stdin 独立线程送参：参数再大也不与 stdout 读取互锁。
        let mut stdin = child.stdin.take().expect("stdin 已声明管道");
        let payload = format!("{}\n", stdin_payload);
        let writer = thread::spawn(move || {
            use std::io::Write;
            let _ = stdin.write_all(payload.as_bytes());
            let _ = stdin.flush();
        });
        let mut stdout_pipe = child.stdout.take().expect("stdout 已声明管道");
        let mut stderr_pipe = child.stderr.take().expect("stderr 已声明管道");
        let out_reader = thread::spawn(move || read_to_string(&mut stdout_pipe));
        let err_reader = thread::spawn(move || read_to_string(&mut stderr_pipe));

        // 轮询等待至截止；取消旗先看（停止要立刻生效），到点再杀整棵树。
        let deadline = Instant::now() + plan.timeout;
        let mut timed_out = false;
        let mut cancelled = false;
        loop {
            if let Some(flag) = plan.cancel {
                if flag.load(std::sync::atomic::Ordering::Relaxed) {
                    cancelled = true;
                    kill_tree(&mut child);
                    break;
                }
            }
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        timed_out = true;
                        kill_tree(&mut child);
                        break;
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }
        let _ = writer.join();
        let out = out_reader.join().unwrap_or_default();
        let err = err_reader.join().unwrap_or_default();
        let code = child.try_wait().ok().flatten().and_then(|s| s.code());
        let mut outcome = if plan.shell {
            self.assemble_shell(out, err, timed_out, cancelled, code, plan.timeout)
        } else {
            self.assemble(out, err, timed_out, code)
        };
        // 用户裁决"本轮无围栏跑一次"：这次执行在回执里**如实标为无围栏**（模型与用户都看得到）。
        if let Some(note) = unfenced_note {
            outcome.output.push('\n');
            outcome.output.push_str(&note);
        }
        outcome
    }

    /// 目的：本机 shell 的启动方式：Windows 用 PowerShell（-File 临时脚本），其它平台用 sh（脚本走 stdin）。
    /// 约束：脚本是数据，不拼进命令行；Windows 的临时脚本落在 shell 的 cwd（私有沙箱）里，跑完删掉。
    #[cfg(not(windows))]
    fn run_shell_impl(&self, run: &crate::kernel::ports::ShellRun<'_>) -> RunOutcome {
        // 外层守门进程本身就是 sh -c：这里再起一个 sh 读 stdin，脚本不会与命令的 stdin 抢同一条流。
        let command = "sh -c \"$(cat)\"";
        self.run_inner(
            run.fence,
            command,
            run.script,
            &[],
            run.path,
            run.ask,
            run.gate,
            RunPlan {
                timeout: run.timeout,
                strict: true,
                shell: true,
                cancel: run.cancel,
            },
        )
    }

    #[cfg(windows)]
    fn run_shell_impl(&self, run: &crate::kernel::ports::ShellRun<'_>) -> RunOutcome {
        let (name, file) = match self.shell_script_file(run) {
            Ok(v) => v,
            Err(e) => {
                return RunOutcome {
                    ok: false,
                    output: e,
                }
            }
        };
        let command = format!(
            "{} -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File {}",
            self.powershell_program(),
            name
        );
        // shell 的脚本不在命令行里，解释器基线看不到 node：Windows 容器里 node 需要跳过 realpath。
        let env = vec![(
            "NODE_OPTIONS".to_string(),
            confine::NODE_REALPATH_FLAGS.to_string(),
        )];
        let out = self.run_inner(
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

    /// 目的：拼 shell 回执：正文分区 + 截断 + 退出码 / 超时 / 用户停止 / 围栏失败。
    /// 约束：形状与本机/guest 共用（见 kernel::domain::receipt）；cwd 与耗时由工具层追加。
    fn assemble_shell(
        &self,
        out: String,
        err: String,
        timed_out: bool,
        cancelled: bool,
        code: Option<i32>,
        timeout: Duration,
    ) -> RunOutcome {
        let end = if cancelled {
            crate::kernel::domain::receipt::ShellEnd::Cancelled
        } else if timed_out {
            crate::kernel::domain::receipt::ShellEnd::TimedOut {
                secs: timeout.as_secs(),
            }
        } else {
            crate::kernel::domain::receipt::ShellEnd::Exit(code)
        };
        let mut output = crate::kernel::domain::receipt::shell_receipt(
            self.texts.as_ref(),
            &out,
            &err,
            end,
            self.max_output_chars,
        );
        if code == Some(confine::FENCE_FAILED) {
            output.push('\n');
            output.push_str(&self.texts.fence_failed());
        }
        RunOutcome {
            ok: !timed_out && !cancelled && code == Some(0),
            output,
        }
    }
}

/// 目的：Windows PowerShell 5.1 在容器里自动加载内核模块会带非终止错误；先按 Continue 载入常用模块，
///   再开 Stop（否则 Stop 会把加载错误变终止，Set-Location 之类就成了「命令不存在」）。
#[cfg(windows)]
const PS_PRELUDE: &str = "try { Import-Module Microsoft.PowerShell.Management, Microsoft.PowerShell.Utility -ErrorAction SilentlyContinue } catch { }\n$ErrorActionPreference = 'Stop'\ntry { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch { }\n$OutputEncoding = [System.Text.Encoding]::UTF8\ntry { Set-Location -LiteralPath $PSScriptRoot -ErrorAction Stop } catch { }\n";
#[cfg(windows)]
const PS_TAIL: &str = "\nif ($null -ne $LASTEXITCODE) { exit $LASTEXITCODE }\n";

#[cfg(windows)]
impl ProcTools {
    /// 目的：这次 shell 用哪个 PowerShell：显式覆盖优先，否则 pwsh 7 优先、powershell 5.1 回退。
    fn powershell_program(&self) -> &'static str {
        if let Some(program) = self.shell_ps {
            return program;
        }
        if crate::kernel::detail::host_probe::find_exe("pwsh").is_some() {
            "pwsh"
        } else {
            "powershell"
        }
    }
}

#[cfg(windows)]
impl ProcTools {
    /// 目的：把 shell 脚本写进 cwd 下的临时 .ps1（UTF-8 BOM + 错误即止 / 输出编码前导 + 退出码尾巴）；返回相对名与全路径。
    /// 约束：create_new 防跟随同名符号链接；名字由本进程生成，不含模型内容。
    fn shell_script_file(
        &self,
        run: &crate::kernel::ports::ShellRun<'_>,
    ) -> Result<(String, PathBuf), String> {
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
                    // 未处理错误由包装器收成 exit 1：不依赖宿主进程对"未捕获终止错误"的收场路径
                    // （Windows PowerShell 5.1 在容器里那条路会卡住），stderr 仍原样交给模型。
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
}

/// 目的：未授权时段遇到机制自检结论时的放行规矩——**只有本机装不上能降级**；
///   自检已确认机制有效却仍装不上 = 我们写错了（按无围栏跑等于用户以为有围栏、实际什么都没有），
///   所以拒绝执行（命令不落进程），回执用提示词册里的固定说法。
#[cfg(windows)]
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

/// 目的：杀掉整棵进程树——工具进程 fork 出来的子孙一并收掉（不留孤儿）。
pub(crate) fn kill_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        // 负 pid = 整个进程组（守门进程是组长，组员含 shell 与工具本身）。
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
        let _ = child.wait();
    }
    #[cfg(not(unix))]
    {
        // Windows：守门进程一死，它 Job Object 里的整棵树随之消亡（KILL_ON_JOB_CLOSE）。
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn read_to_string(r: &mut impl std::io::Read) -> String {
    let mut buf = Vec::new();
    let _ = r.read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

impl ProcTools {
    /// 把工具进程的原始输出与事实拼成回执：成功与否看退出码；
    /// stderr 段头、超时、围栏没装上、截断这些**标记文案全部来自提示词册**——它们随工具结果进模型上下文。
    fn assemble(&self, out: String, err: String, timed_out: bool, code: Option<i32>) -> RunOutcome {
        let mut ok = !timed_out && code == Some(0);
        let mut output = out;
        if !err.is_empty() {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(&self.texts.stderr_header());
            output.push('\n');
            output.push_str(&err);
        }
        if timed_out {
            output.push('\n');
            output.push_str(&self.texts.timeout());
        }
        // 围栏没装上：守门进程用固定退出码报明（命令没被执行），这里如实告诉模型。
        if code == Some(confine::FENCE_FAILED) {
            ok = false;
            output.push('\n');
            output.push_str(&self.texts.fence_failed());
        }
        RunOutcome {
            ok,
            output: self.truncate(&output),
        }
    }

    /// 按字符截断（不劈开 UTF-8），尾部如实注明（文案来自提示词册）。
    fn truncate(&self, s: &str) -> String {
        let count = s.chars().count();
        if count <= self.max_output_chars {
            return s.to_string();
        }
        let head: String = s.chars().take(self.max_output_chars).collect();
        let tail = self
            .texts
            .truncated(&count.to_string(), &self.max_output_chars.to_string());
        format!("{}\n{}", head, tail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::api::FenceSpec;
    // 测试夹具按 &Path 收参（clippy 的 ptr_arg）：Path 显式写在测试模块里，
    // 顶层只按需导入 PathBuf——否则顶层会多出一次"只被 glob 用到"的导入。
    use std::path::{Path, PathBuf};

    /// 环境白名单：父进程的无关变量（密钥之类）不进子进程；HOME / TEMP 落在该 agent 的私有沙箱里。
    #[test]
    fn env_whitelist_drops_foreign_vars_and_moves_home_into_the_sandbox() {
        std::env::set_var("SOLOMNI_PROBE_SECRET", "leak-me");
        let private = PathBuf::from("demo").join("agent-a");
        let spec = FenceSpec {
            agent: "a".to_string(),
            lease: String::new(),
            private: private.clone(),
            ro_tree: Vec::new(),
            rw: vec![PathBuf::from("demo").join("work"), private.clone()],
            cwd: PathBuf::from("mods").join("m0"),
            ro: Vec::new(),
            net: false,
        };
        let env: Vec<(String, String)> =
            crate::kernel::detail::confine::fence_env(&spec, "python tools/x.py")
                .into_iter()
                .map(|(k, v)| {
                    (
                        k.to_string_lossy().into_owned(),
                        v.to_string_lossy().into_owned(),
                    )
                })
                .collect();
        let get = |key: &str| env.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
        assert!(
            !env.iter().any(|(_, v)| v.contains("leak-me")),
            "无关变量不得进子进程"
        );
        assert!(get("SOLOMNI_PROBE_SECRET").is_none());
        assert_eq!(
            get("HOME").as_deref(),
            Some(private.to_string_lossy().as_ref()),
            "HOME 落在私有沙箱"
        );
        assert_eq!(
            get("TEMP").as_deref(),
            Some(private.to_string_lossy().as_ref())
        );
        // Windows 建 AppContainer 进程要读 LOCALAPPDATA：白名单里没有它，CreateProcessW 直接失败（os error 203），
        // 容器整条路会静默降级成无围栏——落户同样指进私有沙箱。
        assert_eq!(
            get("LOCALAPPDATA").as_deref(),
            Some(private.to_string_lossy().as_ref()),
            "LOCALAPPDATA 也落在私有沙箱"
        );
        assert_eq!(
            get("PYTHONIOENCODING").as_deref(),
            Some("utf-8"),
            "编码统一 UTF-8"
        );
        std::env::remove_var("SOLOMNI_PROBE_SECRET");
    }

    /// 回执里的标记文案由进程机制自己的代码返回（它们随 [工具结果] 进模型上下文）。
    #[test]
    fn receipt_markers_are_code_owned() {
        let texts = CodeProcessTexts;
        let tools = ProcTools::new(
            PathBuf::from("solomni"),
            std::sync::Arc::new(CodeProcessTexts),
            PathBuf::from("target").join("test-scratch"),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );

        let ok = tools.assemble("正常输出".to_string(), String::new(), false, Some(0));
        assert!(ok.ok);
        assert_eq!(ok.output, "正常输出", "没有异常就不加任何标记");

        let with_err = tools.assemble("正文".to_string(), "警告".to_string(), false, Some(0));
        assert!(
            with_err.output.contains(&texts.stderr_header()) && with_err.output.contains("警告")
        );

        let timed = tools.assemble(String::new(), String::new(), true, None);
        assert!(!timed.ok);
        assert!(timed.output.contains(&texts.timeout()), "{}", timed.output);

        let fenced = tools.assemble(
            String::new(),
            String::new(),
            false,
            Some(confine::FENCE_FAILED),
        );
        assert!(!fenced.ok);
        assert!(
            fenced.output.contains(&texts.fence_failed()),
            "{}",
            fenced.output
        );

        let long = "字".repeat(20_000);
        let cut = tools.assemble(long, String::new(), false, Some(0));
        assert!(
            cut.output.contains("20000"),
            "截断要如实报字符数：{}",
            &cut.output[cut.output.len() - 80..]
        );
    }

    /// 全局沙箱要求：机制不可用且未同意降级时拒绝；同意降级后放行；"允许联网"不需要网络围栏。
    #[test]
    fn sandbox_mode_refuses_missing_requirements_until_degraded() {
        use crate::kernel::api::{SandboxMode, SandboxRequirements};
        let spec = FenceSpec {
            agent: "a".to_string(),
            lease: String::new(),
            rw: Vec::new(),
            ro: Vec::new(),
            ro_tree: Vec::new(),
            private: PathBuf::new(),
            cwd: PathBuf::new(),
            net: false,
        };
        let mode = SandboxMode::new(
            SandboxRequirements {
                filesystem: true,
                network: true,
                process_tree: true,
                env_whitelist: true,
            },
            false,
            false,
            false,
            false,
            true,
        );
        assert!(mode.check(&spec, false).is_err(), "缺机制时默认拒绝");
        mode.set_degraded(true);
        assert!(mode.check(&spec, false).is_ok(), "本进程同意降级后放行");
        mode.set_degraded(false);
        let allow_net = FenceSpec {
            net: true,
            ro: Vec::new(),
            ..spec.clone()
        };
        // 文件系统仍然缺，所以还是拒绝；但“网络围栏”不应再单独要求。
        let err = mode.check(&allow_net, false).unwrap_err();
        assert!(!err.contains("网络围栏"), "{}", err);
    }

    /// 未授权时段的放行规矩：**只有本机装不上能降级**。
    /// 自检已确认机制有效却仍装不上 = 我们写错了，那一路必须拒绝执行——
    /// 按无围栏跑等于用户以为有围栏、实际什么都没有（回执文案由进程机制代码返回）。
    #[cfg(windows)]
    #[test]
    fn broken_mechanism_refuses_execution_instead_of_degrading() {
        let texts = code_texts();
        let broken = refuse_when_broken(
            &*texts,
            confine::FenceVerdict::Broken("profile 写错".to_string()),
        )
        .expect("我们写错了必须拒绝执行");
        assert!(!broken.ok, "拒绝执行时 ok 必须为假");
        assert_eq!(
            broken.output,
            texts.fence_failed(),
            "回执用册子里的固定说法"
        );
        assert!(
            refuse_when_broken(&*texts, confine::FenceVerdict::Enforced).is_none(),
            "机制装上了就没有拒绝的理由"
        );
        assert!(
            refuse_when_broken(
                &*texts,
                confine::FenceVerdict::EnvUnavailable("内核不支持".to_string())
            )
            .is_none(),
            "本机不允许是环境结论：如实降级照跑，不拒绝"
        );
    }

    // ---------- 真实工具进程（T2 真实适配器边界；见 docs/testing/doubles.md 的 ProcessRunner 行） ----------
    /// 已构建的产品可执行文件（守门进程就是它自己）：`cargo build` 之后才存在；没有就如实跳过。
    fn built_exe() -> Option<PathBuf> {
        let me = std::env::current_exe().ok()?;
        let profile_dir = me.parent()?.parent()?; // target/<profile>/deps → target/<profile>
        let name = if cfg!(windows) {
            "solomni.exe"
        } else {
            "solomni"
        };
        let p = profile_dir.join(name);
        if p.is_file() {
            Some(p)
        } else {
            None
        }
    }

    /// 本机可用的 python（没有就如实跳过需要解释器的用例）。
    fn python() -> Option<&'static str> {
        for name in ["python", "python3"] {
            if let Ok(o) = std::process::Command::new(name)
                .arg("-c")
                .arg("print(1)")
                .output()
            {
                if o.status.success() {
                    return Some(name);
                }
            }
        }
        None
    }

    /// 本机可用的 node（没有就如实跳过需要它的用例）。解释器基线只在 Windows 的容器围栏上需要。
    #[cfg(windows)]
    fn node() -> Option<&'static str> {
        let ok = std::process::Command::new("node")
            .arg("-v")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if ok {
            Some("node")
        } else {
            None
        }
    }

    fn code_texts() -> std::sync::Arc<dyn crate::kernel::ports::ProcessTexts> {
        std::sync::Arc::new(CodeProcessTexts)
    }

    /// 造一个按「cwd = 隔离根」跑真工具的 runner（命令用裸文件名，避免命令行里出现引号）。
    fn real_runner(exe: PathBuf, home: &std::path::Path, timeout_secs: u64) -> ProcTools {
        let mut t = ProcTools::new(
            exe,
            code_texts(),
            home.to_path_buf(),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        t.timeout = Duration::from_secs(timeout_secs);
        t
    }

    fn spec_for(dir: &Path) -> FenceSpec {
        FenceSpec {
            agent: "proc".to_string(),
            lease: String::new(),
            private: PathBuf::new(),
            ro_tree: Vec::new(),
            rw: vec![dir.to_path_buf()],
            cwd: dir.to_path_buf(),
            ro: Vec::new(),
            net: false,
        }
    }

    /// 真实工具进程：stdin 的 JSON 原样送达，退出码决定 ok（不猜、不吞）。
    #[test]
    fn real_tool_process_receives_stdin_json_and_reports_success() {
        let Some(exe) = built_exe() else {
            eprintln!(
                "[探针] 未找到已构建的 solomni 可执行文件（先 cargo build），跳过真实工具进程契约"
            );
            return;
        };
        let Some(py) = python() else {
            eprintln!("[探针] 本机没有可用的 python，跳过真实工具进程契约");
            return;
        };
        let dir = crate::tests::scratch("proc-tools-stdin");
        std::fs::write(
            dir.join("echo_stdin.py"),
            "import sys\nprint(sys.stdin.read().strip())\n",
        )
        .expect("写脚本");
        let tools = real_runner(exe, &dir, 60);
        let out = tools.run(
            &spec_for(&dir),
            &format!("{} echo_stdin.py", py),
            "{\"k\":\"v\"}",
            &[],
            None,
            None,
            None,
        );
        assert!(out.ok, "工具应当成功：{}", out.output);
        assert!(
            out.output.contains("{\"k\":\"v\"}"),
            "stdin 的 JSON 必须原样送达工具：{}",
            out.output
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 真实工具进程：父进程的无关变量（密钥之类）不得进工具进程——环境白名单要在真进程上生效。
    #[test]
    fn real_tool_process_does_not_inherit_foreign_env() {
        let Some(exe) = built_exe() else {
            eprintln!("[探针] 未找到已构建的 solomni 可执行文件（先 cargo build），跳过工具进程环境白名单契约");
            return;
        };
        let Some(py) = python() else {
            eprintln!("[探针] 本机没有可用的 python，跳过工具进程环境白名单契约");
            return;
        };
        let dir = crate::tests::scratch("proc-tools-env");
        std::fs::write(
            dir.join("echo_env.py"),
            "import os\nprint('LEAK=' + str(os.environ.get('SOLOMNI_PROBE_ENV_LEAK')))\n",
        )
        .expect("写脚本");
        std::env::set_var("SOLOMNI_PROBE_ENV_LEAK", "leak-me");
        let tools = real_runner(exe, &dir, 60);
        let out = tools.run(
            &spec_for(&dir),
            &format!("{} echo_env.py", py),
            "{}",
            &[],
            None,
            None,
            None,
        );
        std::env::remove_var("SOLOMNI_PROBE_ENV_LEAK");
        assert!(out.ok, "工具应当成功：{}", out.output);
        assert!(
            out.output.contains("LEAK=None"),
            "无关变量不得进工具进程：{}",
            out.output
        );
        assert!(
            !out.output.contains("leak-me"),
            "密钥不得进工具进程：{}",
            out.output
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 解释器基线要在**真进程**上成立：`node <文件>` 的工具进程拿到 `NODE_OPTIONS`（容器里 node 靠它跳过
    /// realpath，否则脚本执行前就 EPERM 死）；别的解释器看不到它——注入面只覆盖真正需要它的那一种命令。
    #[cfg(windows)]
    #[test]
    fn real_node_tool_process_carries_the_realpath_skip() {
        let Some(exe) = built_exe() else {
            eprintln!(
                "[探针] 未找到已构建的 solomni 可执行文件（先 cargo build），跳过解释器基线契约"
            );
            return;
        };
        let Some(node) = node() else {
            eprintln!("[探针] 本机没有可用的 node，跳过解释器基线契约");
            return;
        };
        let dir = crate::tests::scratch("proc-tools-node-env");
        std::fs::write(
            dir.join("echo_node_opts.js"),
            "process.stdout.write('OPTS=' + String(process.env.NODE_OPTIONS));",
        )
        .expect("写脚本");
        let tools = real_runner(exe, &dir, 60);
        let out = tools.run(
            &spec_for(&dir),
            &format!("{} echo_node_opts.js", node),
            "{}",
            &[],
            None,
            None,
            None,
        );
        assert!(out.ok, "node 工具应当成功：{}", out.output);
        for flag in ["--preserve-symlinks", "--preserve-symlinks-main"] {
            assert!(
                out.output.contains(flag),
                "工具进程要拿到解释器基线（缺 {}）：{}",
                flag,
                out.output
            );
        }
        if let Some(py) = python() {
            std::fs::write(
                dir.join("echo_node_opts.py"),
                "import os\nprint('OPTS=' + str(os.environ.get('NODE_OPTIONS')))\n",
            )
            .expect("写 python 脚本");
            let out = tools.run(
                &spec_for(&dir),
                &format!("{} echo_node_opts.py", py),
                "{}",
                &[],
                None,
                None,
                None,
            );
            assert!(out.ok, "python 工具应当成功：{}", out.output);
            assert!(
                out.output.contains("OPTS=None"),
                "非 node 命令不该带解释器基线：{}",
                out.output
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 命令里的**程序名**写成带正斜杠的相对路径也必须跑得起来：Windows 的 cmd 不认程序名里的 `/`
    /// （`build/indexer build` 会被它读成命令 `build` + 开关 `/indexer`），守门进程要按平台把程序名里的 `/` 转成 `\`。
    /// 参数里的正斜杠不受影响——`node tools/report.js` 这类命令靠的就是它。
    #[test]
    fn real_tool_process_runs_a_relative_program_path() {
        let Some(exe) = built_exe() else {
            eprintln!(
                "[探针] 未找到已构建的 solomni 可执行文件（先 cargo build），跳过相对程序名契约"
            );
            return;
        };
        let dir = crate::tests::scratch("proc-tools-relative-program");
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).expect("建子目录");
        let (name, body, command) = if cfg!(windows) {
            (
                "probe.cmd",
                "@echo off\r\necho PROBE-OK\r\n",
                "sub/probe.cmd",
            )
        } else {
            ("probe.sh", "echo PROBE-OK\n", "sub/probe.sh")
        };
        let script = sub.join(name);
        std::fs::write(&script, body).expect("写脚本");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perm = std::fs::metadata(&script).expect("读权限位").permissions();
            perm.set_mode(0o755);
            std::fs::set_permissions(&script, perm).expect("加执行位");
        }
        let tools = real_runner(exe, &dir, 60);
        let out = tools.run(&spec_for(&dir), command, "{}", &[], None, None, None);
        assert!(out.ok, "带正斜杠的相对程序名必须能跑起来：{}", out.output);
        assert!(
            out.output.contains("PROBE-OK"),
            "工具输出要如实回来：{}",
            out.output
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 超时：连根杀掉整棵树并如实回执失败（不静默、不无限等它自然结束）。
    #[test]
    fn real_tool_process_is_killed_on_timeout_and_reported() {
        let Some(exe) = built_exe() else {
            eprintln!("[探针] 未找到已构建的 solomni 可执行文件，跳过超时杀树契约");
            return;
        };
        let Some(py) = python() else {
            eprintln!("[探针] 本机没有可用的 python，跳过超时杀树契约");
            return;
        };
        if !crate::kernel::detail::confine::capability().tree {
            eprintln!("[探针] 本机进程树围栏不可用，跳过超时杀树契约");
            return;
        }
        let dir = crate::tests::scratch("proc-tools-timeout");
        std::fs::write(dir.join("sleep60.py"), "import time\ntime.sleep(60)\n").expect("写脚本");
        let tools = real_runner(exe, &dir, 2);
        let started = Instant::now();
        let out = tools.run(
            &spec_for(&dir),
            &format!("{} sleep60.py", py),
            "{}",
            &[],
            None,
            None,
            None,
        );
        assert!(!out.ok, "超时必须如实回执失败：{}", out.output);
        assert!(
            out.output.contains(&tools.texts.timeout()),
            "回执要带上超时标记：{}",
            out.output
        );
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "超时后不得继续等它自然结束（实际 {:?}）",
            started.elapsed()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 开了授权的 runner：只有开了才会走到"必要落点授不上"这条路（关着是用户自己选的档位）。
    /// 授权是把本机目录 ACL 写给容器身份（真机探针才用，所以它在 `--fence-live` 之外没人调）。
    #[cfg(windows)]
    fn real_runner_with_write(exe: PathBuf, home: &std::path::Path) -> ProcTools {
        let mut t = ProcTools::new(
            exe,
            code_texts(),
            home.to_path_buf(),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
        );
        t.timeout = Duration::from_secs(60);
        t
    }

    /// 是否允许跑"会改本机状态"的真机探针（默认否：测试不该在真机上留下痕迹）。
    #[cfg(windows)]
    fn fence_live() -> bool {
        std::env::var("SOLOMNI_FENCE_LIVE")
            .map(|v| v == "1")
            .unwrap_or(false)
    }

    /// 探针用的提问端口替身：记下每次问到的选项 id，按脚本作答（`None` = 拒绝 / 没人答）。
    #[cfg(windows)]
    struct RecordingAsk {
        answer: Option<String>,
        asked: std::sync::Mutex<Vec<Vec<String>>>,
        halted: std::sync::Mutex<Vec<String>>,
    }

    #[cfg(windows)]
    impl RecordingAsk {
        fn new(answer: Option<&str>) -> RecordingAsk {
            RecordingAsk {
                answer: answer.map(|s| s.to_string()),
                asked: std::sync::Mutex::new(Vec::new()),
                halted: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn asked_ids(&self) -> Vec<Vec<String>> {
            self.asked.lock().unwrap_or_else(|e| e.into_inner()).clone()
        }
    }

    #[cfg(windows)]
    impl crate::kernel::ports::AskUser for RecordingAsk {
        fn ask(&self, ask: &crate::kernel::api::Ask) -> crate::kernel::api::AskOutcome {
            use crate::kernel::api::AskOutcome;
            self.asked
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(ask.options.iter().map(|(id, _)| id.clone()).collect());
            match &self.answer {
                Some(id) => AskOutcome::Chosen(id.clone()),
                // 替身按脚本作答；脚本没写 = 没人答（发起方按声明 / fail-closed 收场）。
                None => AskOutcome::NoAnswer,
            }
        }

        fn halt(&self, why: &str) {
            self.halted
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(why.to_string());
        }
    }

    /// **真机探针（T4）**：**必要落点授不上时绝不静默无围栏执行**。
    /// 本机构造这一态：解释器装在属主不是当前用户的目录里（系统级安装，当前用户对那个目录没有写 DACL 的权限）——
    /// 那里的 DACL 写不进（错误码 5），于是**任何**这种解释器的工具都曾静默变成无围栏跑。
    /// 断言三条：选了"本轮无围栏跑一次"才跑（且回执如实标为无围栏）；选"放弃"不执行；没有可回答的前端也不执行。
    /// 本机构造不出这一态（解释器目录授得进）就如实 env-skip——不静默当作通过。
    #[cfg(windows)]
    #[test]
    fn unwritable_interpreter_dir_never_silently_runs_unfenced() {
        use crate::capabilities::tools::domain::fence::{OPT_FENCE_ABORT, OPT_FENCE_UNFENCED};
        use crate::capabilities::tools::service::fence::ToolFenceGate;
        use crate::kernel::api::FencePart;
        if !fence_live() {
            eprintln!(
                "[探针] 未开启真机围栏测试：本探针要写本机权限项，已跳过；要真跑加 --fence-live"
            );
            return;
        }
        let Some(node) = node() else {
            eprintln!("[探针] 本机没有可用的 node：这条探针构造不出解释器目录授不上，跳过");
            return;
        };
        let Some(exe) = built_exe() else {
            eprintln!("[探针] 未找到已构建的 solomni 可执行文件（先 cargo build），跳过真机探针");
            return;
        };
        let dir = crate::tests::scratch("proc-tools-fence-blocked");
        let home = dir.join(".home");
        std::fs::write(dir.join("probe.js"), "process.stdout.write('RAN');").expect("写脚本");
        let spec = spec_for(&dir);
        let command = format!("{} probe.js", node);
        // 先问机制：这一态在本机构造得出来吗（真授不上才继续，能授权就如实跳过）。
        let prep = confine::prepare_fence(&spec, &command, &home);
        let Some(blocked) = prep.blocked.as_ref() else {
            eprintln!(
                "[探针] 本机解释器目录授得进（构造不出\"授不上\"这一态）：真机探针跳过（不静默当作通过）"
            );
            let _ = confine::clean(&home);
            let _ = std::fs::remove_dir_all(&dir);
            return;
        };
        if blocked.part != FencePart::Interpreter {
            eprintln!(
                "[诊断] 本机授不上的不是解释器目录，而是 {}：这条探针仍按同一条规矩断言",
                blocked.line()
            );
        }
        let tools = real_runner_with_write(exe, &home);
        let gate = ToolFenceGate;
        // ① 用户选"本轮无围栏跑一次"：命令真跑，回执**如实标为无围栏**。
        let ask = RecordingAsk::new(Some(OPT_FENCE_UNFENCED));
        let out = tools.run(&spec, &command, "{}", &[], None, Some(&ask), Some(&gate));
        assert_eq!(
            ask.asked_ids(),
            vec![vec![
                OPT_FENCE_UNFENCED.to_string(),
                OPT_FENCE_ABORT.to_string()
            ]],
            "必要落点授不上要**问**用户（选项 id 是契约）"
        );
        assert!(
            !blocked.why.contains("回滚失败"),
            "写入没成功就不许报“回滚失败”（否则用户以为本机权限被改了一半）：{}",
            blocked.why
        );
        assert!(
            ask.halted.lock().expect("锁").is_empty(),
            "有选项就不该停会话"
        );
        assert!(
            // 册子文案带占位符，渲染后才进回执：只断言"如实标为无围栏"这个稳定事实，不复制整段文案。
            out.output.contains("无围栏"),
            "回执要如实标为无围栏：{}",
            out.output
        );
        assert!(
            out.output.contains("RAN"),
            "选了跑一次就要真跑起来：{}",
            out.output
        );
        // ② 用户选"放弃这次调用"：不执行，回执写清哪一环、哪个目录、缺什么、怎么补。
        let ask = RecordingAsk::new(Some(OPT_FENCE_ABORT));
        let out = tools.run(&spec, &command, "{}", &[], None, Some(&ask), Some(&gate));
        assert!(!out.ok, "放弃 = 不执行");
        assert!(
            !out.output.contains("RAN"),
            "放弃之后命令绝不许跑：{}",
            out.output
        );
        for want in [blocked.part.label(), "没有执行", "怎么补"] {
            assert!(
                out.output.contains(want),
                "回执要写清 {}：{}",
                want,
                out.output
            );
        }
        // ③ 没有可回答的前端（纯终端 / e2e）：没有可点的选项 = 不执行（fail-closed）。
        let out = tools.run(&spec, &command, "{}", &[], None, None, Some(&gate));
        assert!(!out.ok, "没有可回答的前端 = 不执行");
        assert!(
            !out.output.contains("RAN"),
            "没有可回答的前端时命令绝不许跑：{}",
            out.output
        );
        assert!(out.output.contains("没有执行"), "{}", out.output);
        // 收尾：撤掉这次写下的授权、删掉台账——测试不在本机留痕。
        if let Err(e) = confine::release_fence(&spec, &home) {
            eprintln!("[诊断] 撤权未完成（{}）：要收尾请跑 --fence-clean", e);
        }
        confine::clean(&home).ok();
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------- 一次性 shell（T3 真实进程） ----------
    #[cfg(unix)]
    fn shell_fence_ready() -> bool {
        let c = confine::capability();
        c.fs && c.net
    }

    #[cfg(unix)]
    #[test]
    fn real_shell_reports_chinese_and_exit_code() {
        let Some(exe) = built_exe() else {
            eprintln!("[探针] 未找到已构建的产品可执行文件，跳过 shell 契约");
            return;
        };
        if !shell_fence_ready() {
            eprintln!("[探针] 本机围栏不可用（环境结论），跳过 shell 契约");
            return;
        }
        let dir = crate::tests::scratch("proc-shell-basic");
        let tools = real_runner(exe, &dir, 30);
        let out = tools.run_shell(&crate::kernel::ports::ShellRun {
            fence: &spec_for(&dir),
            script: "printf '中文\\n'; exit 3",
            timeout: Duration::from_secs(10),
            cancel: None,
            path: None,
            ask: None,
            gate: None,
        });
        assert!(!out.ok, "退出码 3 应如实失败：{}", out.output);
        assert!(out.output.contains("中文"), "{}", out.output);
        assert!(
            out.output.contains("[shell] 退出码 3"),
            "回执要带退出码：{}",
            out.output
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn real_shell_keeps_head_and_tail_when_output_is_long() {
        let Some(exe) = built_exe() else {
            eprintln!("[探针] 未找到已构建的产品可执行文件，跳过 shell 截断契约");
            return;
        };
        if !shell_fence_ready() {
            eprintln!("[探针] 本机围栏不可用（环境结论），跳过 shell 截断契约");
            return;
        }
        let dir = crate::tests::scratch("proc-shell-truncate");
        let tools = real_runner(exe, &dir, 30);
        let out = tools.run_shell(&crate::kernel::ports::ShellRun {
            fence: &spec_for(&dir),
            script: "i=0; while [ $i -lt 20000 ]; do printf a; i=$((i+1)); done; printf TAIL",
            timeout: Duration::from_secs(10),
            cancel: None,
            path: None,
            ask: None,
            gate: None,
        });
        assert!(out.ok, "{}", out.output);
        assert!(out.output.contains("TAIL"), "结尾要保留：{}", out.output);
        assert!(
            out.output.contains("开头") && out.output.contains("结尾"),
            "截断标记要如实说清：{}",
            out.output
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn real_shell_timeout_and_cancel_kill_the_tree() {
        let Some(exe) = built_exe() else {
            eprintln!("[探针] 未找到已构建的产品可执行文件，跳过 shell 超时契约");
            return;
        };
        if !shell_fence_ready() {
            eprintln!("[探针] 本机围栏不可用（环境结论），跳过 shell 超时契约");
            return;
        }
        let dir = crate::tests::scratch("proc-shell-stop");
        let tools = real_runner(exe, &dir, 30);
        let spec = spec_for(&dir);
        let started = Instant::now();
        let out = tools.run_shell(&crate::kernel::ports::ShellRun {
            fence: &spec,
            script: "sleep 5",
            timeout: Duration::from_secs(1),
            cancel: None,
            path: None,
            ask: None,
            gate: None,
        });
        assert!(!out.ok, "{}", out.output);
        assert!(out.output.contains("超时"), "{}", out.output);
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "超时后不得继续等"
        );

        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let set = std::sync::Arc::clone(&flag);
        let spec2 = spec.clone();
        let started = Instant::now();
        let h = std::thread::spawn(move || {
            tools.run_shell(&crate::kernel::ports::ShellRun {
                fence: &spec2,
                script: "sleep 5",
                timeout: Duration::from_secs(30),
                cancel: Some(&set),
                path: None,
                ask: None,
                gate: None,
            })
        });
        std::thread::sleep(Duration::from_millis(300));
        flag.store(true, std::sync::atomic::Ordering::Relaxed);
        let out = h.join().expect("shell 线程");
        assert!(!out.ok, "{}", out.output);
        assert!(out.output.contains("用户停止"), "{}", out.output);
        assert!(started.elapsed() < Duration::from_secs(5), "停止要立刻收树");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn real_shell_fence_denies_outside_paths_and_marks_the_boundary() {
        let Some(exe) = built_exe() else {
            eprintln!("[探针] 未找到已构建的产品可执行文件，跳过 shell 围栏契约");
            return;
        };
        if !shell_fence_ready() {
            eprintln!("[探针] 本机围栏不可用（环境结论），跳过 shell 围栏契约");
            return;
        }
        let inside = crate::tests::scratch("proc-shell-inside");
        let outside = crate::tests::scratch("proc-shell-outside");
        let secret = outside.join("secret.txt");
        std::fs::write(&secret, "SHELL-SECRET").expect("写 secret");
        let mut spec = spec_for(&inside);
        spec.private = inside.clone();
        let tools = real_runner(exe, &inside, 30);
        let out = tools.run_shell(&crate::kernel::ports::ShellRun {
            fence: &spec,
            script: &format!("cat {}", secret.display()),
            timeout: Duration::from_secs(10),
            cancel: None,
            path: None,
            ask: None,
            gate: None,
        });
        assert!(!out.ok, "{}", out.output);
        assert!(!out.output.contains("SHELL-SECRET"), "{}", out.output);
        assert!(
            out.output.contains(crate::kernel::api::FENCE_BOUNDARY_MARK),
            "{}",
            out.output
        );
        let _ = std::fs::remove_dir_all(&inside);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// 目的：生成的 .ps1 让**未处理的 PowerShell 错误**以非 0 收场；显式容错与原生退出码不变。
    /// 约束：这条契约只测脚本包装本身，不装容器围栏（CI 无 ACL 也能跑）；缺 PowerShell 就如实跳过。
    #[cfg(windows)]
    #[test]
    fn generated_shell_script_fails_on_unhandled_powershell_errors() {
        let dir = crate::tests::scratch("proc-shell-script-stop");
        let tools = ProcTools::new(
            PathBuf::from("not-used"),
            code_texts(),
            dir.clone(),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        let program = tools.powershell_program();
        if crate::kernel::detail::host_probe::find_exe(program).is_none() {
            eprintln!("[探针] 本机没有 {}：跳过 PowerShell 脚本契约", program);
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let spec = spec_for(&dir);
        let cases = [
            (
                "Get-Content C:\\definitely-missing-solomni-xyz",
                Some(1),
                "未处理的 cmdlet 错误要以 1 中止",
            ),
            ("Write-Error \"boom\"", Some(1), "Write-Error 要以 1 中止"),
            (
                "Get-Content C:\\definitely-missing-solomni-xyz -ErrorAction SilentlyContinue; Write-Output ok",
                Some(0),
                "显式静默的错误不算失败",
            ),
            ("cmd /c exit 3", Some(3), "原生退出码照旧转发"),
        ];
        for (script, want, why) in cases {
            let (_, path) = tools
                .shell_script_file(&crate::kernel::ports::ShellRun {
                    fence: &spec,
                    script,
                    timeout: Duration::from_secs(10),
                    cancel: None,
                    path: None,
                    ask: None,
                    gate: None,
                })
                .expect("生成 shell 脚本");
            let out = std::process::Command::new(program)
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                ])
                .arg(&path)
                .output()
                .expect("跑 PowerShell");
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert_eq!(
                out.status.code(),
                want,
                "{}（{}，script = {}）",
                why,
                stderr,
                script
            );
            if want == Some(1) {
                assert!(!stderr.trim().is_empty(), "错误要留在 stderr：{}", script);
            }
            let _ = std::fs::remove_file(&path);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn real_shell_uses_powershell_utf8_exit_code_and_cleans_its_script() {
        if !fence_live() {
            eprintln!("[探针] 未开启真机围栏测试（会写本机 ACL）：跳过 Windows shell 契约");
            return;
        }
        let Some(exe) = built_exe() else {
            eprintln!("[探针] 未找到已构建的产品可执行文件，跳过 Windows shell 契约");
            return;
        };
        for program in ["pwsh", "powershell"] {
            if crate::kernel::detail::host_probe::find_exe(program).is_none() {
                eprintln!("[探针] 本机没有 {}：跳过它的 PowerShell 契约", program);
                continue;
            }
            let dir = crate::tests::scratch(&format!("proc-shell-{}", program));
            let home = dir.join(".home");
            let tools = real_runner_with_write(exe.clone(), &home).with_powershell(Some(program));
            let mut spec = spec_for(&dir);
            spec.private = dir.clone();
            let out = tools.run_shell(&crate::kernel::ports::ShellRun {
                fence: &spec,
                script: "Write-Output '中文'; exit 3",
                timeout: Duration::from_secs(30),
                cancel: None,
                path: None,
                ask: None,
                gate: None,
            });
            assert!(!out.ok, "{}（{}）", out.output, program);
            assert!(out.output.contains("中文"), "{}（{}）", out.output, program);
            assert!(
                out.output.contains("[shell] 退出码 3"),
                "{}（{}）",
                out.output,
                program
            );
            // 未处理 cmdlet 错误也要非 0。用 Write-Error：它只依赖内核自带 Utility；
            // 容器里的 5.1 找不到 Management（Get-Content 会变 CommandNotFound），不适合验这条契约。
            let out = tools.run_shell(&crate::kernel::ports::ShellRun {
                fence: &spec,
                script: "Write-Error 'boom'",
                timeout: Duration::from_secs(30),
                cancel: None,
                path: None,
                ask: None,
                gate: None,
            });
            assert!(!out.ok, "未处理错误要失败（{}）：{}", program, out.output);
            assert!(
                out.output.contains("[shell] 退出码 1"),
                "{}（{}）",
                out.output,
                program
            );
            let leftovers = std::fs::read_dir(&dir)
                .expect("读 shell 目录")
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with(".solomni-shell-")
                })
                .count();
            assert_eq!(leftovers, 0, "临时 .ps1 要清掉");
            let _ = confine::release_fence(&spec, &home);
            confine::clean(&home).ok();
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// 目的：Windows 真机上相对路径必须落在私有沙箱 cwd（shell 的 cwd 契约）。
    #[cfg(windows)]
    #[test]
    fn real_windows_shell_writes_relative_files_in_its_private_cwd() {
        if !fence_live() {
            eprintln!("[探针] 未开启真机围栏测试（会写本机 ACL）：跳过 Windows shell cwd 契约");
            return;
        }
        let Some(exe) = built_exe() else {
            eprintln!("[探针] 未找到已构建的产品可执行文件，跳过 Windows shell cwd 契约");
            return;
        };
        let dir = crate::tests::scratch("proc-shell-cwd");
        let home = dir.join(".home");
        let tools = real_runner_with_write(exe, &home);
        let mut spec = spec_for(&dir);
        spec.private = dir.clone();
        let out = tools.run_shell(&crate::kernel::ports::ShellRun {
            fence: &spec,
            script: "Set-Content -Path shell-ok.txt -Value 'CWD-OK' -Encoding utf8; Write-Output (Get-Location)",
            timeout: Duration::from_secs(30),
            cancel: None,
            path: None,
            ask: None,
            gate: None,
        });
        let file = dir.join("shell-ok.txt");
        assert!(
            file.is_file(),
            "shell 的相对路径要落在私有沙箱：{}\n{}",
            file.display(),
            out.output
        );
        assert!(
            std::fs::read_to_string(&file)
                .unwrap_or_default()
                .contains("CWD-OK"),
            "文件内容要写进去"
        );
        assert!(
            out.output.contains(&dir.display().to_string()),
            "Get-Location 要是私有沙箱：{}\n{}",
            dir.display(),
            out.output
        );
        let _ = confine::release_fence(&spec, &home);
        confine::clean(&home).ok();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 守门进程起不来：如实回执「启动失败」，不 panic、不假装跑过。
    #[test]
    fn missing_launcher_binary_is_reported_honestly() {
        let dir = crate::tests::scratch("proc-tools-missing-exe");
        let tools = real_runner(PathBuf::from("definitely-not-here-solomni"), &dir, 5);
        let out = tools.run(&spec_for(&dir), "echo hi", "{}", &[], None, None, None);
        assert!(!out.ok);
        assert!(out.output.contains("工具进程启动失败"), "{}", out.output);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
