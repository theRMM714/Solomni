//! 目的：provision 的出站端口——编排需要什么就声明什么，实现放在 detail。
//! 管：取数（网络/摘要）这一组抽象；下游按端口签名实现，service 只按语义调用。
//! 不管：具体协议、超时、代理（在 detail::fetch）；清单怎么解析（在 service）。
//! 联动：service 持有 Arc<dyn Fetcher>；组合根构造 detail::fetch::HttpFetch 注入。

use std::path::Path;

/// 目的：取数与摘要——下载资产、读目录页文本、算文件 sha256、探源速度。
pub trait Fetcher: Send + Sync {
    /// 目的：取一个文本页（列包名用）；非 2xx 如实报错。
    fn text(&self, url: &str) -> Result<String, String>;

    /// 目的：算文件摘要（小写十六进制）——完整性判据。
    fn digest(&self, path: &Path) -> Result<String, String>;

    /// 目的：一次 2 MB 范围 GET，回（状态码, KB/s）；网络层失败记 000。
    fn sample(&self, url: &str) -> (u16, f64);

    /// 目的：把 URL 下到目标文件——选源后带续传；半截文件留作下次断点。
    fn download(&self, url: &str, dest: &Path, on_line: &mut dyn FnMut(&str))
        -> Result<(), String>;
}
