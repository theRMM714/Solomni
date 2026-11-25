//! 目的：端点候选、重试判定与尝试归类的纯逻辑（无 IO）。
//! 管：chat / models 候选序列、补全与回落规则、进程内端点记忆、可重试状态判定。
//! 不管：实际发请求（在 http_chat / model_catalog / http_probe）；协议形状与信封。
//! 联动：被 http_chat、model_catalog、http_probe 复用；无本能力之外的依赖。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// 目的：端点记忆：key（供应商 base_url + 用途）→ 已验证可用的完整 URL。
pub type Memo = Arc<Mutex<HashMap<String, String>>>;

pub fn memo_new() -> Memo {
    Arc::new(Mutex::new(HashMap::new()))
}

/// 目的：取已固定的端点。
pub fn memo_get(memo: &Memo, key: &str) -> Option<String> {
    memo.lock().ok().and_then(|m| m.get(key).cloned())
}

/// 目的：固定一个已验证可用的端点。
pub fn memo_set(memo: &Memo, key: &str, url: &str) {
    if let Ok(mut m) = memo.lock() {
        m.insert(key.to_string(), url.to_string());
    }
}

/// 目的：一次出站尝试的归类：Ok=成功载荷；Retry=可换下一个候选；Fatal=立即报错。
pub enum Attempt<T> {
    Ok(T),
    Retry(String),
    Fatal(String),
}

/// 目的：按候选依次尝试：Ok 即返回 (命中端点, 载荷)；Retry 才换下一个（有则回调埋点）；Fatal 立即返回。
///    全部 Retry 耗尽 = 返回最后一个错误，不静默。
pub fn resolve_candidates<T, F>(
    candidates: &[String],
    mut try_one: F,
    mut on_retry: impl FnMut(&str, &str, &str),
) -> Result<(String, T), String>
where
    F: FnMut(&str) -> Attempt<T>,
{
    let last_index = candidates.len().saturating_sub(1);
    let mut last_err = String::new();
    for (i, url) in candidates.iter().enumerate() {
        match try_one(url) {
            Attempt::Ok(v) => return Ok((url.clone(), v)),
            Attempt::Fatal(e) => return Err(e),
            Attempt::Retry(e) => {
                last_err = e;
                if i < last_index {
                    on_retry(url, &last_err, &candidates[i + 1]);
                }
            }
        }
    }
    Err(last_err)
}

/// 目的：对话端点候选：{base}/chat/completions [→ {base}/v1/chat/completions]。
pub fn chat_candidates(base_url: &str) -> Vec<String> {
    candidates(base_url, "chat/completions")
}

/// 目的：模型清单端点候选：{base}/models [→ {base}/v1/models]。
pub fn models_candidates(base_url: &str) -> Vec<String> {
    candidates(base_url, "models")
}

/// 目的：换下一个候选的判定：只有 404/405 换；其余 HTTP 状态（含 401/403）立即报，避免无意义重试。
pub fn retryable_status(code: u16) -> bool {
    code == 404 || code == 405
}

fn candidates(base_url: &str, suffix: &str) -> Vec<String> {
    let base = base_url.trim_end_matches('/');
    if base.is_empty() {
        return vec![format!("/{}", suffix)];
    }
    if base.ends_with(&format!("/{}", suffix)) {
        return vec![base.to_string()]; // 已是完整端点：原样用，防重复拼接
    }
    let full = format!("{}/{}", base, suffix);
    if ends_with_version(base) {
        return vec![full]; // 已带版本段：只补后缀
    }
    vec![full, format!("{}/v1/{}", base, suffix)]
}

/// 末段形如 v1 / v2 / v1beta：视为已带版本段。
fn ends_with_version(base: &str) -> bool {
    let seg = base.rsplit('/').next().unwrap_or("");
    let mut chars = seg.chars();
    matches!(chars.next(), Some('v')) && chars.next().is_some_and(|c| c.is_ascii_digit())
}
