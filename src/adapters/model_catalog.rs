//! 模型目录适配器：OpenAI 兼容 GET {base_url}/models（实现 core 的 ModelCatalog 端口）。
//! 端点补全/回落规则见 endpoint 模块：无版本段先直连，404/405 再试 /v1。
//! 机制only：密钥只用于出站请求头；错误信息先脱敏再出适配层；响应形状不符即报错，不猜测兜底。

use super::endpoint::{memo_get, memo_set, models_candidates, resolve_candidates, retryable_status, Attempt, Memo};
use crate::core::ports::{Log, ModelCatalog};
use crate::core::providers::Provider;
use std::sync::Arc;

/// 真实模型目录：ureq 出站，独立超时（比对话短，配置期等待）。
pub struct HttpModelCatalog {
    log: Arc<dyn Log + Send + Sync>,
    memo: Memo,
}

impl HttpModelCatalog {
    /// 组合根注入日志端口（HTTP 失败落盘）与共享端点记忆。
    pub fn with_log(log: Arc<dyn Log + Send + Sync>, memo: Memo) -> HttpModelCatalog {
        HttpModelCatalog { log, memo }
    }

    /// 单次 GET：请求与解析都在此；失败按「可换候选 / 立即报」归类。
    fn fetch(&self, url: &str, provider: &Provider) -> Attempt<Vec<String>> {
        let agent = super::http_agent::agent(10, 30);
        let resp = match agent
            .get(url)
            .set("Authorization", &format!("Bearer {}", provider.api_key))
            .call()
        {
            Ok(r) => r,
            Err(ureq::Error::Status(code, resp)) => {
                let snippet = resp.into_string().unwrap_or_default();
                let snippet: String = snippet.chars().take(200).collect();
                let msg = redact(format!("供应商返回 {}：{}", code, snippet), &provider.api_key);
                return if retryable_status(code) { Attempt::Retry(msg) } else { Attempt::Fatal(msg) };
            }
            Err(other) => return Attempt::Retry(redact(format!("网络错误：{}", other), &provider.api_key)),
        };
        let body = match resp.into_string() {
            Ok(t) => t,
            Err(e) => return Attempt::Retry(redact(e.to_string(), &provider.api_key)),
        };
        match parse_models(&body) {
            Ok(models) => Attempt::Ok(models),
            // 2xx 但不是模型清单形状（SPA 回落的 HTML 首页等）：这里不是该 API，换下一个候选。
            Err(e) => Attempt::Retry(e),
        }
    }
}

impl ModelCatalog for HttpModelCatalog {
    fn list_models(&self, provider: &Provider) -> Result<Vec<String>, String> {
        let memo_key = format!("{}|models", provider.base_url);
        let candidates = match memo_get(&self.memo, &memo_key) {
            Some(url) => vec![url],
            None => models_candidates(&provider.base_url),
        };
        let outcome = resolve_candidates(
            &candidates,
            |url| self.fetch(url, provider),
            |url, err, next| {
                self.log.warn("model_catalog::list_models", &format!("端点 {} 不可用（{}），改试 {}", url, err, next));
            },
        );
        match outcome {
            Ok((url, models)) => {
                memo_set(&self.memo, &memo_key, &url);
                Ok(models)
            }
            Err(e) => {
                self.log.error("model_catalog::list_models", &format!("供应商 {} 全部候选端点失败：{}", provider.base_url, e));
                Err(e)
            }
        }
    }
}

/// 出站错误里的密钥一律替换掉再出适配层。
fn redact(s: String, key: &str) -> String {
    if key.is_empty() {
        s
    } else {
        s.replace(key, "***")
    }
}

/// 解析 OpenAI 兼容 /models 响应：取 data[].id，按出现顺序去重。
/// 形状不符或空列表 = 报错暴露，不静默兜底。
pub fn parse_models(body: &str) -> Result<Vec<String>, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| format!("响应不是 JSON：{}", e))?;
    let arr = v.get("data").and_then(|d| d.as_array()).ok_or_else(|| {
        let snippet: String = body.chars().take(200).collect();
        format!("响应缺少 data 数组：{}", snippet)
    })?;
    let mut out: Vec<String> = Vec::new();
    for m in arr {
        if let Some(id) = m.get("id").and_then(|x| x.as_str()) {
            if !id.is_empty() && !out.iter().any(|x| x == id) {
                out.push(id.to_string());
            }
        }
    }
    if out.is_empty() {
        return Err("供应商未返回任何模型".to_string());
    }
    Ok(out)
}
