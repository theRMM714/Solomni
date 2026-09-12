//! 提示词渲染层：{{key}} 占位替换，纯逻辑。
//! 册子文本来自 PromptSource（文件机制在适配层）；缺键/缺变量报错，不静默。
//! 提示词是最不稳定的文本：改文案只动 prompts.yaml，不改代码。

use serde::Deserialize;

/// 渲染时的变量表。
pub type Vars<'a> = &'a [(&'a str, String)];

/// 渲染单段提示词：替换 {{name}}；遇到未提供的变量 = 错误（如实，不静默）。
pub fn render(template: &str, vars: Vars) -> Result<String, String> {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            if let Some(end_rel) = template[i + 2..].find("}}") {
                let key = template[i + 2..i + 2 + end_rel].trim();
                let value = vars
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| v.clone())
                    .ok_or_else(|| format!("提示词变量缺失：{}", key))?;
                out.push_str(&value);
                i += 2 + end_rel + 2;
                continue;
            }
        }
        // 字面 {{（非占位）按原样保留；JSON 示例中的花括号不受影响（单括号）。
        let ch_len = template[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        out.push_str(&template[i..i + ch_len]);
        i += ch_len;
    }
    Ok(out)
}

/// 提示词册（prompts.yaml 的内存形态）。
#[derive(Debug, Clone, Deserialize)]
pub struct Prompts {
    pub core: CorePrompts,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CorePrompts {
    pub chat_protocol: String,
    pub discuss: DiscussPrompts,
    pub synthesize: SynthPrompts,
    pub execute: ExecutePrompts,
    pub review: ReviewPrompts,
    pub rerun: RerunPrompts,
    pub slate: SlatePrompts,
    pub omni: OmniPrompts,
    pub module_system: String,
    pub module_system_tools: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiscussPrompts {
    /// opener 变量：protocol, task
    pub opener: String,
    /// step 变量：transcript
    pub step: String,
    pub autonomy_note: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SynthPrompts {
    pub system: String,
    /// user 变量：transcript
    pub user: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExecutePrompts {
    /// user 变量：tasks
    pub user: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReviewPrompts {
    pub system: String,
    /// user 变量：plan, reports
    pub user: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RerunPrompts {
    /// user 变量：tasks, review, report
    pub user: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlatePrompts {
    pub system: String,
    /// user 变量：modules, task
    pub user: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OmniPrompts {
    /// system 变量：modules
    pub system: String,
}

impl Prompts {
    pub fn render<'a>(&self, template: &str, vars: Vars<'a>) -> String {
        render(template, vars).expect("提示词渲染失败：变量缺失属于装配错误，须修复 prompts.yaml 或调用方")
    }
}