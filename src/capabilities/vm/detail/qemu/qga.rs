//! 目的：QGA 通道——一次 boot 的连接、JSON 收发、guest 命令与文件传输、收尾。
//! 管：QemuHandle 的 ping/重连、命令回执、base64 与输出裁剪、shutdown 与 Drop。
//! 不管：QEMU 参数与磁盘（qemu.rs）、种子服务（seed.rs）。
//! 联动：vm/ports.rs 的 GuestChannel 与 GuestVm。

use super::seed::SeedServer;
use super::UNFENCED_GUEST_RUN_SECS;
use crate::capabilities::vm::domain::guest::{GuestAccess, GuestRun, OutputLimit};
use crate::capabilities::vm::ports::{GuestChannel, GuestVm};
use crate::kernel::api::ToolOutcome;
use crate::kernel::detail::procs::ProcessBook;
use crate::kernel::domain::receipt::{shell_receipt, ShellEnd};
use crate::kernel::ports::ProcessTexts;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Child;
use std::time::{Duration, Instant};

pub(super) fn connect_within(
    port: u16,
    budget: Duration,
    mut child: Option<&mut Child>,
) -> Result<TcpStream, String> {
    let deadline = Instant::now() + budget;
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(s) => return Ok(s),
            Err(e) => {
                if let Some(proc) = child.as_deref_mut() {
                    if let Ok(Some(status)) = proc.try_wait() {
                        return Err(format!("QEMU 进程已退出（{}）：{}", status, e));
                    }
                }
                if Instant::now() >= deadline {
                    return Err(format!("等 QGA 通道超时（端口 {}）：{}", port, e));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

pub(super) struct QemuHandle {
    pub(super) child: Option<Child>,
    pub(super) reader: BufReader<TcpStream>,
    pub(super) writer: TcpStream,
    /// 目的：QGA 的本地端口——通道被 QEMU 重置时照它重连（chardev 准备好之前连上的那条会被 RST）。
    pub(super) port: u16,
    /// 目的：首启种子服务；正常会话启动是 None。
    pub(super) seed: Option<SeedServer>,
    /// 目的：每租约可写层文件；stop / Drop 时删掉（删会话即丢写层）。
    pub(super) overlay: Option<PathBuf>,
    /// 目的：长驻子进程的账（有就在收尾时注销这一条）。
    pub(super) book: Option<std::sync::Arc<ProcessBook>>,
    /// 目的：这个句柄对应的 QEMU 进程（注销登记用）。
    pub(super) pid: u32,
    /// 目的：shell 回执的收尾文案（来自引擎，与本机守门进程同一份）。
    pub(super) texts: std::sync::Arc<dyn ProcessTexts>,
}


impl QemuHandle {
    /// 目的：优雅收尾——请 guest 自己断电，**等它落盘退出**（超时才杀）；再收种子服务与可写层。
    /// 约束：guest-shutdown 是异步的——不等它，guest 里刚要落盘的元数据（如刚建的 python 符号链接）会丢。
    fn shutdown_and_wait(&mut self) {
        let asked = self
            .send(&serde_json::json!({"execute":"guest-shutdown"}))
            .is_ok();
        if let Some(mut s) = self.seed.take() {
            s.stop();
        }
        if let Some(mut c) = self.child.take() {
            let mut waited = false;
            if asked {
                let deadline = Instant::now() + Duration::from_secs(8);
                loop {
                    match c.try_wait() {
                        Ok(Some(_)) => {
                            waited = true;
                            break;
                        }
                        Ok(None) if Instant::now() < deadline => {
                            std::thread::sleep(Duration::from_millis(100));
                        }
                        _ => break,
                    }
                }
            }
            if !waited {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
        if let Some(path) = self.overlay.take() {
            let _ = std::fs::remove_file(&path);
        }
        if let Some(book) = &self.book {
            book.forget(self.pid);
        }
    }

    /// 目的：通道断了就照同一个端口重连一次，并把读超时照原样装回去。
    /// 约束：QEMU 的 chardev 还没就绪时连上的那条会被它 RST——"连上"不等于"通"，断了就重连。
    fn reconnect(&mut self) -> Result<(), String> {
        let stream = connect_within(self.port, Duration::from_secs(5), self.child.as_mut())?;
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .map_err(|e| format!("给 QGA 通道设读超时失败：{}", e))?;
        self.writer = stream.try_clone().map_err(|e| e.to_string())?;
        self.reader = BufReader::new(stream);
        Ok(())
    }

    /// 目的：握手一次；不通就退避重连再握手（通道被 QEMU 换掉是常态，不是故障）。
    /// 约束：有界（约 15 秒）——guest agent 刚就绪时可能连着换几次通道，换一次不够； 真起不来时仍如实报错，不无限等。
    fn sync_or_heal(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let last = match self.sync() {
                Ok(()) => return Ok(()),
                Err(e) => e,
            };
            if Instant::now() >= deadline {
                return Err(last);
            }
            std::thread::sleep(Duration::from_millis(200));
            let _ = self.reconnect();
        }
    }

    /// 目的：问一次进程状态；通道断了先重连再问一次（进程状态在 guest 侧，换通道不影响）。
    fn status_once(&mut self, pid: i64) -> Result<serde_json::Value, String> {
        let ask = |h: &mut QemuHandle| -> Result<serde_json::Value, String> {
            h.send(&serde_json::json!({
                "execute":"guest-exec-status","arguments":{"pid":pid}
            }))?;
            h.recv()
        };
        match ask(self) {
            Ok(v) => Ok(v),
            Err(_) => {
                self.reconnect()?;
                ask(self)
            }
        }
    }
    fn send(&mut self, v: &serde_json::Value) -> Result<(), String> {
        let mut line = v.to_string();
        line.push('\n');
        self.writer
            .write_all(line.as_bytes())
            .map_err(|e| e.to_string())
    }

    fn recv(&mut self) -> Result<serde_json::Value, String> {
        let mut line = String::new();
        let n = self
            .reader
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("QGA 通道断了".to_string());
        }
        serde_json::from_str(line.trim()).map_err(|e| format!("QGA 回包不是 JSON：{}", e))
    }

    fn sync(&mut self) -> Result<(), String> {
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1);
        self.send(&serde_json::json!({"execute":"guest-sync","arguments":{"id":id}}))?;
        for _ in 0..64 {
            let v = self.recv()?;
            if v.get("return").and_then(|r| r.as_u64()) == Some(id) {
                return Ok(());
            }
        }
        Err("QGA 同步不上".to_string())
    }

    /// 目的：打开 guest 里的一个文件，拿回 QGA 句柄。
    fn file_open(&mut self, path: &str, mode: &str) -> Result<i64, String> {
        self.send(&serde_json::json!({
            "execute":"guest-file-open",
            "arguments":{"path":path,"mode":mode}
        }))?;
        let v = self.recv()?;
        v.get("return")
            .and_then(|r| r.as_i64())
            .ok_or_else(|| "QGA 没给出文件句柄".to_string())
    }

    /// 目的：关掉 guest 文件句柄（回包不关心）。
    fn file_close(&mut self, handle: i64) {
        let _ = self.send(&serde_json::json!({
            "execute":"guest-file-close",
            "arguments":{"handle":handle}
        }));
        let _ = self.recv();
    }
}

impl QemuHandle {
    /// 目的：一次 guest 执行；带 access 时经 guest 内助手按 per-agent 允许集执行，失败绝不回退到无围栏。
    fn exec_run(
        &mut self,
        run: &GuestRun,
        access: Option<&GuestAccess>,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> ToolOutcome {
        if let Err(why) = self.sync_or_heal() {
            return fail(&why);
        }
        // QGA 的 guest-exec 只收 "NAME=VALUE" 的字符串数组（对象形状会被它按参数类型拒掉）。
        let env: Vec<String> = run
            .env
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        let request = if let Some(access) = access {
            let policy = serde_json::json!({
                "script": run.command,
                "cwd": run.cwd,
                "rw": access.rw,
                "ro": access.ro,
                "ro_tree": access.ro_tree,
                "private": access.private,
                "net": access.net,
                // 本次执行的超时秒数：guest helper 到点杀整棵进程树。
                "timeout_secs": run.timeout_secs,
                // 只传隐秘字段的**名字**（值走 QGA env），不把值写进临时文件。
                "env_names": run.env.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>(),
                // 系统级只读诊断：helper 在 Landlock restrict 之后走诊断模式（不 spawn 命令）。
                "diagnose": access.diagnose,
            });
            let n = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(1);
            let path = format!("/tmp/solomni-fence-{}-{}.json", std::process::id(), n);
            if let Err(why) = self.put_file(&path, policy.to_string().as_bytes()) {
                return fail(&format!("把 per-agent 围栏写进 guest 失败：{}", why));
            }
            serde_json::json!({
                "execute":"guest-exec",
                "arguments":{
                    "path": super::super::guest_helper::HELPER_PATH,
                    "arg":[path],
                    "input-data": b64(&run.args_json),
                    "capture-output":true,
                    "env":env,
                }
            })
        } else {
            let script = if run.cwd.is_empty() {
                run.command.clone()
            } else {
                format!("cd {} && {}", run.cwd, run.command)
            };
            serde_json::json!({
                "execute":"guest-exec",
                "arguments":{
                    "path":"/bin/sh",
                    "arg":["-c",script],
                    "input-data": b64(&run.args_json),
                    "capture-output":true,
                    "env":env,
                }
            })
        };
        if let Err(why) = self.send(&request) {
            return fail(&why);
        }
        let pid = match self.recv() {
            Ok(v) => match v
                .get("return")
                .and_then(|r| r.get("pid"))
                .and_then(|p| p.as_i64())
            {
                Some(p) => p,
                // 回执里没有 pid 通常是一条 error（例如 QGA 拒了这次 exec）：把原话带出来，别只说"没给"。
                None => return fail(&format!("QGA 没给出进程号：{}", v)),
            },
            Err(why) => return fail(&why),
        };
        // 有围栏时 helper 自己按本次 timeout_secs 杀树，外层只留一段缓冲；供给编排（无围栏）要等 apt / 下载完。
        let budget = if access.is_some() {
            run.timeout_secs + 30
        } else {
            UNFENCED_GUEST_RUN_SECS
        };
        let deadline = Instant::now() + Duration::from_secs(budget);
        let mut cancel_sent = false;
        loop {
            if !cancel_sent {
                if let Some(flag) = cancel {
                    if flag.load(std::sync::atomic::Ordering::Relaxed) {
                        cancel_sent = true;
                        // 先给 guest helper 发 SIGTERM：它的事件循环会杀掉子进程组再退出。
                        let kill = format!("kill -TERM {}", pid);
                        let _ = self.exec_run(
                            &GuestRun {
                                command: kill,
                                args_json: "{}".to_string(),
                                env: Vec::new(),
                                cwd: String::new(),
                                timeout_secs: 5,
                                output_limit: OutputLimit::Head,
                            },
                            None,
                            None,
                        );
                    }
                }
            }
            let v = match self.status_once(pid) {
                Ok(v) => v,
                Err(why) => return fail(&why),
            };
            let Some(ret) = v.get("return") else {
                return fail("QGA 回包里没有 return");
            };
            if ret.get("exited").and_then(|b| b.as_bool()) == Some(true) {
                let code = ret.get("exitcode").and_then(|c| c.as_i64()).unwrap_or(-1);
                let out = ret
                    .get("out-data")
                    .and_then(|d| d.as_str())
                    .map(unb64)
                    .unwrap_or_default();
                let err = ret
                    .get("err-data")
                    .and_then(|d| d.as_str())
                    .map(unb64)
                    .unwrap_or_default();
                let out = String::from_utf8_lossy(&out).into_owned();
                let err = String::from_utf8_lossy(&err).into_owned();
                if run.output_limit == OutputLimit::Shell {
                    return shell_outcome(self.texts.as_ref(), run, &out, &err, code, cancel_sent);
                }
                return ToolOutcome {
                    ok: code == 0,
                    output: cap_output(format!("{}{}", out, err), run.output_limit),
                };
            }
            if Instant::now() >= deadline {
                // 外层兜底：helper 没在预算内收场——按取消通道杀它（helper 会杀整棵进程组），再按 shell 契约收口。
                let kill = format!("kill -TERM {}", pid);
                let _ = self.exec_run(
                    &GuestRun {
                        command: kill,
                        args_json: "{}".to_string(),
                        env: Vec::new(),
                        cwd: String::new(),
                        timeout_secs: 5,
                        output_limit: OutputLimit::Head,
                    },
                    None,
                    None,
                );
                if run.output_limit == OutputLimit::Shell {
                    return ToolOutcome {
                        ok: false,
                        output: shell_receipt(
                            self.texts.as_ref(),
                            "",
                            "",
                            ShellEnd::TimedOut {
                                secs: run.timeout_secs,
                            },
                            GUEST_SHELL_MAX_CHARS,
                        ),
                    };
                }
                return fail("等 guest 里的命令超时");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// 目的：shell 回执的正文上限（与本机守门进程同一口径）。
pub(super) const GUEST_SHELL_MAX_CHARS: usize = 16_000;

/// 目的：guest shell 回执的收尾判定——只看 helper 的机器标记与宿主取消旗，不猜自然语言。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuestShellEnd {
    /// 目的：子进程（脚本）返回了退出码。
    Exit(i64),
    /// 目的：helper 到点杀了整棵进程树。
    Timeout,
    /// 目的：宿主取消旗已置位。
    Cancelled,
    /// 目的：子进程被信号结束，没有退出码。
    Signal,
    /// 目的：helper 的围栏/启动失败——命令没有执行，不产出 shell 收尾行。
    NotRun,
}

/// 目的：把一次 guest 执行的结果翻成 shell 回执（正文分区 + 状态行）；非 shell 运行的形状不受影响。
/// 约束：只有真正跑过命令才产出 `[shell]` 收尾行；helper 的围栏/启动失败保留诊断原文。
pub(super) fn shell_outcome(
    texts: &dyn ProcessTexts,
    run: &GuestRun,
    out: &str,
    err: &str,
    code: i64,
    cancel_sent: bool,
) -> ToolOutcome {
    let end = guest_shell_end(err, code, cancel_sent);
    if end == GuestShellEnd::NotRun {
        return ToolOutcome {
            ok: false,
            output: cap_output(format!("{}{}", out, err), run.output_limit),
        };
    }
    let cleaned = strip_helper_markers(err);
    let (end, ok) = match end {
        GuestShellEnd::Exit(c) => (ShellEnd::Exit(Some(c as i32)), c == 0),
        GuestShellEnd::Timeout => (
            ShellEnd::TimedOut {
                secs: run.timeout_secs,
            },
            false,
        ),
        GuestShellEnd::Cancelled => (ShellEnd::Cancelled, false),
        GuestShellEnd::Signal => (ShellEnd::Exit(None), false),
        GuestShellEnd::NotRun => unreachable!("上面已提前返回"),
    };
    ToolOutcome {
        ok,
        output: shell_receipt(texts, out, &cleaned, end, GUEST_SHELL_MAX_CHARS),
    }
}

/// 目的：按 helper 的机器标记与取消旗判定收尾；围栏失败（die 诊断）判为没执行。
pub(super) fn guest_shell_end(err: &str, code: i64, cancel_sent: bool) -> GuestShellEnd {
    if cancel_sent {
        return GuestShellEnd::Cancelled;
    }
    let mut not_run = false;
    for line in err.lines() {
        let line = line.trim();
        if line == super::super::guest_helper::HELPER_TIMEOUT_MARKER {
            return GuestShellEnd::Timeout;
        }
        if line == super::super::guest_helper::HELPER_SIGNAL_MARKER {
            return GuestShellEnd::Signal;
        }
        if line.starts_with(super::super::guest_helper::HELPER_DIAG_PREFIX) {
            not_run = true;
        }
    }
    if not_run {
        return GuestShellEnd::NotRun;
    }
    GuestShellEnd::Exit(code)
}

/// 目的：把 helper 的机器标记行剥掉（人读文案由 ProcessTexts 产出；标记不进最终回执）。
pub(super) fn strip_helper_markers(err: &str) -> String {
    let kept: Vec<&str> = err
        .lines()
        .filter(|l| {
            let line = l.trim();
            line != super::super::guest_helper::HELPER_TIMEOUT_MARKER
                && line != super::super::guest_helper::HELPER_SIGNAL_MARKER
        })
        .collect();
    let mut out = kept.join("\n");
    if !out.is_empty() && err.ends_with('\n') {
        out.push('\n');
    }
    out
}

impl GuestChannel for QemuHandle {
    /// 目的：QGA 就绪探测——同步握手能过一次就算通道已就绪（读超时保证有界）。
    fn ping(&mut self) -> bool {
        self.sync_or_heal().is_ok()
    }

    fn exec(&mut self, run: &GuestRun) -> ToolOutcome {
        self.exec_run(run, None, None)
    }

    fn exec_fenced(&mut self, run: &GuestRun, access: &GuestAccess) -> ToolOutcome {
        self.exec_run(run, Some(access), None)
    }

    fn exec_fenced_cancellable(
        &mut self,
        run: &GuestRun,
        access: &GuestAccess,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> ToolOutcome {
        self.exec_run(run, Some(access), cancel)
    }

    /// 目的：把字节分块写进 guest 文件（QGA guest-file-write，base64）。
    fn put_file(&mut self, path: &str, bytes: &[u8]) -> Result<(), String> {
        self.sync()?;
        let handle = self.file_open(path, "wb")?;
        let mut off = 0;
        while off < bytes.len() {
            let end = (off + 3072).min(bytes.len());
            let encoded = b64_bytes(&bytes[off..end]);
            if let Err(why) = self.send(&serde_json::json!({
                "execute":"guest-file-write",
                "arguments":{"handle":handle,"buf-b64":encoded}
            })) {
                self.file_close(handle);
                return Err(why);
            }
            let v = match self.recv() {
                Ok(v) => v,
                Err(why) => {
                    self.file_close(handle);
                    return Err(why);
                }
            };
            let n = v
                .get("return")
                .and_then(|r| r.get("count"))
                .and_then(|c| c.as_i64())
                .unwrap_or(0);
            if n <= 0 {
                self.file_close(handle);
                return Err("QGA 写 guest 文件返回 0 字节".to_string());
            }
            off += n as usize;
        }
        self.file_close(handle);
        Ok(())
    }

    /// 目的：把 guest 文件按块读回来（QGA guest-file-read，base64）。
    fn get_file(&mut self, path: &str) -> Result<Vec<u8>, String> {
        self.sync()?;
        let handle = self.file_open(path, "rb")?;
        let mut out: Vec<u8> = Vec::new();
        loop {
            if let Err(why) = self.send(&serde_json::json!({
                "execute":"guest-file-read",
                "arguments":{"handle":handle,"count":65536}
            })) {
                self.file_close(handle);
                return Err(why);
            }
            let v = match self.recv() {
                Ok(v) => v,
                Err(why) => {
                    self.file_close(handle);
                    return Err(why);
                }
            };
            let ret = match v.get("return") {
                Some(r) => r,
                None => {
                    self.file_close(handle);
                    return Err("QGA 读文件回包里没有 return".to_string());
                }
            };
            let n = ret.get("count").and_then(|c| c.as_i64()).unwrap_or(0);
            if n <= 0 {
                break;
            }
            if let Some(s) = ret.get("buf-b64").and_then(|d| d.as_str()) {
                out.extend(unb64(s));
            }
            if n < 65536 {
                break;
            }
        }
        self.file_close(handle);
        Ok(out)
    }
}

impl GuestVm for QemuHandle {
    fn channel(&mut self) -> &mut dyn GuestChannel {
        self
    }

    fn stop(&mut self) {
        self.shutdown_and_wait();
    }
}

impl Drop for QemuHandle {
    /// 目的：兜底收尾——种子服务、QEMU 进程与可写层都不能因为提前 drop 而漏掉；同样先请 guest 优雅断电。
    fn drop(&mut self) {
        self.shutdown_and_wait();
    }
}

/// 目的：guest 输出与本机守门进程同一份上限；截断时如实报原字符数。
fn cap_output(text: String, limit: OutputLimit) -> String {
    const MAX: usize = 16_000;
    let count = text.chars().count();
    if count <= MAX {
        return text;
    }
    match limit {
        OutputLimit::Head => {
            let head: String = text.chars().take(MAX).collect();
            format!("{}\n[截断] 输出共 {} 字符，仅保留前 {}", head, count, MAX)
        }
        OutputLimit::HeadAndTail | OutputLimit::Shell => {
            let head_n = MAX / 4;
            let tail_n = MAX - head_n;
            let head: String = text.chars().take(head_n).collect();
            let tail: String = text.chars().skip(count - tail_n).collect();
            format!(
                "{}\n[截断] 输出共 {} 字符；保留开头 {} 与结尾 {} 字符\n{}",
                head, count, head_n, tail_n, tail
            )
        }
    }
}

fn fail(why: &str) -> ToolOutcome {
    ToolOutcome {
        ok: false,
        output: why.to_string(),
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64(s: &str) -> String {
    b64_bytes(s.as_bytes())
}

fn b64_bytes(data: &[u8]) -> String {
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
        out.push(B64[((n >> 18) & 63) as usize] as char);
        out.push(B64[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            B64[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn unb64(s: &str) -> Vec<u8> {
    let mut vals: Vec<u8> = Vec::new();
    for c in s.bytes() {
        if c == b'=' {
            break;
        }
        if let Some(i) = B64.iter().position(|x| *x == c) {
            vals.push(i as u8);
        }
    }
    let mut out = Vec::new();
    for chunk in vals.chunks(4) {
        let mut n: u32 = 0;
        for (i, v) in chunk.iter().enumerate() {
            n |= (*v as u32) << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    out
}

