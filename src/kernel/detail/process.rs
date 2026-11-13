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

// 围栏准备的耗时经同线程交棒给回执：run_inner 的 Windows 分支写入，调用方读过即清（见 take_fence_prep）。
// 不走 RunOutcome：端口只陈述成功与输出（kernel/contracts/run.rs）；thread_local! 不产文档，注释用 //。
thread_local! {
    static FENCE_PREP: std::cell::Cell<Option<FenceTiming>> = const { std::cell::Cell::new(None) };
}

/// 目的：一次围栏准备的耗时交棒：总耗时 + 真写 ACL 的分步明细（每条授权一条；纯命中时为空）。
/// 约束：分步明细只用于解释「时间花在哪」，不改回执行文——回执仍只读总秒数（`as_secs`）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FenceTiming {
    /// 目的：整段 prepare_fence 的墙钟耗时（回执的「其中围栏准备 N 秒」用它）。
    pub total: Duration,
    /// 目的：真写 ACL 的步骤（解释器目录 / 祖先链 / 受保护根 / 可信基座），纯命中为空。
    pub steps: Vec<confine::FenceStep>,
}

impl FenceTiming {
    /// 目的：回执口径的总秒数（与改动前的 Option<Duration> 同义：回执行文与解析口径不变）。
    pub fn as_secs(&self) -> u64 {
        self.total.as_secs()
    }
}

/// 目的：读走本次 shell 执行的围栏准备耗时（读即清：同一线程的下一趟不会读到这一趟的值）。
/// 约束：只有 Windows 且真的走到围栏准备时才有值；其它平台 / 未授权写权限 / 参数拒绝都恒为 None。
pub fn take_fence_prep() -> Option<FenceTiming> {
    FENCE_PREP.with(|slot| slot.take())
}

/// 目的：每趟执行开始前把交棒位置清空——上一趟没被取走的值不会漏给这一趟。
/// 约束：在 `run_inner` 入口调用（各平台都清，成本是一次线程本地写）；不判断有没有围栏准备。
fn reset_fence_prep() {
    FENCE_PREP.with(|slot| slot.set(None));
}

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
        // 上一趟的交棒只对那一趟有效：新一趟先清空，这趟没走到围栏准备就保持空（读即清见 take_fence_prep）。
        reset_fence_prep();
        // Windows：容器围栏要先把「可达范围」授权给容器 SID。
        // **默认不写本机任何权限项**：只有用户显式授权（设置里的 sandbox.policy.windows.write_acl，或环境变量 SOLOMNI_FENCE_WRITE=1）才做。
        #[cfg(windows)]
        let (prepared, unfenced_note) = {
            use std::sync::atomic::Ordering;
            let mut prepared = false;
            let mut note: Option<String> = None;
            if self.write_allowed.load(Ordering::Relaxed) {
                // 围栏准备单独计时：首调用逐条写 ACL 常常占掉整次耗时的绝大部分，回执要能分辨它（交棒见 take_fence_prep）。
                let fence_started = Instant::now();
                let prep = confine::prepare_fence(fence, command, &self.home);
                let steps = prep.steps.clone();
                FENCE_PREP.with(|slot| {
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
                            return RunOutcome { ok: false, output };
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
        if !plan.strict
            && unfenced_note.is_none()
            && let Err(why) = self.mode.check(fence, prepared)
        {
            return RunOutcome {
                ok: false,
                output: why,
            };
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
                };
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
            if let Some(flag) = plan.cancel
                && flag.load(std::sync::atomic::Ordering::Relaxed)
            {
                cancelled = true;
                kill_tree(&mut child);
                break;
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
                };
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
const PS_PRELUDE: &str = concat!(
    "try { Import-Module Microsoft.PowerShell.Management, Microsoft.PowerShell.Utility -ErrorAction SilentlyContinue } catch { }\n",
    "$ErrorActionPreference = 'Stop'\n",
    "try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch { }\n",
    "$OutputEncoding = [System.Text.Encoding]::UTF8\n",
    // 当前定位必须落在脚本所在目录（即工具进程的工作目录）：切不过去就如实报到 stderr——否则相对路径
    // 会静默落到继承来的别处，看起来只是「写不进去」。
    "try { Set-Location -LiteralPath $PSScriptRoot -ErrorAction Stop }\n",
    "catch {\n",
    "    if ((Get-Location).Path -ne $PSScriptRoot) {\n",
    "        [Console]::Error.WriteLine((\"[shell] 当前定位没能切到工作目录（{0}）：{1}\" -f $PSScriptRoot, $_.Exception.Message))\n",
    "    }\n",
    "}\n",
);
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
mod tests;
