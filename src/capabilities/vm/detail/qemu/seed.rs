//! 目的：首启种子的小 HTTP 服务（cloud-init NoCloud net）——只在这次 boot 期间活着，随 guest 一起收掉。
//! 管：绑 127.0.0.1 随机端口、只认 /user-data 与 /meta-data、停服务并等线程退出（幂等）。
//! 不管：QEMU 参数与磁盘（qemu.rs）、QGA 通道（qga.rs）。
//! 联动：vm/domain/guest.rs 的 SeedDocs；由 qemu.rs 的启动路径构造。

use crate::capabilities::vm::domain::guest::SeedDocs;

/// 目的：首启种子的小 HTTP 服务（cloud-init NoCloud net）——只在这次 boot 期间活着，随 guest 一起收掉。
pub(super) struct SeedServer {
    server: std::sync::Arc<tiny_http::Server>,
    thread: Option<std::thread::JoinHandle<()>>,
    pub(super) port: u16,
}

impl SeedServer {
    /// 目的：绑 127.0.0.1 的随机端口起服务；路径只认 `/user-data` 与 `/meta-data`，其余 404。
    pub(super) fn start(docs: &SeedDocs) -> Result<SeedServer, String> {
        let server = std::sync::Arc::new(
            tiny_http::Server::http("127.0.0.1:0")
                .map_err(|e| format!("种子服务占不到端口：{}", e))?,
        );
        let port = server
            .server_addr()
            .to_ip()
            .map(|a| a.port())
            .ok_or_else(|| "种子服务没有 TCP 端口".to_string())?;
        let user_data = docs.user_data.clone();
        let meta_data = docs.meta_data.clone();
        let worker = std::sync::Arc::clone(&server);
        let thread = std::thread::spawn(move || {
            for req in worker.incoming_requests() {
                let path = req.url().split(['?', '#']).next().unwrap_or("").to_string();
                let resp = match path.as_str() {
                    "/user-data" => tiny_http::Response::from_string(user_data.clone()),
                    "/meta-data" => tiny_http::Response::from_string(meta_data.clone()),
                    _ => tiny_http::Response::from_string("").with_status_code(404),
                };
                let _ = req.respond(resp);
            }
        });
        Ok(SeedServer {
            server,
            thread: Some(thread),
            port,
        })
    }

    /// 目的：cloud-init NoCloud net 的种子 URL（guest 经 slirp 的 10.0.2.2 访问宿主）。
    pub(super) fn url(&self) -> String {
        format!("http://10.0.2.2:{}/", self.port)
    }

    /// 目的：停服务并等线程退出（幂等）。
    pub(super) fn stop(&mut self) {
        self.server.unblock();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for SeedServer {
    fn drop(&mut self) {
        self.stop();
    }
}
