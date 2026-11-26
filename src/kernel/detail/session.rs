//! 目的：**长驻外部进程**的机制实现——拉起守门进程并保住它，按行收发，关闭时连根杀树。
//! 管：`ProcessSessions`（实现 `SessionHost`）：守门进程协议、管道、进程组与父死子亡、进程账的登记与注销、环境白名单与隐私字段注入。
//! 不管：端口定义（在 `ports`）；按行协议的语义（MCP / ACP 在各自适配器里）。
//! 联动：由入口层的组合根构造并注入适配器；一次性执行仍走 `process.rs` 的 `ProcTools`；父死子亡与进程组走 `procs.rs`。

use crate::kernel::detail::procs;
use crate::kernel::ports::{Session, SessionHost, SessionSpec};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{ChildStdin, ChildStdout};

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
    /// 目的：平台侧状态——Windows 持有产品安装根与写权限开关；其余平台为空。
    platform: platform::State,
    /// 目的：全局沙箱要求与本进程是否已同意降级。
    mode: std::sync::Arc<crate::kernel::domain::fence::SandboxMode>,
}

impl ProcessSessions {
    /// 目的：组合根注入：当前可执行文件（守门进程就是它自己）、产品安装根、写权限开关与进程账。
    /// 约束：与一次执行（`ProcTools::new`）同一份装配事实；安装根不是台账目录（台账由内核机制自己派生）；
    ///   非 Windows 平台忽略安装根 / 写权限（显式忽略以免被误读成漏用）；
    ///   账必须与组合根用**同一个**实例（账本是整份文件写回，两份会互相覆盖）。
    pub fn new(
        exe: PathBuf,
        root: PathBuf,
        write_allowed: std::sync::Arc<std::sync::atomic::AtomicBool>,
        book: std::sync::Arc<procs::ProcessBook>,
    ) -> ProcessSessions {
        ProcessSessions {
            exe,
            book,
            platform: platform::State::new(root, write_allowed),
            mode: std::sync::Arc::new(crate::kernel::domain::fence::SandboxMode::permissive()),
        }
    }

    /// 目的：组合根注入全局沙箱模式；缺省是“无要求”的测试替身。
    pub fn with_mode(
        mut self,
        mode: std::sync::Arc<crate::kernel::domain::fence::SandboxMode>,
    ) -> Self {
        self.mode = mode;
        self
    }
}

/// 一个跑着的守门进程会话。
struct FencedSession {
    /// 目的：长驻子进程的句柄——收场、判退出与销账都经它（丢掉句柄就是收掉）。
    daemon: procs::daemon::Daemon,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    /// 见过 EOF 或读错误 = 这个会话已经不可用了（崩溃检测的判据）。
    eof: bool,
}

/// 目的：守门进程在账上的标记——对账时在命令行里找这一段来确认"还是我们那一个"。
/// 约束：agent 名出现在 FenceJob 的 JSON 里；没有 agent（不经过常驻服务的调用方）就退回它一定是的那一段。
fn resident_marker(agent: &str) -> String {
    if agent.is_empty() {
        "--fence-run".to_string()
    } else {
        agent.to_string()
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

    /// 目的：收掉守门进程连同它拉起的服务（幂等；句柄丢掉时同样收）。
    fn kill(&mut self) {
        self.daemon.stop();
    }
}

impl SessionHost for ProcessSessions {
    fn open(&self, spec: &SessionSpec) -> Result<Box<dyn Session>, String> {
        // 平台后端决定要不要先准备围栏：Windows 的容器围栏要先把可达范围授权给容器 SID；
        // 其余平台没有这一步，直接返回「未准备、无家目录」。
        let (prepared, root) = platform::prepare(&self.platform, spec)?;
        // 全局沙箱要求：机制不可用且本进程未同意降级时，常驻服务不拉起。
        self.mode
            .check(&spec.fence, prepared)
            .map_err(|why| format!("服务起不来（沙箱要求未满足）：{}", why))?;
        let job = crate::kernel::detail::confine::FenceJob {
            plan: spec.fence.clone(),
            prepared,
            product_home: root,
            policy: crate::kernel::contracts::fence::FenceRunPolicy {
                require_fs: self.mode.requirements.filesystem,
                require_net: self.mode.requirements.network,
                allow_degraded: self.mode.degraded(),
            },
        };
        let launcher = crate::kernel::detail::confine::launcher(&self.exe, &job, &spec.command);
        // 环境从零开始拼：继承来的变量（含凭据）不跟着守门进程走，只给围栏与调用方点名的那几个。
        let mut spawn = procs::spawn::Spawn::new(launcher.get_program())
            .args(launcher.get_args())
            .cwd(&spec.fence.cwd)
            .clear_env()
            .stdin(procs::spawn::Stream::Piped)
            .stdout(procs::spawn::Stream::Piped)
            // 围栏自己的说明（如降级提示）直接进父进程 stderr，与一次执行的落脚一致。
            .stderr(procs::spawn::Stream::Inherit)
            .purpose("resident")
            .owner(spec.fence.agent.as_str())
            .marker(resident_marker(&spec.fence.agent))
            .book(Some(std::sync::Arc::clone(&self.book)));
        for (k, v) in crate::kernel::detail::confine::fence_env(&spec.fence, &spec.command) {
            spawn = spawn.env(k, v);
        }
        if let Some(p) = &spec.path {
            spawn = spawn.env("PATH", p);
        }
        for (k, v) in &spec.env {
            spawn = spawn.env(k, v);
        }
        // 起进程这一步已经把父死子亡、独立进程组、长命线程与登记入账一起办好了。
        let mut daemon = procs::spawn::spawn(spawn)
            .map_err(|e| format!("服务进程启动失败：{}", e))?;
        // 取不到管道就没有会话可用：句柄一丢就收掉守门进程与账上那条，不留半步。
        let (Some(stdin), Some(stdout)) = (daemon.take_stdin(), daemon.take_stdout()) else {
            return Err("无法取得服务进程的 stdin / stdout".to_string());
        };
        Ok(Box::new(FencedSession {
            daemon,
            stdin,
            stdout: BufReader::new(stdout),
            eof: false,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 目的：守门进程的标记要在它自己的命令行里找得到（对账靠它确认身份，认不出就收不掉孤儿）。
    #[test]
    fn a_resident_marker_is_findable_in_its_command_line() {
        assert_eq!(
            resident_marker("resident/m1/echo"),
            "resident/m1/echo",
            "agent 名出现在 FenceJob 的 JSON 里，账上用同一段做标记"
        );
    }

    /// 目的：不经过常驻服务的调用方没有 agent 名时，标记退回守门进程一定有的那一段，免得对账认不出这个 pid。
    #[test]
    fn a_resident_marker_without_an_agent_still_exists() {
        assert_eq!(resident_marker(""), "--fence-run");
    }
}
