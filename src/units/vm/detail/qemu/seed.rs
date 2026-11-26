//! 目的：首启种子的小 HTTP 服务（cloud-init NoCloud net）——只在这次 boot 期间活着，随 guest 一起收掉。
//! 管：经内核的入站回环服务开户、只认 /user-data 与 /meta-data、给 guest 的访问地址。
//! 不管：绑定与停机（`src/kernel/detail/io/net/loopback.rs`）、QEMU 参数与磁盘（qemu.rs）、QGA 通道（qga.rs）。
//! 联动：vm/domain/guest.rs 的 SeedDocs；由 qemu.rs 的启动路径构造。

use crate::kernel::api::net::{self, Answer, Loopback};
use crate::units::vm::domain::guest::SeedDocs;

/// 目的：首启种子的小 HTTP 服务（cloud-init NoCloud net）——只在这次 boot 期间活着，随 guest 一起收掉。
pub(super) struct SeedServer {
    server: Loopback,
}

impl SeedServer {
    /// 目的：绑 127.0.0.1 的随机端口起服务；路径只认 `/user-data` 与 `/meta-data`，其余 404。
    pub(super) fn start(docs: &SeedDocs) -> Result<SeedServer, String> {
        let user_data = docs.user_data.clone();
        let meta_data = docs.meta_data.clone();
        let server = net::serve(0, 64 * 1024, move |call| {
            let path = call.path.split(['?', '#']).next().unwrap_or("");
            match path {
                "/user-data" => Answer::new(200, user_data.clone()),
                "/meta-data" => Answer::new(200, meta_data.clone()),
                _ => Answer::new(404, ""),
            }
        })
        .map_err(|e| format!("种子服务占不到端口：{}", e))?;
        Ok(SeedServer { server })
    }

    /// 目的：实际监听的端口（只给测试；guest 走 `url()` 里的 10.0.2.2）。
    #[cfg(test)]
    pub(super) fn port(&self) -> u16 {
        self.server.port()
    }

    /// 目的：cloud-init NoCloud net 的种子 URL（guest 经 slirp 的 10.0.2.2 访问宿主）。
    pub(super) fn url(&self) -> String {
        format!("http://10.0.2.2:{}/", self.server.port())
    }

    /// 目的：停服务并等工作线程退出（幂等）。
    pub(super) fn stop(&mut self) {
        self.server.stop();
    }
}
