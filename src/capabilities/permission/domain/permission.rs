//! 权限的**纯规则**：决定粒度 + 工作区路径的白名单 / 黑名单 + 模块写授权。
//!
//! 语义（用户口径，唯一一份）：
//! - 默认：整棵工作区可读、可提交；
//! - **白名单一出现就取代默认**（配了 allow_read 就失去默认的整棵可读，配了 allow_write 就失去默认提交）；
//! - **黑名单只做减法，不让默认失效**；
//! - 两者重叠时**黑大于白**；
//! - 模块目录默认只读；`module_write` 命中该模块才整块可写（`<module>/userdata/` 恒可写，由执行点单独放行）。
//!
//! 归属：本文件只处理器**声明**→**生效态**的解析与判定，不认识会话、不碰文件系统。
//! 白/黑是呈现层词汇；解析后下游只见 `read_ok` / `write_ok` 一个正向判定。

use serde::{Deserialize, Serialize};

/// 决定粒度：`full` = 任何工具都不设确认（AI 自决）；`ask` = `ask` 表里的工具必须用户点是/否才执行。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Granularity {
    /// 询问：落在 `ask` 表里的工具调用要先经用户确认。
    #[default]
    Ask,
    /// 完全代理：不确认任何工具调用。
    Full,
}

/// 一份权限声明（全局默认，或已被覆盖后的完整生效态）。
/// 空的白名单 = 默认（整棵工作区）；空的 `deny` = 不禁止任何路径。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Permissions {
    /// 读白名单（相对工作区根；空 = 整棵工作区可读）。
    #[serde(default)]
    pub allow_read: Vec<String>,
    /// 提交白名单（相对工作区根；空 = 整棵工作区可提交）。
    #[serde(default)]
    pub allow_write: Vec<String>,
    /// 黑名单：读写都禁，只做减法（空 = 不禁止）。
    #[serde(default)]
    pub deny: Vec<String>,
    /// 模块目录写授权（模块 id；空 = 模块目录只读，`userdata/` 恒可写）。
    #[serde(default)]
    pub module_write: Vec<String>,
    /// 决定粒度。
    #[serde(default)]
    pub granularity: Granularity,
    /// 需要用户确认的工具名（`granularity=ask` 时生效；模块工具写 `module.tool`）。
    #[serde(default)]
    pub ask: Vec<String>,
}

impl Default for Permissions {
    fn default() -> Self {
        Permissions {
            allow_read: Vec::new(),
            allow_write: Vec::new(),
            deny: Vec::new(),
            module_write: Vec::new(),
            granularity: Granularity::Ask,
            ask: Vec::new(),
        }
    }
}

/// 会话 / 逐 agent 的**覆盖**：只覆盖显式给出的字段（None = 继承上一层）。
/// 为什么用 Option 而不是空集合：空白名单有确切语义（= 默认整棵），与"没给"必须分得开。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionsOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_read: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_write: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deny: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_write: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granularity: Option<Granularity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<Vec<String>>,
}

impl Permissions {
    /// 叠加一层覆盖：只替换它显式给出的字段。
    pub fn apply(&self, over: &PermissionsOverride) -> Permissions {
        let mut out = self.clone();
        if let Some(v) = &over.allow_read {
            out.allow_read = v.clone();
        }
        if let Some(v) = &over.allow_write {
            out.allow_write = v.clone();
        }
        if let Some(v) = &over.deny {
            out.deny = v.clone();
        }
        if let Some(v) = &over.module_write {
            out.module_write = v.clone();
        }
        if let Some(g) = over.granularity {
            out.granularity = g;
        }
        if let Some(v) = &over.ask {
            out.ask = v.clone();
        }
        out
    }

    /// 一条工作区相对路径能不能读。
    pub fn read_ok(&self, rel: &str) -> bool {
        self.allowed(rel, &self.allow_read)
    }

    /// 一条工作区相对路径能不能提交。
    pub fn write_ok(&self, rel: &str) -> bool {
        self.allowed(rel, &self.allow_write)
    }

    /// 白/黑的唯一判定：黑名单先否决；白名单非空时只认白名单；否则是默认（放行）。
    fn allowed(&self, rel: &str, allow: &[String]) -> bool {
        if hit(&self.deny, rel) {
            return false;
        }
        if allow.is_empty() {
            return true;
        }
        hit(allow, rel)
    }

    /// 这个模块的目录能不能整块写（内置工具与围栏共用）。
    pub fn module_write_ok(&self, module: &str) -> bool {
        self.module_write.iter().any(|m| m.trim() == module)
    }

    /// 这个工具调用要不要用户确认（`full` 一律不确认；`ask` 表里命中才要）。
    pub fn should_ask(&self, tool: &str) -> bool {
        self.granularity == Granularity::Ask && self.ask.iter().any(|t| t.trim() == tool)
    }

    /// 允许的写根清单（拒绝文案里列全，模型照着改）。
    pub fn write_roots_listing(&self) -> String {
        listing(&self.allow_write)
    }

    /// 允许的读根清单。
    pub fn read_roots_listing(&self) -> String {
        listing(&self.allow_read)
    }
}

/// 命中判定：`.`（或空）代表整棵工作区；其余按**路径组件**匹配目录子树，不做字符串前缀
/// （否则 `web` 会误匹配 `website`）。
pub fn hit(list: &[String], rel: &str) -> bool {
    let rel = normalize(rel);
    list.iter().any(|entry| {
        let e = normalize(entry);
        if e.is_empty() || e == "." {
            return true;
        }
        rel == e || rel.starts_with(&format!("{}/", e))
    })
}

/// 统一书写形式：去首尾空白与斜杠、反斜杠归一成 `/`。
pub fn normalize(s: &str) -> String {
    s.trim().replace('\\', "/").trim_matches('/').to_string()
}

fn listing(entries: &[String]) -> String {
    if entries.is_empty() {
        return "（整棵工作区）".to_string();
    }
    entries
        .iter()
        .map(|e| normalize(e))
        .collect::<Vec<_>>()
        .join("、")
}

/// 校验一条工作区相对路径条目：非空、相对、无 `..` 与空段；`.` 合法（整棵）。
pub fn validate_entry(entry: &str) -> Result<(), String> {
    let raw = entry.trim();
    if raw.is_empty() {
        return Err("路径条目不能为空".to_string());
    }
    let flat = raw.replace('\\', "/");
    if flat.starts_with('/') || flat.starts_with('\\') {
        return Err(format!("路径条目必须是相对工作区根的路径：{}", raw));
    }
    // Windows 盘符形式（C:...）也要挡掉：它不是相对路径。
    let bytes = flat.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && (bytes[0] as char).is_ascii_alphabetic() {
        return Err(format!("路径条目必须是相对工作区根的路径：{}", raw));
    }
    for seg in flat.split('/') {
        if seg.is_empty() {
            return Err(format!("路径条目含空段（连续分隔符）：{}", raw));
        }
        if seg == ".." {
            return Err(format!("路径条目不能含 .. 段：{}", raw));
        }
    }
    Ok(())
}

/// 校验一层**覆盖**里显式给出的条目（只有给出的才校验）。
pub fn validate_override(o: &PermissionsOverride) -> Result<(), String> {
    let mut probe = Permissions::default();
    if let Some(v) = &o.allow_read {
        probe.allow_read = v.clone();
    }
    if let Some(v) = &o.allow_write {
        probe.allow_write = v.clone();
    }
    if let Some(v) = &o.deny {
        probe.deny = v.clone();
    }
    if let Some(v) = &o.module_write {
        probe.module_write = v.clone();
    }
    validate(&probe)?;
    if let Some(list) = &o.ask {
        for t in list {
            if t.trim().is_empty() {
                return Err("ask 里的工具名不能为空".to_string());
            }
        }
    }
    Ok(())
}

/// 校验整份声明：路径条目合法、模块 id 非空。
pub fn validate(p: &Permissions) -> Result<(), String> {
    for (label, list) in [
        ("allow_read", &p.allow_read),
        ("allow_write", &p.allow_write),
        ("deny", &p.deny),
    ] {
        for e in list.iter() {
            validate_entry(e).map_err(|why| format!("{}：{}", label, why))?;
        }
    }
    for m in &p.module_write {
        let id = m.trim();
        if id.is_empty() {
            return Err("module_write 里的模块 id 不能为空".to_string());
        }
        if id.contains('/') || id.contains('\\') || id == ".." {
            return Err(format!("module_write 里的模块 id 不合法：{}", m));
        }
    }
    Ok(())
}
