//! 目的：启动门探针——未授权且没有可回答的前端时，`-webUI` 也必须退出，不能先监听端口。
//! 管：起被测二进制、喂空 stdin、检查退出码与端口未开。
//! 不管：围栏机制本身与 ACL 写入；门在装配前退出，不碰本机权限，只在 target/ 下落日志。
//! 联动：`src/main.rs` 的 `run_gate`；语义见 `docs/kernel/README.md`。

use crate::probe::{bin, scratch};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::time::Duration;

#[test]
fn webui_does_not_start_without_fence_authorization() {
    let root = scratch("startup-gate-webui");
    let port = TcpListener::bind(("127.0.0.1", 0))
        .expect("占一个回环端口")
        .local_addr()
        .expect("取端口")
        .port();
    let port_text = port.to_string();
    let out = Command::new(bin())
        .args(["-webUI", "--web-port", &port_text, "--root"])
        .arg(&root)
        .env_remove("SOLOMNI_SANDBOX_DEGRADED")
        .env("SOLOMNI_FENCE_WRITE", "0")
        .stdin(Stdio::null())
        .output()
        .expect("起被测二进制");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!out.status.success(), "环境不满足时不能成功启动：{text}");
    assert!(
        text.contains("没有可回答的前端"),
        "要如实说清为什么退出：{text}"
    );
    let addr = format!("127.0.0.1:{port}").parse().expect("地址");
    assert!(
        TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_err(),
        "门没过就不该监听 {port}"
    );
}
