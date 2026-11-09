//! 目的：网络取数——选源测速、带续传的下载、目录页取文本与文件摘要。
//! 管：候选源派生（上游 + 可选前缀镜像）、2 MB 测速、.part 断点、sha256 计算。
//! 不管：装什么、装到哪（在 service 的编排里）；解包（在 kernel::detail::unpack）。
//! 联动：由组合根构造并经 ports::Fetcher 交给 service；本文件是 provision 唯一碰 ureq 的地方。

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::capabilities::provision::ports::Fetcher;

/// 目的：HTTP 的 Fetcher 实现——一个 ureq agent 走完全部请求（连接与代理配置只建一次）。
pub struct HttpFetch {
    agent: ureq::Agent,
}

impl HttpFetch {
    /// 目的：按（连接超时, 总超时）建实现；代理从环境变量拾取（交给 ureq 默认配置）。
    pub fn new(connect_secs: u64, total_secs: u64) -> HttpFetch {
        HttpFetch {
            agent: crate::kernel::detail::outbound::agent(connect_secs, total_secs),
        }
    }
}

impl Fetcher for HttpFetch {
    fn text(&self, url: &str) -> Result<String, String> {
        get_text(&self.agent, url)
    }

    fn digest(&self, path: &Path) -> Result<String, String> {
        sha256_file(path)
    }

    fn download(
        &self,
        url: &str,
        dest: &Path,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(), String> {
        download(&self.agent, url, dest, on_line)
    }
}

/// 目的：把 URL 下到目标文件——选源（前缀镜像测速）后带续传下载；半截文件留作下次续传的断点。
/// 代理：ureq 的默认配置从环境拾取（HTTPS_PROXY / HTTP_PROXY / NO_PROXY），这里不另写一套。
pub(crate) fn download(
    agent: &ureq::Agent,
    url: &str,
    dest: &Path,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let picked = pick_source(agent, &source_candidates(url), on_line)?;
    download_with_resume(agent, &picked, dest, on_line)
}

/// 目的：下载候选——上游 URL 在前；配了 SOLOMNI_GH_MIRROR 时再加一条前缀镜像（只对 GitHub 资产）。
pub(crate) fn source_candidates(upstream: &str) -> Vec<String> {
    let mirror = std::env::var("SOLOMNI_GH_MIRROR").unwrap_or_default();
    let mut out = vec![upstream.to_string()];
    if let Some(m) = mirror_of(upstream, &mirror) {
        out.push(m);
    }
    out
}

/// 目的：前缀镜像的写法——`<镜像根>/github.com/<owner>/<repo>/...`；不是 GitHub 资产或没配镜像就不加。
pub(crate) fn mirror_of(upstream: &str, mirror: &str) -> Option<String> {
    let root = mirror.trim().trim_end_matches('/');
    let rest = upstream.strip_prefix("https://github.com/")?;
    if root.is_empty() {
        return None;
    }
    Some(format!("{}/github.com/{}", root, rest))
}

/// 目的：选源——只有一个候选就直接用（没有选择时测速是白花的时间）；多个时各探 2 MB，取最快且可达的。
pub(crate) fn pick_source(
    agent: &ureq::Agent,
    urls: &[String],
    on_line: &mut dyn FnMut(&str),
) -> Result<String, String> {
    if urls.len() <= 1 {
        return urls
            .first()
            .cloned()
            .ok_or_else(|| "没有下载源".to_string());
    }
    on_line(&format!("测速选源（{} 个候选，各探 2 MB）...", urls.len()));
    let mut samples: Vec<(String, u16, f64)> = Vec::new();
    for u in urls {
        let (code, kbps) = probe_speed(agent, u);
        on_line(&format!("  {:.0} KB/s  [http {}]  {}", kbps, code, u));
        samples.push((u.clone(), code, kbps));
    }
    choose_fastest(&samples)
}

/// 目的：从探测结果里选源——2xx 且有数据的最快者；都不行时把状态码含义说清（不静默换源硬试）。
pub(crate) fn choose_fastest(samples: &[(String, u16, f64)]) -> Result<String, String> {
    let best = samples
        .iter()
        .filter(|(_, code, kbps)| (200..300).contains(code) && *kbps > 0.0)
        .max_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
    if let Some((url, _, _)) = best {
        return Ok(url.clone());
    }
    let codes: Vec<String> = samples.iter().map(|(_, c, _)| c.to_string()).collect();
    Err(format!(
        "没有可用的下载源（HTTP {}）：000 = 网络层失败（配 HTTPS_PROXY，或浏览器下载后放进缓存），403 = 被限流，404 = 该源没有这个资产",
        codes.join(" / ")
    ))
}

/// 目的：一次 2 MB 的范围 GET，回 (状态码, KB/s)；网络层失败记 000（与 curl 的观感一致）。
/// 约束：非 2xx 再试一次——受限网络上瞬时重置很常见，只探一次会把可达源误杀。
pub(crate) fn probe_speed(agent: &ureq::Agent, url: &str) -> (u16, f64) {
    let first = probe_speed_once(agent, url);
    if (200..300).contains(&first.0) {
        return first;
    }
    probe_speed_once(agent, url)
}

pub(crate) fn probe_speed_once(agent: &ureq::Agent, url: &str) -> (u16, f64) {
    let started = Instant::now();
    let resp = match agent.get(url).header("Range", "bytes=0-2097151").call() {
        Ok(r) => r,
        Err(_) => return (0, 0.0),
    };
    let code = resp.status().as_u16();
    if !(200..300).contains(&code) {
        return (code, 0.0);
    }
    let mut reader = resp.into_body().into_reader().take(2 * 1024 * 1024);
    let bytes = io::copy(&mut reader, &mut io::sink()).unwrap_or(0);
    let secs = started.elapsed().as_secs_f64().max(0.001);
    (code, bytes as f64 / 1024.0 / secs)
}

/// 目的：分片路径（`.part`）——下载先写它再改名，半截文件不会被当成缓存。
pub(crate) fn part_path(dest: &Path) -> PathBuf {
    dest.with_extension("part")
}

/// 目的：分片的来源标记——续传必须打向产出这份分片的那条源（换源续等于把两份字节接起来）。
pub(crate) fn part_source_marker(part: &Path) -> PathBuf {
    let mut s = part.as_os_str().to_os_string();
    s.push(".src");
    PathBuf::from(s)
}

/// 目的：带续传的下载——`.part` 在场且来源标记与本次 URL 一致就从断点续（Range）；
///   来源不明 / 换了源就从头下；服务端忽略 Range（回 200）时从零重写。
/// 约束：失败时保留分片与来源标记（续传证据），成功才改名并删标记。
pub(crate) fn download_with_resume(
    agent: &ureq::Agent,
    url: &str,
    dest: &Path,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let part = part_path(dest);
    let marker = part_source_marker(&part);
    let same_source = fs::read_to_string(&marker)
        .map(|s| s.trim() == url)
        .unwrap_or(false);
    let offset = if part.is_file() {
        if same_source {
            fs::metadata(&part).map(|m| m.len()).unwrap_or(0)
        } else {
            let _ = fs::remove_file(&part);
            0
        }
    } else {
        0
    };
    if offset == 0 {
        let _ = fs::remove_file(&marker);
    }
    fs::write(&marker, url).map_err(|e| format!("写来源标记 {} 失败：{}", marker.display(), e))?;
    let mut req = agent.get(url);
    if offset > 0 {
        req = req.header("Range", format!("bytes={}-", offset));
    }
    let resp = match req.call() {
        Ok(r) => r,
        Err(e) => return Err(format!("下载失败（{}）：{}（分片保留，重跑续传）", url, e)),
    };
    let code = resp.status().as_u16();
    if !(200..300).contains(&code) {
        return Err(format!(
            "下载 {} 返回 {}（分片保留，重跑续传；要换源先删分片与来源标记 {}）",
            url,
            code,
            part.display()
        ));
    }
    let append = offset > 0 && code == 206;
    let mut opts = fs::OpenOptions::new();
    opts.create(true).write(true);
    if append {
        opts.append(true);
    } else {
        opts.truncate(true);
    }
    let mut f = opts
        .open(&part)
        .map_err(|e| format!("建 {} 失败：{}", part.display(), e))?;
    let mut reader = resp.into_body().into_reader();
    let mut added = 0u64;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let got = reader
            .read(&mut buf)
            .map_err(|e| format!("读下载流失败（{}）：{}", url, e))?;
        if got == 0 {
            break;
        }
        io::Write::write_all(&mut f, &buf[..got])
            .map_err(|e| format!("写 {} 失败：{}", part.display(), e))?;
        added += got as u64;
    }
    drop(f);
    fs::rename(&part, dest).map_err(|e| format!("改名 {} 失败：{}", part.display(), e))?;
    let _ = fs::remove_file(&marker);
    if append {
        on_line(&format!(
            "续传完成：断点 {} 字节 + 新增 {} 字节",
            offset, added
        ));
    } else {
        on_line(&format!("下载完成：{} 字节", added));
    }
    Ok(())
}
/// 目的：一个文件的 sha256（小写十六进制）——依赖补齐的完整性判据。
/// 约束：复用 workspace 的 `content_hash`（项目里唯一那份零依赖 SHA-256，见其模块注释：
///   "不引入加密库（项目依赖刻意保持最小）"）。整份读进内存是一次性的补齐全动作，最多几百 MB——
///   宁可这样，也不引第二套算法或加密库。
pub(crate) fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("读不了 {}：{}", path.display(), e))?;
    Ok(crate::capabilities::workspace::api::content_hash(&bytes))
}
/// 目的：取一个目录页的纯文本（列 Debian 包名用）。
pub(crate) fn get_text(agent: &ureq::Agent, url: &str) -> Result<String, String> {
    let resp = agent
        .get(url)
        .call()
        .map_err(|e| format!("取 {} 失败：{}", url, e))?;
    if !resp.status().is_success() {
        return Err(format!("取 {} 返回 {}", url, resp.status().as_u16()));
    }
    let mut body = resp.into_body();
    body.read_to_string()
        .map_err(|e| format!("读 {} 响应失败：{}", url, e))
}
