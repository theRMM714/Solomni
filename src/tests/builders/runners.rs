//! 目的：进程 runner 替身——记录命令与参数，按脚本回输出或失败。
//! 管：静默、记录、环境记录、shell、并发与提问型 runner。
//! 不管：通道（`chat.rs`）、执行入口（`exec.rs`）。
//! 联动：`src/kernel/ports.rs` 的 `ProcessRunner`。

use super::*;
use super::super::prelude::*;

/// 目的：守护 runner：任何调用即失败（守护不该用工具的路径）。
pub(crate) struct SilentRunner;

impl ProcessRunner for SilentRunner {
    fn run(
        &self,
        _fence: &crate::kernel::api::FenceSpec,
        _command: &str,
        _args: &str,
        _env: &[(String, String)],
        _path: Option<&std::ffi::OsStr>,
        _ask: Option<&dyn crate::kernel::ports::AskUser>,
        _gate: Option<&dyn crate::kernel::ports::FenceGate>,
    ) -> RunOutcome {
        panic!("不应调用工具");
    }

    fn run_shell(&self, _run: &crate::kernel::ports::ShellRun<'_>) -> RunOutcome {
        panic!("不应调用 shell");
    }
}

/// 目的：记录型 runner：记录 (root, command, args)，回放固定输出。
pub(crate) struct RecordingRunner {
    pub(crate) calls: Mutex<Vec<(PathBuf, String, String)>>,
    pub(crate) out: String,
    pub(crate) ok: bool,
}

impl RecordingRunner {
    pub(crate) fn new(out: &str, ok: bool) -> RecordingRunner {
        RecordingRunner {
            calls: Mutex::new(Vec::new()),
            out: out.to_string(),
            ok,
        }
    }
}

/// 目的：记下每次调用注入项的 runner（断言隐私字段只经环境变量进工具进程）。
pub(crate) struct EnvRecordingRunner {
    pub(crate) envs: Mutex<Vec<Vec<(String, String)>>>,
    pub(crate) paths: Mutex<Vec<Option<std::ffi::OsString>>>,
}

impl EnvRecordingRunner {
    pub(crate) fn new() -> EnvRecordingRunner {
        EnvRecordingRunner {
            envs: Mutex::new(Vec::new()),
            paths: Mutex::new(Vec::new()),
        }
    }
}

impl ProcessRunner for EnvRecordingRunner {
    fn run(
        &self,
        _fence: &crate::kernel::api::FenceSpec,
        _command: &str,
        args_json: &str,
        env: &[(String, String)],
        path: Option<&std::ffi::OsStr>,
        _ask: Option<&dyn crate::kernel::ports::AskUser>,
        _gate: Option<&dyn crate::kernel::ports::FenceGate>,
    ) -> RunOutcome {
        self.envs.lock().expect("锁").push(env.to_vec());
        self.paths
            .lock()
            .expect("锁")
            .push(path.map(|p| p.to_os_string()));
        RunOutcome {
            ok: true,
            output: format!("跑完了 {}", args_json),
        }
    }

    fn run_shell(&self, run: &crate::kernel::ports::ShellRun<'_>) -> RunOutcome {
        self.paths
            .lock()
            .expect("锁")
            .push(run.path.map(|p| p.to_os_string()));
        RunOutcome {
            ok: true,
            output: format!("shell {}", run.script),
        }
    }
}

impl ProcessRunner for RecordingRunner {
    fn run(
        &self,
        fence: &crate::kernel::api::FenceSpec,
        command: &str,
        args_json: &str,
        _env: &[(String, String)],
        _path: Option<&std::ffi::OsStr>,
        _ask: Option<&dyn crate::kernel::ports::AskUser>,
        _gate: Option<&dyn crate::kernel::ports::FenceGate>,
    ) -> RunOutcome {
        // 记下工具进程的工作目录（= 该模块的根）与命令、参数。
        self.calls.lock().expect("锁").push((
            fence.cwd.clone(),
            command.to_string(),
            args_json.to_string(),
        ));
        RunOutcome {
            ok: self.ok,
            output: self.out.clone(),
        }
    }

    fn run_shell(&self, run: &crate::kernel::ports::ShellRun<'_>) -> RunOutcome {
        // shell 也记进同一份 calls：cwd / "shell" / 脚本；超时与围栏看专门的 ShellRunner。
        self.calls.lock().expect("锁").push((
            run.fence.cwd.clone(),
            "shell".to_string(),
            run.script.to_string(),
        ));
        RunOutcome {
            ok: self.ok,
            output: self.out.clone(),
        }
    }
}

/// 目的：一次 shell 调用的事实记录（脚本、围栏、超时、取消旗、是否按次临时 profile、PATH）。
pub(crate) struct ShellRecord {
    /// 目的：这次调用的围栏。
    pub(crate) fence: crate::kernel::api::FenceSpec,
    /// 目的：脚本原文。
    pub(crate) script: String,
    /// 目的：按次超时。
    pub(crate) timeout: std::time::Duration,
    /// 目的：调用时取消旗是否已置位。
    pub(crate) cancel_set: bool,
    /// 目的：是否按次临时容器 profile。
    /// 目的：这一趟的 PATH。
    pub(crate) path: Option<std::ffi::OsString>,
}

/// 目的：记录型 shell runner——每次 `run_shell` 记下完整事实，按脚本回放固定输出。
pub(crate) struct ShellRunner {
    /// 目的：调用记录。
    pub(crate) calls: Mutex<Vec<ShellRecord>>,
    /// 目的：回放输出。
    pub(crate) out: String,
    /// 目的：回放成败。
    pub(crate) ok: bool,
}

impl ShellRunner {
    /// 目的：按给定输出与成败造一个 shell 替身。
    pub(crate) fn new(out: &str, ok: bool) -> ShellRunner {
        ShellRunner {
            calls: Mutex::new(Vec::new()),
            out: out.to_string(),
            ok,
        }
    }
}

impl ProcessRunner for ShellRunner {
    fn run(
        &self,
        _fence: &crate::kernel::api::FenceSpec,
        _command: &str,
        _args: &str,
        _env: &[(String, String)],
        _path: Option<&std::ffi::OsStr>,
        _ask: Option<&dyn crate::kernel::ports::AskUser>,
        _gate: Option<&dyn crate::kernel::ports::FenceGate>,
    ) -> RunOutcome {
        panic!("这个替身只该跑 shell");
    }

    fn run_shell(&self, run: &crate::kernel::ports::ShellRun<'_>) -> RunOutcome {
        self.calls.lock().expect("锁").push(ShellRecord {
            fence: run.fence.clone(),
            script: run.script.to_string(),
            timeout: run.timeout,
            cancel_set: run
                .cancel
                .map(|c| c.load(std::sync::atomic::Ordering::Relaxed))
                .unwrap_or(false),
            path: run.path.map(|p| p.to_os_string()),
        });
        RunOutcome {
            ok: self.ok,
            output: self.out.clone(),
        }
    }
}

/// 目的：并发记录型 runner：记录**同时在跑**的调用数峰值（模块工具是否真的并发，只有它能作证）。
pub(crate) struct ParallelRunner {
    pub(crate) active: AtomicUsize,
    pub(crate) peak: AtomicUsize,
    pub(crate) delay_ms: u64,
}

impl ParallelRunner {
    pub(crate) fn new(delay_ms: u64) -> ParallelRunner {
        ParallelRunner {
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            delay_ms,
        }
    }

    /// 目的：同时在跑的峰值（串行恒为 1）。
    pub(crate) fn peak_concurrent(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }
}

impl ProcessRunner for ParallelRunner {
    fn run(
        &self,
        _fence: &crate::kernel::api::FenceSpec,
        command: &str,
        args_json: &str,
        _env: &[(String, String)],
        _path: Option<&std::ffi::OsStr>,
        _ask: Option<&dyn crate::kernel::ports::AskUser>,
        _gate: Option<&dyn crate::kernel::ports::FenceGate>,
    ) -> RunOutcome {
        let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        if self.delay_ms > 0 {
            std::thread::sleep(Duration::from_millis(self.delay_ms));
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
        RunOutcome {
            ok: true,
            output: format!("{} 跑完了 {}", command, args_json),
        }
    }

    fn run_shell(&self, run: &crate::kernel::ports::ShellRun<'_>) -> RunOutcome {
        RunOutcome {
            ok: true,
            output: format!("shell 跑完了 {}", run.script),
        }
    }
}

/// 目的：提问型 runner——跑之前先经**工具层提问端口**问一次（模拟围栏那类"这一环装不上就先问"的执行层）。
/// 参数：`allow` = 只有收到这个选项 id 才真跑（别的选项、没有回答、没有端口一律不跑）。
pub(crate) struct AskingRunner {
    pub(crate) asked: Mutex<Vec<Vec<String>>>,
    pub(crate) calls: Mutex<Vec<(PathBuf, String, String)>>,
    /// 目的：只有收到这个选项 id 才真跑（别的选项、没有回答、没有端口一律不跑）。
    pub(crate) allow: String,
}

impl AskingRunner {
    pub(crate) fn new(allow: &str) -> AskingRunner {
        AskingRunner {
            asked: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
            allow: allow.to_string(),
        }
    }
}

impl ProcessRunner for AskingRunner {
    fn run(
        &self,
        fence: &crate::kernel::api::FenceSpec,
        command: &str,
        args_json: &str,
        _env: &[(String, String)],
        _path: Option<&std::ffi::OsStr>,
        ask: Option<&dyn crate::kernel::ports::AskUser>,
        _gate: Option<&dyn crate::kernel::ports::FenceGate>,
    ) -> RunOutcome {
        let Some(ask) = ask else {
            // 没有可回答的前端：没有可点的选项 = 不执行（fail-closed）。
            return RunOutcome {
                ok: false,
                output: "这一趟没有可回答的前端：这次调用没有执行".to_string(),
            };
        };
        let request = crate::kernel::api::Ask {
            role: "tools".to_string(),
            name: fence.agent.clone(),
            title: "围栏的这一环装不上，这次调用怎么跑？".to_string(),
            body: "按规则不许悄悄按无围栏执行。".to_string(),
            detail: "模块目录（C:/mods/a）授不上：写 DACL 失败（错误码 5）".to_string(),
            options: vec![
                (
                    crate::capabilities::tools::domain::fence::OPT_FENCE_UNFENCED.to_string(),
                    "本轮无围栏跑一次".to_string(),
                ),
                (
                    crate::capabilities::tools::domain::fence::OPT_FENCE_ABORT.to_string(),
                    "放弃这次调用".to_string(),
                ),
            ],
            on_unanswered: None,
        };
        self.asked
            .lock()
            .expect("锁")
            .push(request.options.iter().map(|(id, _)| id.clone()).collect());
        if ask.ask(&request).decided() != Some(self.allow.as_str()) {
            return RunOutcome {
                ok: false,
                output: "用户没有放行这次调用：它没有执行".to_string(),
            };
        }
        self.calls.lock().expect("锁").push((
            fence.cwd.clone(),
            command.to_string(),
            args_json.to_string(),
        ));
        RunOutcome {
            ok: true,
            output: "按你的裁决无围栏跑了一次".to_string(),
        }
    }

    fn run_shell(&self, _run: &crate::kernel::ports::ShellRun<'_>) -> RunOutcome {
        RunOutcome {
            ok: true,
            output: "shell 按你的裁决跑了一次".to_string(),
        }
    }
}
