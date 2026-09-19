//! HTTPS/TLS 探针（三个平台目标共用这**一份正文**：各自只在自己的平台上编译，所以只会跑一次）。
//! 为什么单独有它：TLS 后端是**按平台选的**（Windows = native-tls、unix = rustls），
//! 而 T3 明令不依赖外网，所以"这条构建的 TLS 栈真能连外网"只能在平台探针（T4）里验。
//! 三态如实区分（见 TESTING.md 的探针四态）：连不上外网 = env-skip（附命令行原话）；
//! TLS/HTTP 坏了 = 失败——绝不把"我们链路坏了"说成"环境不允许"；通了 = 通过并报状态码与后端。

use crate::probe::bin;
use std::process::Command;

/// 探针用的公网端点：稳定、无鉴权、走真 TLS（IANA 维护）。
const URL: &str = "https://example.com/";

#[test]
fn https_reaches_a_public_endpoint_through_the_product_chain() {
    let out = Command::new(bin())
        .arg("--https-check")
        .arg(URL)
        .output()
        .expect("跑 --https-check");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let line = stdout
        .lines()
        .find(|l| l.starts_with("[HTTPS]"))
        .unwrap_or("<没有 [HTTPS] 行>")
        .to_string();
    let mut it = line.split_whitespace();
    let kind = it.nth(1).unwrap_or("").to_string();
    match kind.as_str() {
        // 通了：再钉一下"真的拿到了响应"（2xx），否则说明链路只是没报错
        "ok" => {
            let code: u16 = it.next().and_then(|c| c.parse().ok()).unwrap_or(0);
            assert!((200..300).contains(&code), "公网端点的应答不正常：{}", line);
            println!("{}", line);
        }
        // 连不上外网：如实跳过（这不是被测代码的问题）
        "no-net" => eprintln!("[探针] 本环境连不上外网，HTTPS/TLS 探针跳过：{}", line),
        // 其余（tls-fail / fail / 空）：链路真有问题，必须失败
        _ => panic!("HTTPS/TLS 链路失败：{}", line),
    }
}
