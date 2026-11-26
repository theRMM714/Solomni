//! 目的：长驻子进程的唯一施工面——一次调用拿到「已在账上、父死子亡挂好、stderr 有人在抽」的句柄。
//! 管：进程组 / 父死子亡 / 作业对象的接线、长命线程上 spawn、登记入账、stderr 常驻抽干与尾行。
//! 不管：命令行怎么拼、就绪怎么判（端口 / 套接字 / 首行协议）、优雅停机语义与重启策略（在单元）。
//! 联动：单元经 `api::process::spawn` 用它，拿回 `Daemon`（同目录 `daemon.rs`）；平台机制与账在 `procs.rs`。

use super::daemon::{Daemon, Tail};
use super::{Entry, ProcessBook, attach, guard, now_ts, spawn_raw};
use std::ffi::OsString;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;

/// 目的：一条子进程标准流的要求。
/// 约束：`Piped` 的那一头由本机制转交——stdin / stdout 交给调用方取，stderr 由本机制常驻抽干。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stream {
    /// 目的：接到空设备——不读也不写（不需要交互的长驻进程）。
    Null,
    /// 目的：继承父进程那一条（围栏自己的说明要直达用户输出）。
    Inherit,
    /// 目的：接一根管道（按行协议、就绪首行、失败尾行都用它）。
    Piped,
}

impl Stream {
    fn stdio(self) -> Stdio {
        match self {
            Stream::Null => Stdio::null(),
            Stream::Inherit => Stdio::inherit(),
            Stream::Piped => Stdio::piped(),
        }
    }
}

/// 目的：stderr 每行转写的回调（进日志、上控制台）；要跨线程共享，所以是可克隆的闭包。
type Echo = Arc<dyn Fn(&str) + Send + Sync>;

/// 目的：起一个长驻子进程的完整要求——命令行、三路流、账上的身份，外加 stderr 的去处。
/// 约束：`purpose` / `owner` / `marker` 只是账上的标签与身份（对账用 `marker` 确认"还是我们那一个"），
///   不参与任何决策；身份给不准会让对账认不出、只能报不能收。
pub struct Spawn {
    program: PathBuf,
    args: Vec<OsString>,
    cwd: Option<PathBuf>,
    clear_env: bool,
    env: Vec<(OsString, OsString)>,
    stdin: Stream,
    stdout: Stream,
    stderr: Stream,
    tail: usize,
    echo: Option<Echo>,
    purpose: &'static str,
    owner: String,
    marker: String,
    book: Option<Arc<ProcessBook>>,
}

impl Spawn {
    /// 目的：从头拼一个要求（默认三路 Null、尾行 20 行、不转写 stderr）。
    pub fn new(program: impl Into<PathBuf>) -> Spawn {
        Spawn {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            clear_env: false,
            env: Vec::new(),
            stdin: Stream::Null,
            stdout: Stream::Null,
            stderr: Stream::Null,
            tail: 20,
            echo: None,
            purpose: "process",
            owner: String::new(),
            marker: String::new(),
            book: None,
        }
    }

    /// 目的：加一个参数。
    pub fn arg(mut self, arg: impl Into<OsString>) -> Spawn {
        self.args.push(arg.into());
        self
    }

    /// 目的：加一串参数。
    pub fn args<I, S>(mut self, args: I) -> Spawn
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// 目的：工作目录（不设就继承本进程）。
    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Spawn {
        self.cwd = Some(dir.into());
        self
    }

    /// 目的：先清空继承来的环境（凭据与密钥不跟着子进程走），再逐条给。
    /// 约束：必须在 `env` 之前调，语义与 `Command::env_clear` 一致。
    pub fn clear_env(mut self) -> Spawn {
        self.clear_env = true;
        self
    }

    /// 目的：追加一个环境变量。
    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Spawn {
        self.env.push((key.into(), value.into()));
        self
    }

    /// 目的：指定 stdin / stdout / stderr 的去处。
    pub fn stdin(mut self, stream: Stream) -> Spawn {
        self.stdin = stream;
        self
    }

    /// 目的：见 `stdin`。
    pub fn stdout(mut self, stream: Stream) -> Spawn {
        self.stdout = stream;
        self
    }

    /// 目的：见 `stdin`。
    pub fn stderr(mut self, stream: Stream) -> Spawn {
        self.stderr = stream;
        self
    }

    /// 目的：stderr 尾行留几行（0 = 只抽干不留）。
    pub fn tail(mut self, lines: usize) -> Spawn {
        self.tail = lines;
        self
    }

    /// 目的：stderr 每一行除了入尾行，再交给这里（日志与控制台转写）。
    /// 约束：在抽干线程上调用，**不能阻塞太久**——管道必须一直抽，写满会把子进程卡住。
    pub fn echo_stderr(mut self, echo: impl Fn(&str) + Send + Sync + 'static) -> Spawn {
        self.echo = Some(Arc::new(echo));
        self
    }

    /// 目的：账上的用途标签（guest / share:xxx / resident 这类）。
    pub fn purpose(mut self, purpose: &'static str) -> Spawn {
        self.purpose = purpose;
        self
    }

    /// 目的：账上的归属（租约名 / 挂载落点 / 常驻服务的身份）。
    pub fn owner(mut self, owner: impl Into<String>) -> Spawn {
        self.owner = owner.into();
        self
    }

    /// 目的：命令行里应出现的标记（Unix 靠它确认"还是我们那一个"）。
    pub fn marker(mut self, marker: impl Into<String>) -> Spawn {
        self.marker = marker.into();
        self
    }

    /// 目的：登记到这本账上（`None` = 不记账：只在测试或不该进账的场合用）。
    pub fn book(mut self, book: Option<Arc<ProcessBook>>) -> Spawn {
        self.book = book;
        self
    }
}

/// 目的：起一个长驻子进程，交回已经"在账上、挂好、stderr 有人在抽"的句柄。
/// 返回：起不来时给一条能直接给用户看的原因（带上被起的程序路径）。
/// 约束：spawn 走长命线程（`procs.rs` 的 `spawn_raw`）——父死子亡绑的是发起线程，不能在请求线程上起；
///   stderr 是 `Piped` 时抽干线程立刻起来，调用方不从 `Daemon` 取它也一直有人抽。
pub fn spawn(spec: Spawn) -> Result<Daemon, String> {
    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args)
        .stdin(spec.stdin.stdio())
        .stdout(spec.stdout.stdio())
        .stderr(spec.stderr.stdio());
    if let Some(dir) = &spec.cwd {
        cmd.current_dir(dir);
    }
    if spec.clear_env {
        cmd.env_clear();
    }
    for (key, value) in &spec.env {
        cmd.env(key, value);
    }
    // 父死子亡与独立进程组：核心被强杀时，这个进程连同它拉起的服务不该留下。
    guard(&mut cmd);
    let mut child =
        spawn_raw(cmd).map_err(|e| format!("起不了 {}：{}", spec.program.display(), e))?;
    let tail = match (spec.stderr, child.stderr.take()) {
        (Stream::Piped, Some(stderr)) => {
            let tail = Tail::new(spec.tail);
            drain_stderr(stderr, tail.clone(), spec.echo.clone());
            Some(tail)
        }
        _ => None,
    };
    // 作业对象（Windows）/ 守门进程（macOS）：核心一死，它跟着死；挂不上只如实少一层兜底，不阻断启动。
    let launched = attach(&child, spec.purpose);
    if let Some(book) = &spec.book {
        book.record(Entry {
            pid: child.id(),
            since: now_ts(),
            purpose: spec.purpose.to_string(),
            owner: spec.owner,
            marker: spec.marker,
            created: launched.created,
            guard_note: launched.guard_note,
        });
    }
    Ok(Daemon::new(child, tail, spec.book))
}

/// 目的：把 stderr 一直抽干——管道写满会把子进程卡住，这是必须常驻的一根线。
/// 约束：逐行转写（裸字节直写控制台在非 UTF-8 代码页下是乱码）；收尾时把没有换行的残行也交出去。
fn drain_stderr<R: Read + Send + 'static>(
    stderr: R,
    tail: Tail,
    echo: Option<Echo>,
) {
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(stderr);
        let mut buf = [0u8; 4096];
        let mut pending: Vec<u8> = Vec::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    pending.extend_from_slice(&buf[..n]);
                    while let Some(at) = pending.iter().position(|b| *b == b'\n') {
                        let line: Vec<u8> = pending.drain(..=at).collect();
                        emit_stderr_line(&line, &tail, echo.as_deref());
                    }
                }
            }
        }
        if !pending.is_empty() {
            emit_stderr_line(&pending, &tail, echo.as_deref());
        }
    });
}

/// 目的：一行的去处——转写给调用方，再入尾行。
fn emit_stderr_line(raw: &[u8], tail: &Tail, echo: Option<&(dyn Fn(&str) + Send + Sync)>) {
    let text = decode_line(raw);
    if text.is_empty() {
        return;
    }
    if let Some(echo) = echo {
        echo(&text);
    }
    tail.push(text);
}

/// 目的：把一行 stderr 字节解成可读文本——子进程的错误文本按 UTF-8 宽容解码（Windows 上也不是本地代码页）；
///   行尾的 CR/LF 与首尾空白去掉，全空的行走空串（调用方据此丢行）。
fn decode_line(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw)
        .trim_end_matches(['\r', '\n'])
        .trim()
        .to_string()
}

/// 目的：一条命令的标准流与账上身份要怎么给（测试夹具用）。
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// 目的：一行 stderr 按 UTF-8 宽容解码——中文错误文本不再被本地代码页解成乱码；坏字节也不炸、空白行丢空。
    #[test]
    fn a_stderr_line_decodes_as_utf8() {
        let chinese = "warning: failed to open(): 在一个非套接字上尝试了一个操作。";
        assert_eq!(decode_line(chinese.as_bytes()), chinese);
        assert_eq!(decode_line(b"line\r\n"), "line");
        assert!(decode_line(&[0xff, 0xfe, b'x']).contains('x'));
        assert_eq!(decode_line(b"   "), "");
    }

    /// 目的：起进程这一处要把账记全、把 stderr 抽干留尾行、收场时销账——单元面只该走这一条施工面。
    #[test]
    fn a_started_daemon_is_recorded_drained_and_forgotten() {
        let dir = crate::tests::scratch("procs-spawn-daemon");
        let book = ProcessBook::open(&dir.join(".home"));
        let mut spec = Spawn::new(if cfg!(windows) { "cmd" } else { "sh" })
            .stderr(Stream::Piped)
            .tail(5)
            .purpose("test")
            .owner("t-1")
            .marker("solomni:test")
            .book(Some(std::sync::Arc::clone(&book)));
        spec = if cfg!(windows) {
            spec.args(["/C", "echo boom 1>&2 & ping -n 20 127.0.0.1"])
        } else {
            spec.args(["-c", "echo boom >&2; sleep 20"])
        };
        let mut daemon = spawn(spec).expect("起一个长驻进程");
        let entries = book.list();
        assert_eq!(entries.len(), 1, "起进程这一处要登记：{:?}", entries);
        assert!(entries[0].pid > 0, "账上要记真 pid：{:?}", entries);
        assert_eq!(entries[0].purpose, "test");
        assert_eq!(entries[0].owner, "t-1");
        assert_eq!(entries[0].marker, "solomni:test");
        // 尾行是被抽干线程异步送来的：等到有内容为止，但不要拿固定 sleep 当断言。
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline && daemon.stderr_tail(5).is_empty() {
            std::thread::sleep(Duration::from_millis(20));
        }
        let tail = daemon.stderr_tail(5);
        assert!(
            tail.iter().any(|line| line.contains("boom")),
            "stderr 要被抽干并留尾行：{:?}",
            tail
        );
        assert!(daemon.try_wait().is_none(), "它该还在跑");
        daemon.stop();
        assert!(book.list().is_empty(), "收场要销账");
        assert!(daemon.try_wait().is_some(), "收过场就不该再判活着");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
