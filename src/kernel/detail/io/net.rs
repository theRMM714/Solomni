//! 目的：网络原语——出站 TLS 装配、入站回环服务、回环 TCP。
//! 管：`backend`（按平台装配 TLS）、`outbound`（ureq 代理的预算与状态语义）、`loopback`（入站回环 HTTP）、`tcp`（回环端口的预订 / 拨号 / 等待）。
//! 不管：请求内容、状态码判定、重试与错误归类（在各自单元）；代理与信任环境由各栈自己从环境拾取，这里不另写解析；路由语义在 handler 里。
//! 联动：对外面 `src/kernel/api/net.rs`；llm 的 http_agent / http_stream、provision 的 fetch、呈现层的转录中心与虚拟机的种子服务都经它取。

pub(crate) mod backend;
pub(crate) mod loopback;
pub(crate) mod outbound;
pub(crate) mod tcp;

#[cfg(test)]
mod tests {
    /// 目的：两条通道装配的是同一套平台 TLS——后端名称与平台一致，且两边都真的建得出来。
    #[test]
    fn two_channels_share_one_tls_decision() {
        let expected = if cfg!(windows) {
            "native-tls"
        } else {
            "rustls"
        };
        assert_eq!(super::backend::name(), expected);

        let _agent = super::outbound::agent(5, 30);

        super::backend::reqwest_builder(reqwest::Client::builder())
            .expect("平台 TLS 配置")
            .build()
            .expect("HTTP 客户端");
    }
}
