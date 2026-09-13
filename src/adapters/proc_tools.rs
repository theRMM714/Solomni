//! 工具执行适配器：把核心放行的工具命令拉起为子进程（实现 core 的 ToolRunner 端口）。
//! 机制边界：命令行来自 module.yaml（模块作者），JSON 参数走 stdin（不进命令行，杜绝注入）；
//! 截获 stdout/stderr、超时击杀、输出截断。
//! 沙箱与权限分级是后置工作：当前工具进程权限等同运行产品的用户，如实告知于文档。

use crate::core::ports::{ToolOutcome, ToolRunner};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub struct ProcTools {
    /// 单次工具执行的超时（到时击杀进程，ok = false）。
    pub timeout: Duration,
    /// 回传给模型/轨迹的输出上限（字符数）。
    pub max_output_chars: usize,
}

impl Default for ProcTools {
    fn default() -> ProcTools {
        ProcTools { timeout: Duration::from_secs(30), max_output_chars: 16_000 }
    }
}

impl ToolRunner for ProcTools {
    fn run(&self, root: &std::path::Path, command: &str, args_json: &str) -> ToolOutcome {
        let mut cmd = shell_command(command);
        cmd.current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return ToolOutcome { ok: false, output: format!("工具进程启动失败：{}", e) },
        };
        // stdin 独立线程送参：参数再大也不与 stdout 读取互锁。
        let mut stdin = child.stdin.take().expect("stdin 已声明管道");
        let payload = format!("{}\n", args_json);
        let writer = thread::spawn(move || {
            use std::io::Write;
            let _ = stdin.write_all(payload.as_bytes());
            let _ = stdin.flush();
        });
        let mut stdout_pipe = child.stdout.take().expect("stdout 已声明管道");
        let mut stderr_pipe = child.stderr.take().expect("stderr 已声明管道");
        let out_reader = thread::spawn(move || read_to_string(&mut stdout_pipe));
        let err_reader = thread::spawn(move || read_to_string(&mut stderr_pipe));

        // 轮询等待至截止；到时击杀（管道随之关闭，读线程自然结束）。
        let deadline = Instant::now() + self.timeout;
        let mut timed_out = false;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        timed_out = true;
                        let _ = child.kill();
                        let _ = child.wait();
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
        let status_ok = !timed_out && child.try_wait().ok().flatten().map(|s| s.success()).unwrap_or(false);

        let mut output = out;
        if !err.is_empty() {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str("[stderr]\n");
            output.push_str(&err);
        }
        if timed_out {
            output.push_str("\n[超时] 工具进程被击杀");
        }
        let truncated = truncate_chars(&output, self.max_output_chars);
        ToolOutcome { ok: status_ok, output: truncated }
    }
}

/// 命令行交由系统 shell 解释（命令来自模块清单；模型只提供 stdin 数据）。
#[cfg(windows)]
fn shell_command(command: &str) -> Command {
    let mut c = Command::new("cmd");
    c.arg("/C").arg(command);
    c
}

#[cfg(not(windows))]
fn shell_command(command: &str) -> Command {
    let mut c = Command::new("sh");
    c.arg("-c").arg(command);
    c
}

fn read_to_string(r: &mut impl std::io::Read) -> String {
    let mut buf = Vec::new();
    let _ = r.read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

/// 按字符截断（不劈开 UTF-8），尾部如实注明。
fn truncate_chars(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    format!("{}\n[截断] 输出共 {} 字符，仅保留前 {}", head, count, max)
}
