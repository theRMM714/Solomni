//! 目的：GuestHost 的 QEMU 实现——拉起 qemu-system-*，经 QGA 通道执行与销毁。
//! 管：QEMU 进程的启停、QGA 的 JSON 收发、命令回执与 base64 数据。
//! 不管：基础根判定、挂载与 QEMU 参数派生（domain/guest.rs）。
//! 联动：端口在 ports.rs；由组合根构造、service.rs 持有。

use crate::capabilities::vm::domain::guest::{qemu_args, qemu_exe, GuestRun, GuestSpec};
use crate::capabilities::vm::ports::{Guest, GuestHost};
use crate::kernel::api::ToolOutcome;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// 目的：QEMU 后端——每次 start 拉起一个 qemu-system-* 进程并接上 QGA 通道。
pub struct QemuGuest;

impl QemuGuest {
    /// 目的：组合根专用。
    pub fn new() -> QemuGuest {
        QemuGuest
    }
}

impl GuestHost for QemuGuest {
    fn start(&self, spec: &GuestSpec) -> Result<Box<dyn Guest>, String> {
        let listener =
            TcpListener::bind("127.0.0.1:0").map_err(|e| format!("占不到 QGA 端口：{}", e))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let opts = format!("host=127.0.0.1,port={},server=on,wait=off", port);
        let exe = spec
            .qemu_path
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| qemu_exe(spec.arch).to_string());
        let child = Command::new(&exe)
            .args(qemu_args(spec, &opts))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("起不了 QEMU（{}）：{}", exe, e))?;
        drop(listener);
        let stream = connect_within(port, Duration::from_secs(30))?;
        let writer = stream.try_clone().map_err(|e| e.to_string())?;
        let mut handle = QemuHandle {
            child: Some(child),
            reader: BufReader::new(stream),
            writer,
        };
        // QEMU 只把额外 9p 共享接进来，不会替 guest 挂：接上 QGA 后逐个挂到宿主原路径。
        // 约束：挂载点要在 guest 根里建得出来——基础根因此按读写共享（与内核 root=... rw 一致）。
        for (i, m) in spec.mounts.iter().enumerate() {
            let ro = if m.read_only { ",ro" } else { "" };
            let script = format!(
                "mkdir -p '{g}' && mount -t 9p -o trans=virtio,version=9p2000.L{ro} m{i} '{g}'",
                g = m.guest,
                ro = ro,
                i = i
            );
            let out = handle.exec(&GuestRun {
                command: script,
                args_json: "{}".to_string(),
                env: Vec::new(),
                cwd: String::new(),
            });
            if !out.ok {
                handle.stop();
                return Err(format!("把 {} 挂进 guest 失败：{}", m.guest, out.output));
            }
        }
        Ok(Box::new(handle))
    }
}

fn connect_within(port: u16, budget: Duration) -> Result<TcpStream, String> {
    let deadline = Instant::now() + budget;
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(s) => return Ok(s),
            Err(e) => {
                if Instant::now() >= deadline {
                    return Err(format!("等 QGA 通道超时（端口 {}）：{}", port, e));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

struct QemuHandle {
    child: Option<Child>,
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl QemuHandle {
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
}

impl Guest for QemuHandle {
    fn exec(&mut self, run: &GuestRun) -> ToolOutcome {
        if let Err(why) = self.sync() {
            return fail(&why);
        }
        let env: Vec<serde_json::Value> = run
            .env
            .iter()
            .map(|(k, v)| serde_json::json!({"name":k,"value":v}))
            .collect();
        let script = if run.cwd.is_empty() {
            run.command.clone()
        } else {
            format!("cd {} && {}", run.cwd, run.command)
        };
        if let Err(why) = self.send(&serde_json::json!({
            "execute":"guest-exec",
            "arguments":{
                "path":"/bin/sh",
                "arg":["-c",script],
                "input-data": b64(&run.args_json),
                "capture-output":true,
                "env":env,
            }
        })) {
            return fail(&why);
        }
        let pid = match self.recv() {
            Ok(v) => match v
                .get("return")
                .and_then(|r| r.get("pid"))
                .and_then(|p| p.as_i64())
            {
                Some(p) => p,
                None => return fail("QGA 没给出进程号"),
            },
            Err(why) => return fail(&why),
        };
        let deadline = Instant::now() + Duration::from_secs(600);
        loop {
            if let Err(why) = self.send(&serde_json::json!({
                "execute":"guest-exec-status","arguments":{"pid":pid}
            })) {
                return fail(&why);
            }
            let v = match self.recv() {
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
                let output = format!(
                    "{}{}",
                    String::from_utf8_lossy(&out),
                    String::from_utf8_lossy(&err)
                );
                return ToolOutcome {
                    ok: code == 0,
                    output,
                };
            }
            if Instant::now() >= deadline {
                return fail("等 guest 里的命令超时");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn stop(&mut self) {
        let _ = self.send(&serde_json::json!({"execute":"guest-shutdown"}));
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
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
    let mut out = String::new();
    for chunk in s.as_bytes().chunks(3) {
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
