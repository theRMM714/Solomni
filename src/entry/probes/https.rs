//! 目的：用产品自己的出站代理打一次最小 HTTPS 请求（隐藏模式 --https-check）。
//! 管：参数解析、真实请求与四态结论打印。
//! 不管：TLS 后端选择与错误分类（在 llm 的 detail）。
//! 联动：由 main 在 --https-check 分支调用。

/// 目的：用**产品自己的出站代理**（含按平台装配的 TLS）打一次最小 HTTPS 请求，如实报结论。
///   四态机器可读：ok（通）/ no-net（环境连不上外网）/ env-tls（本进程取不到系统 TLS 凭证，如沙箱挡住凭证存储）/
///   tls-fail|fail（我们链路坏了）。
///   退出码恒 0：判定归调用方（测试按性质决定 env-skip 还是失败），这里只报事实。
pub fn https_check(args: &[String], i: usize) -> i32 {
    let url = args.get(i + 1).cloned().unwrap_or_default();
    if url.is_empty() {
        eprintln!("用法：solomni --https-check <https url>");
        return 2;
    }
    let backend = crate::capabilities::llm::detail::http_agent::tls_backend();
    let agent = crate::capabilities::llm::detail::http_agent::agent(10, 20);
    match agent.get(&url).call() {
        Ok(resp) => {
            println!("[HTTPS] ok {} {} {}", resp.status().as_u16(), backend, url);
            0
        }
        Err(e) => {
            println!(
                "[HTTPS] {} {} {}",
                crate::capabilities::llm::detail::http_agent::classify(&e),
                backend,
                e
            );
            0
        }
    }
}
