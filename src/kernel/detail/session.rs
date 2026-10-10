//! 目的：**长驻外部进程**的机制实现——拉起守门进程并保住它，按行收发，关闭时连根杀树。
//! 管：`ProcessSessions`（实现 `SessionHost`）：守门进程协议、管道、进程组、环境白名单与隐私字段注入。
//! 不管：端口定义（在 `ports`）；按行协议的语义（MCP / ACP 在各自适配器里）。
//! 联动：由入口层的组合根构造并注入适配器；一次性执行仍走 `process.rs` 的 `ProcTools`。

use crate::kernel::ports::{Session, SessionHost, SessionSpec};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::Mutex;

/// 目的：拉起长驻守门进程的实现（守门进程就是本程序自己的 --fence-run 模式）。
pub struct ProcessSessions {
    exe: PathBuf,
    /// 提示词册里的围栏失败文案（只在 Windows 的拒绝路径用）。
    #[cfg(windows)]
    texts: std::sync::Arc<dyn crate::kernel::ports::ProcessTexts>,
    /// 目的：产品私有区（.home/）——容器 profile 的台账落点（只 Windows 用）。
    #[cfg(windows)]
    home: PathBuf,
    /// 目的：是否允许在本机写权限（只 Windows 用；默认否）。
    #[cfg(windows)]
    write_allowed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 目的：「未授权」的提示只出一次，不刷屏。
    #[cfg(windows)]
    disclosed: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl ProcessSessions {
    /// 目的：组合根注入：当前可执行文件（守门进程就是它自己）、提示词册文案、产品私有区与写权限开关。
    /// 约束：与一次执行（`ProcTools::new`）同一份装配事实；非 Windows 平台忽略后三项（显式忽略以免被误读成漏用）。
    pub fn new(
        exe: PathBuf,
        texts: std::sync::Arc<dyn crate::kernel::ports::ProcessTexts>,
        home: PathBuf,
        write_allowed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> ProcessSessions {
        #[cfg(not(windows))]
        let _ = (texts, home, write_allowed);
        ProcessSessions {
            exe,
            #[cfg(windows)]
            texts,
            #[cfg(windows)]
            home,
            #[cfg(windows)]
            write_allowed,
            #[cfg(windows)]
            disclosed: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

/// 一个跑着的守门进程会话。
struct FencedSession {
    child: Mutex<Child>,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Session for FencedSession {
    fn send(&mut self, line: &str) -> Result<(), String> {
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .map_err(|e| format!("写服务进程失败：{}", e))
    }

    fn recv(&mut self) -> Result<String, String> {
        let mut buf = String::new();
        let n = self
            .stdout
            .read_line(&mut buf)
            .map_err(|e| format!("读服务进程失败：{}", e))?;
        if n == 0 {
            return Err("服务进程已结束（EOF）".to_string());
        }
        Ok(buf.trim_end_matches(['\r', '\n']).to_string())
    }

    fn kill(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            crate::kernel::detail::process::kill_tree(&mut child);
        }
    }
}

impl SessionHost for ProcessSessions {
    fn open(&self, spec: &SessionSpec) -> Result<Box<dyn Session>, String> {
        // Windows：容器围栏要先把可达范围授权给容器 SID（与一次执行同一把尺子）；**默认不写本机权限项**。
        // 起服务这一趟**没有可回答的前端**：必要落点授不上就 fail-closed，把缺哪一环如实报出来（写授权才做容器那一步）。
        #[cfg(windows)]
        let (prepared, home) = {
            use std::sync::atomic::Ordering;
            let mut prepared = false;
            if self.write_allowed.load(Ordering::Relaxed) {
                let prep = crate::kernel::detail::confine::prepare_fence(
                    &spec.fence,
                    &spec.command,
                    &self.home,
                );
                // 可选落点授不上只记事实（不牵动这次拉起）。
                for n in &prep.notes {
                    eprintln!("[围栏] {}（可选落点，本次照常执行）", n);
                }
                if prep.ok() {
                    prepared = true;
                } else if let Some(blocked) = &prep.blocked {
                    return Err(format!(
                        "[围栏] 服务起不来（必要落点授不上）：{}",
                        blocked.line()
                    ));
                }
            } else if !self.disclosed.swap(true, Ordering::Relaxed) {
                eprintln!(
                    "[围栏] 容器围栏未启用（用户没授权在本机写权限）：常驻服务按无围栏执行（要启用：在设置里打开，或写 fence_write: true）"
                );
            }
            if !prepared {
                if let Some(outcome) = crate::kernel::detail::process::refuse_when_broken(
                    &*self.texts,
                    crate::kernel::detail::confine::verify(&spec.fence, &spec.command),
                ) {
                    return Err(outcome.output);
                }
            }
            (prepared, Some(self.home.clone()))
        };
        #[cfg(not(windows))]
        let (prepared, home): (bool, Option<PathBuf>) = (false, None);
        let job = crate::kernel::detail::confine::FenceJob {
            spec: spec.fence.clone(),
            prepared,
            home,
        };
        let mut cmd = crate::kernel::detail::confine::launcher(&self.exe, &job, &spec.command);
        cmd.current_dir(&spec.fence.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // 围栏自己的说明（如降级提示）直接进父进程 stderr，与一次执行的落脚一致。
            .stderr(Stdio::inherit());
        cmd.env_clear();
        for (k, v) in crate::kernel::detail::confine::fence_env(&spec.fence, &spec.command) {
            cmd.env(k, v);
        }
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        // 独立进程组：关闭会话时连根杀掉守门进程与服务进程。
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("服务进程启动失败：{}", e))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "无法取得服务进程 stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "无法取得服务进程 stdout".to_string())?;
        Ok(Box::new(FencedSession {
            child: Mutex::new(child),
            stdin,
            stdout: BufReader::new(stdout),
        }))
    }
}
