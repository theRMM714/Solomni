//! 模型层：会话抽象 + 真实 HTTP 通道 + 假模型（mock 测试）。
//! DIP：编排流程只依赖 Chat 会话对象，不依赖任何具体供应商。

#[cfg(test)]
use crate::envelope;

/// 一条消息：role = system / user / assistant。
/// role/content 由会话拼装使用；complete() 消费整段消息列表。
#[derive(Debug, Clone)]
pub struct Msg {
    pub role: String,
    pub content: String,
}

impl Msg {
    pub fn system(content: impl Into<String>) -> Msg { Msg { role: "system".into(), content: content.into() } }
    pub fn user(content: impl Into<String>) -> Msg { Msg { role: "user".into(), content: content.into() } }
    pub fn assistant(content: impl Into<String>) -> Msg { Msg { role: "assistant".into(), content: content.into() } }
}

/// 一次补全的原始文本输出。
pub type Raw = String;

/// 拥有所有权的会话通道（装箱端口对象）。
pub type BoxedChat = Box<dyn Chat>;

/// Chat 会话端口：核心与一个智能体（或自身整理环节）的对话通道。
/// 上层（编排/会话/呈现）只认此端口；具体实现见 HttpChat / FakeChat。
pub trait Chat {
    fn complete(&mut self, messages: &[Msg]) -> Raw;
}

/// 真实通道：OpenAI 兼容 /chat/completions。
/// 拥有供应商副本（含密钥）；密钥只在出站调用里使用，永不落提示词/日志。
pub struct HttpChat {
    pub provider: crate::providers::Provider,
    pub model: String,
}

impl Chat for HttpChat {
    fn complete(&mut self, messages: &[Msg]) -> Raw {
        // 请求体：messages 数组；响应取 choices[0].message.content。
        // 用极小 JSON 拼装（依赖最小化）；消息内容统一做转义。
        let mut body = String::from("{\"model\":");
        body.push_str(&json_str(&self.model));
        body.push_str(",\"messages\":[");
        for (i, m) in messages.iter().enumerate() {
            if i > 0 { body.push(','); }
            body.push_str("{\"role\":");
            body.push_str(&json_str(&m.role));
            body.push_str(",\"content\":");
            body.push_str(&json_str(&m.content));
            body.push('}');
        }
        body.push_str("]}");
        let url = format!("{}/chat/completions", self.provider.base_url.trim_end_matches('/'));
        ureq_do(&url, &self.provider.api_key, &body)
            .unwrap_or_else(|e| format!("{{\"type\":\"ask\",\"text\":\"模型调用失败：{}\"}}", e))
    }
}

// 真实 HTTP：OpenAI 兼容 /chat/completions；错误信息只含状态与摘要，永不携带密钥。
fn ureq_do(url: &str, key: &str, body: &str) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(120))
        .build();
    let resp = agent
        .post(url)
        .set("Authorization", &format!("Bearer {}", key))
        .set("Content-Type", "application/json")
        .send_string(body)
        .map_err(|e| {
            // 红线：错误信息永不携带密钥（ureq 的 Bad Header 等错误会回显请求头）。
            let redact = |s: String| s.replace(key, "***");
            match e {
                ureq::Error::Status(code, resp) => {
                    let snippet = resp.into_string().unwrap_or_default();
                    let snippet: String = snippet.chars().take(200).collect();
                    redact(format!("供应商返回 {}：{}", code, snippet))
                }
                other => redact(format!("网络错误：{}", other)),
            }
        })?;
    let text = resp.into_string().map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("响应不是 JSON：{}", e))?;
    v.get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "响应缺少 choices[0].message.content".to_string())
}

fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"?\"".into())
}

/// 假模型：确定性脚本应答，用于全链路 mock 测试与无供应商演示（不依赖网络与真实密钥）。
/// 每个应答对应一个信封动词，按调用顺序弹出；脚本为空时默认同意。
pub struct FakeChat {
    pub script: Vec<String>,
    pub calls: Vec<Vec<Msg>>,
}

impl FakeChat {
    pub fn new(script: Vec<String>) -> FakeChat {
        FakeChat { script, calls: Vec::new() }
    }
    /// 构造一个 say 信封文本。
    pub fn say(text: &str) -> String {
        format!("{{\"type\":\"say\",\"text\":\"{}\"}}", text)
    }
    #[cfg(test)]
    pub fn verb_json(verb: &str, text: &str) -> String {
        format!("{{\"type\":\"{}\",\"text\":\"{}\"}}", verb, text)
    }
}

impl Chat for FakeChat {
    fn complete(&mut self, messages: &[Msg]) -> Raw {
        self.calls.push(messages.to_vec());
        if self.script.is_empty() {
            return envelope_reply(VerbKind::Agree, "（脚本已尽，默认同意）");
        }
        let next = self.script.remove(0);
        match next.split_once('|') {
            Some((v, t)) => envelope_reply(
                match v { "ask" => VerbKind::Ask, "leave" => VerbKind::Leave, "agree" => VerbKind::Agree, _ => VerbKind::Say },
                t,
            ),
            None => next,
        }
    }
}

enum VerbKind { Say, Ask, Leave, Agree }

fn envelope_reply(v: VerbKind, text: &str) -> String {
    let verb = match v { VerbKind::Say => "say", VerbKind::Ask => "ask", VerbKind::Leave => "leave", VerbKind::Agree => "agree" };
    format!("{{\"type\":\"{}\",\"text\":\"{}\"}}", verb, text)
}

#[cfg(test)]
/// 从原始输出提取 text（供假模型检查收到的上下文）。
pub fn last_text(raw: &str) -> String {
    envelope::parse(raw).text
}
