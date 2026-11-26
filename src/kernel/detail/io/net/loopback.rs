//! 目的：入站回环 HTTP 服务——只绑 127.0.0.1，请求体有界，停服务时能把线程收回来。
//! 管：绑端口（含 0 = 让系统给一个）、每请求一线程、按上限读 UTF-8 body、回包、`stop` 与 Drop 的幂等收场。
//! 不管：路由、状态码的含义与业务语义（都在 handler 里）；长轮询等多久（handler 自己决定）；坏 body（超限或不是 UTF-8）由这里直接回 400 纯文本，handler 见不到它——别在这里加产品自己的错误信封。
//! 联动：呈现层的转录中心（`src/presentation/web.rs`）与虚拟机的首启种子服务（`src/units/vm/detail/`）都经这里开户。

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

/// 目的：一次入站请求；body 已经按上限解码成 UTF-8（坏的在机制里就被挡掉了）。
pub struct Call {
    /// 目的：请求方法（原样大写形式）。
    pub method: String,
    /// 目的：请求路径（含 query，未解码）。
    pub path: String,
    /// 目的：请求体文本（已按上限校验过 UTF-8）。
    pub body: String,
}

/// 目的：一次应答——状态码、任意条头、UTF-8 body。
pub struct Answer {
    /// 目的：HTTP 状态码。
    pub code: u16,
    /// 目的：响应头（名与值都按原样写出）。
    pub headers: Vec<(String, String)>,
    /// 目的：响应体文本。
    pub body: String,
}

impl Answer {
    /// 目的：只要状态码与 body 的常见形状（不带自定义头）。
    pub fn new(code: u16, body: impl Into<String>) -> Answer {
        Answer {
            code,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    /// 目的：加一条响应头（同名多条按调用顺序追加）。
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Answer {
        self.headers.push((name.into(), value.into()));
        self
    }
}

/// 处理一次请求：可以阻塞（长轮询就靠这个），机制给每个请求一线程。
type Handler = dyn Fn(Call) -> Answer + Send + Sync;

/// 目的：一块已经绑好的回环监听——丢掉它就收摊（也可以显式 `stop`）。
pub struct Loopback {
    server: Arc<tiny_http::Server>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    port: u16,
}

/// 目的：绑 `127.0.0.1:<port>` 起服务；`port = 0` 时由系统分配（随后用 `port()` 问）。
/// 参数：`max_body` = 请求体上限（字节，含）；超限与非法 UTF-8 都在这里挡成 400。
pub fn serve<F>(port: u16, max_body: usize, handler: F) -> Result<Loopback, String>
where
    F: Fn(Call) -> Answer + Send + Sync + 'static,
{
    let addr = format!("127.0.0.1:{}", port);
    let server = Arc::new(
        tiny_http::Server::http(addr.as_str()).map_err(|e| format!("绑不了 {}：{}", addr, e))?,
    );
    let port = server
        .server_addr()
        .to_ip()
        .map(|a| a.port())
        .ok_or_else(|| "入站服务没有 TCP 端口".to_string())?;
    let flag = Arc::new(AtomicBool::new(false));
    let worker = Arc::clone(&server);
    let check = Arc::clone(&flag);
    let handler = Arc::new(handler);
    let thread = std::thread::spawn(move || {
        while !check.load(Ordering::Relaxed) {
            match worker.recv_timeout(Duration::from_millis(200)) {
                Ok(Some(request)) => {
                    // 一线一请求：长轮询绝不阻塞别的请求（转录中心是多端并用的）。
                    let handler = Arc::clone(&handler);
                    std::thread::spawn(move || answer(request, max_body, handler.as_ref()));
                }
                Ok(None) => {}
                // unblock() 之后 recv 以 Err 收场：往外走，别在这儿空转。
                Err(_) => break,
            }
        }
    });
    Ok(Loopback {
        server,
        stop: flag,
        thread: Some(thread),
        port,
    })
}

impl Loopback {
    /// 目的：实际监听端口（`port = 0` 时只有这里能知道真值）。
    pub fn port(&self) -> u16 {
        self.port
    }

    /// 目的：本机访问用的根地址（不带尾斜杠）。
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// 目的：停服务并等工作线程退出（幂等：反复调不会重复 join）。
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.server.unblock();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Loopback {
    fn drop(&mut self) {
        self.stop();
    }
}

/// 目的：把一次请求变成一次应答；读不出合法 body 就直接回 400，不交给业务。
fn answer(mut request: tiny_http::Request, max_body: usize, handler: &Handler) {
    let method = request.method().to_string();
    let path = request.url().to_string();
    let mut body = String::new();
    // 多读一个字节就分得清「刚好满」与「超了」：超限的请求不必整个读进内存。
    let read = request
        .as_reader()
        .take(max_body as u64 + 1)
        .read_to_string(&mut body);
    let reply = match read {
        Ok(_) if body.len() <= max_body => handler(Call { method, path, body }),
        _ => Answer::new(400, "请求体过大或不是 UTF-8")
            .header("Content-Type", "text/plain; charset=utf-8"),
    };
    let mut response = tiny_http::Response::from_string(reply.body).with_status_code(reply.code);
    for (name, value) in reply.headers {
        if let Ok(h) = tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()) {
            response = response.with_header(h);
        }
    }
    let _ = request.respond(response);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Duration;

    /// 目的：手写一条请求并读完整个响应（带 Connection: close，读到 EOF 为止）。
    fn round_trip(port: u16, raw: &str) -> String {
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("连上回环服务");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("设超时");
        stream.write_all(raw.as_bytes()).expect("写请求");
        let mut out = String::new();
        stream.read_to_string(&mut out).expect("读响应");
        out
    }

    #[test]
    fn a_loopback_serves_with_its_own_port_and_stops() {
        let mut server = serve(0, 1024, |call| {
            Answer::new(200, format!("{} {}", call.method, call.path))
        })
        .expect("起服务");
        assert!(server.port() > 0, "0 端口要换成真端口");
        assert_eq!(server.url(), format!("http://127.0.0.1:{}", server.port()));
        let response = round_trip(
            server.port(),
            "GET /ping HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        );
        assert!(
            response.contains("GET /ping") && response.contains("200"),
            "{}",
            response
        );
        server.stop();
        server.stop(); // 幂等
    }

    #[test]
    fn an_oversized_body_is_refused_before_the_handler() {
        let server = serve(0, 4, |_| Answer::new(200, "handler 不该被调到")).expect("起服务");
        let response = round_trip(
            server.port(),
            "POST /up HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Length: 9\r\n\r\n123456789",
        );
        assert!(response.contains("400"), "{}", response);
        assert!(response.contains("请求体过大"), "{}", response);
    }
}
