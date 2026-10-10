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
}

impl ProcessSessions {
    /// 目的：组合根注入当前可执行文件（守门进程就是它自己）。
    pub fn new(exe: PathBuf) -> ProcessSessions {
        ProcessSessions { exe }
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
        // 守门进程：围栏在它里面装（平台机制见 confine）；这一趟不预先授本机权限（容器那一步随写授权接入）。
        let job = crate::kernel::detail::confine::FenceJob {
            spec: spec.fence.clone(),
            prepared: false,
            home: None,
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
