//! 目的：出站 HTTPS 的 ureq 代理——连接 / 总预算与状态语义的唯一一处。
//! 管：按预算构建 Agent；TLS 配置取自 `backend`（与 reqwest 通道同一份平台决策）。
//! 不管：具体协议、状态码判定与错误语义（在各自单元）；代理与信任环境（ureq 自己从环境拾取）。
//! 联动：`api::net::agent` 是单元唯一的取用口；llm 的 http_agent 与 provision 的 fetch 经它出站。

use std::time::Duration;

use super::backend;

/// 目的：构建带连接 / 总超时的通用代理。
/// 约束：4xx/5xx 当**数据**不当错误（`http_status_as_error(false)`），状态由调用方判——错误文案里才留得住对方原话。
pub fn agent(connect_secs: u64, total_secs: u64) -> ureq::Agent {
    let mut builder = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(connect_secs)))
        .timeout_global(Some(Duration::from_secs(total_secs)))
        .http_status_as_error(false);
    if let Some(tls) = backend::ureq_tls() {
        builder = builder.tls_config(tls);
    }
    builder.build().into()
}
