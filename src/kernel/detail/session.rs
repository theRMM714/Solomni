//! 目的：**长驻外部进程**的机制实现——拉起守门进程并保住它，按行收发，关闭时连根杀树。
//! 管：`ProcessSessions`（实现 `SessionHost`）：守门进程协议、管道、进程组与父死子亡、进程账的登记与注销、环境白名单与隐私字段注入。
//! 不管：端口定义（在 `ports`）；按行协议的语义（MCP / ACP 在各自适配器里）。
//! 联动：由入口层的组合根构造并注入适配器；一次性执行仍走 `process.rs` 的 `ProcTools`；父死子亡与进程组走 `procs.rs`。

use crate::kernel::detail::procs;
use crate::kernel::ports::{Session, SessionHost, SessionSpec};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::Mutex;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as platform;
#[cfg(not(windows))]
mod unix;
#[cfg(not(windows))]
use unix as platform;

/// 目的：拉起长驻守门进程的实现（守门进程就是本程序自己的 --fence-run 模式）。
pub struct ProcessSessions {
    exe: PathBuf,
    /// 目的：长驻子进程的账——守门进程起时登记、收时注销，核心被强杀留下的孤儿由启动对账按它回收。
    book: std::sync::Arc<procs::ProcessBook>,
    /// 目的：平台侧状态——Windows 持有文案、产品私有区与写权限开关；其余平台为空。
    platform: platform::State,
    /// 目的：全局沙箱要求与本进程是否已同意降级。
    mode: std::sync::Arc<crate::kernel::api::SandboxMode>,
}

impl ProcessSessions {
    /// 目的：组合根注入：当前可执行文件（守门进程就是它自己）、提示词册文案、产品私有区、写权限开关与进程账。
    /// 约束：与一次执行（`ProcTools::new`）同一份装配事实；非 Windows 平台忽略文案 / 私有区 / 写权限（显式忽略以免被误读成漏用）；
    ///   账必须与组合根用**同一个**实例（账本是整份文件写回，两份会互相覆盖）。
    pub fn new(
        exe: PathBuf,
        texts: std::sync::Arc<dyn crate::kernel::ports::ProcessTexts>,
        home: PathBuf,
        write_allowed: std::sync::Arc<std::sync::atomic::AtomicBool>,
        book: std::sync::Arc<procs::ProcessBook>,
    ) -> ProcessSessions {
        ProcessSessions {
            exe,
            book,
            platform: platform::State::new(texts, home, write_allowed),
            mode: std::sync::Arc::new(crate::kernel::api::SandboxMode::permissive()),
        }
    }

    /// 目的：组合根注入全局沙箱模式；缺省是“无要求”的测试替身。
    pub fn with_mode(mut self, mode: std::sync::Arc<crate::kernel::api::SandboxMode>) -> Self {
        self.mode = mode;
        self
    }
}

/// 一个跑着的守门进程会话。
struct FencedSession {
    child: Mutex<Child>,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    /// 见过 EOF 或读错误 = 这个会话已经不可用了（崩溃检测的判据）。
    eof: bool,
    /// 目的：账上那条的 pid——收掉时按它注销。
    pid: u32,
    /// 目的：同一份进程账（收掉时注销本会话那条）。
    book: std::sync::Arc<procs::ProcessBook>,
}

/// 目的：常驻服务守门进程在账上的一条——身份与父死子亡的结论来自起进程那一刻的事实。
fn resident_entry(agent: &str, pid: u32, since: u64, launched: procs::Launched) -> procs::Entry {
    // 标记要在守门进程的命令行里找得到：agent 名出现在 FenceJob 的 JSON 里；没有 agent（不经过常驻服务的调用方）
    // 就退回它一定是的那一段。
    let marker = if agent.is_empty() {
        "--fence-run".to_string()
    } else {
        agent.to_string()
    };
    procs::Entry {
        pid,
        since,
        purpose: "resident".to_string(),
        owner: agent.to_string(),
        marker,
        created: launched.created,
        guard_note: launched.guard_note,
    }
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
        let n = match self.stdout.read_line(&mut buf) {
            Ok(n) => n,
            Err(e) => {
                self.eof = true;
                return Err(format!("读服务进程失败：{}", e));
            }
        };
        if n == 0 {
            self.eof = true;
            return Err("服务进程已结束（EOF）".to_string());
        }
        Ok(buf.trim_end_matches(['\r', '\n']).to_string())
    }

    fn alive(&self) -> bool {
        !self.eof
    }

    fn kill(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            crate::kernel::detail::process::kill_tree(&mut child);
        }
        self.book.forget(self.pid);
    }
}

impl Drop for FencedSession {
    /// 目的：会话被丢掉（正常关闭，或握手失败后丢了句柄）时同样连根收掉并注销——不留孤儿、不留死条目。
    fn drop(&mut self) {
        self.kill();
    }
}

impl SessionHost for ProcessSessions {
    fn open(&self, spec: &SessionSpec) -> Result<Box<dyn Session>, String> {
        // 平台后端决定要不要先准备围栏：Windows 的容器围栏要先把可达范围授权给容器 SID；
        // 其余平台没有这一步，直接返回「未准备、无家目录」。
        let (prepared, home) = platform::prepare(&self.platform, spec)?;
        // 全局沙箱要求：机制不可用且本进程未同意降级时，常驻服务不拉起。
        self.mode
            .check(&spec.fence, prepared)
            .map_err(|why| format!("服务起不来（沙箱要求未满足）：{}", why))?;
        let job = crate::kernel::detail::confine::FenceJob {
            spec: spec.fence.clone(),
            prepared,
            home,
            policy: crate::kernel::api::FenceRunPolicy {
                require_fs: self.mode.requirements.filesystem,
                require_net: self.mode.requirements.network,
                allow_degraded: self.mode.degraded(),
            },
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
        if let Some(p) = &spec.path {
            cmd.env("PATH", p);
        }
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        // 父死子亡与独立进程组：核心被强杀时，守门进程连同它拉起的服务不该留下。
        procs::guard(&mut cmd);
        let mut child = procs::spawn(cmd).map_err(|e| format!("服务进程启动失败：{}", e))?;
        // 作业对象（Windows）/ 守门进程（macOS）：核心一死，它与服务一起收；挂不上只如实报一条并记进账。
        let launched = procs::attach(&child, &spec.fence.agent);
        let pid = child.id();
        self.book.record(resident_entry(
            &spec.fence.agent,
            pid,
            procs::now_ts(),
            launched,
        ));
        // 取不到管道就没有会话可用：连刚起的守门进程与账上那条一起收掉，不留半步。
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            crate::kernel::detail::process::kill_tree(&mut child);
            self.book.forget(pid);
            return Err("无法取得服务进程的 stdin / stdout".to_string());
        };
        Ok(Box::new(FencedSession {
            child: Mutex::new(child),
            stdin,
            stdout: BufReader::new(stdout),
            eof: false,
            pid,
            book: std::sync::Arc::clone(&self.book),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 目的：常驻服务守门进程在账上的那条——用途 / 归属 / 标记 / 兜底结论都来自起进程那一刻的事实。
    #[test]
    fn a_resident_guard_is_recorded_with_its_service_identity() {
        let e = resident_entry(
            "resident/m1/echo",
            42,
            7,
            procs::Launched {
                created: Some(11),
                guard_note: Some("把子进程绑进作业对象失败".to_string()),
            },
        );
        assert_eq!(e.purpose, "resident");
        assert_eq!(e.owner, "resident/m1/echo");
        assert_eq!(
            e.marker, "resident/m1/echo",
            "标记要在守门进程的命令行里找得到（对账靠它确认身份）"
        );
        assert_eq!(e.created, Some(11));
        assert_eq!(
            e.guard_note.as_deref(),
            Some("把子进程绑进作业对象失败"),
            "没挂上的原因要如实进账"
        );
    }

    /// 目的：不经过常驻服务的调用方没有 agent 名时，标记退回守门进程一定有的那一段，免得对账认不出这个 pid。
    #[test]
    fn a_resident_guard_without_an_agent_still_has_a_marker() {
        let e = resident_entry(
            "",
            43,
            8,
            procs::Launched {
                created: None,
                guard_note: None,
            },
        );
        assert_eq!(e.marker, "--fence-run");
        assert_eq!(e.owner, "");
        assert!(e.guard_note.is_none());
    }
}
