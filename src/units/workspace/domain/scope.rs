//! 目的：共享区的路径归属——默认范围、一层覆盖、生效态与黑名单命中。
//! 管：`ScopeMode`（含手写 serde：`all` / `none` / 路径数组）、`SharedScope`（生效态）、`SharedEdit`（一层覆盖），
//!   以及条目写法与命中规则（`hit` / `normalize` / `validate_entry`）。
//! 不管：档位与网络（在 `permission`）；读写落点的寻址（在 `workspace`）；版本库如何提交（在 `service/`）。
//! 联动：`Sandbox.scope` 持生效态，内置文件工具与 `work_*` 都只经它判共享区归属；
//!   全局默认在 `catalog` 的 `SandboxSettings.scope`，会话覆盖在 `SessionMeta.shared`。

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// 目的：共享区的路径范围三态；空集合不再兼作整棵。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeMode {
    /// 目的：整棵共享区。
    All,
    /// 目的：显式无权限。
    None,
    /// 目的：只允许这些子树。
    Paths(Vec<String>),
}

impl ScopeMode {
    /// 目的：一条共享区相对路径在不在范围内。
    pub fn allows(&self, rel: &str) -> bool {
        match self {
            ScopeMode::All => true,
            ScopeMode::None => false,
            ScopeMode::Paths(paths) => hit(paths, rel),
        }
    }

    /// 目的：许可根清单（拒绝文案与配置视图）。
    pub fn listing(&self) -> String {
        match self {
            ScopeMode::All => "（整棵共享区）".to_string(),
            ScopeMode::None => "（无）".to_string(),
            ScopeMode::Paths(paths) => listing(paths),
        }
    }

    /// 目的：两项的交集；没有交集 = 显式无权限。
    pub fn intersect(&self, other: &ScopeMode) -> ScopeMode {
        match (self, other) {
            (ScopeMode::None, _) | (_, ScopeMode::None) => ScopeMode::None,
            (ScopeMode::All, x) | (x, ScopeMode::All) => x.clone(),
            (ScopeMode::Paths(a), ScopeMode::Paths(b)) => {
                let mut out: Vec<String> = Vec::new();
                for x in a {
                    for y in b {
                        if overlaps(x, y) {
                            let p = narrower(x, y);
                            if !out.iter().any(|e| normalize(e) == p) {
                                out.push(p);
                            }
                        }
                    }
                }
                out.sort();
                if out.is_empty() {
                    ScopeMode::None
                } else {
                    ScopeMode::Paths(out)
                }
            }
        }
    }

    /// 目的：自己是不是 parent 的子集（子会话收窄校验用）。
    pub fn is_subset_of(&self, parent: &ScopeMode) -> bool {
        match (self, parent) {
            (_, ScopeMode::All) => true,
            (ScopeMode::None, _) => true,
            (ScopeMode::All, _) => false,
            (ScopeMode::Paths(_), ScopeMode::None) => false,
            (ScopeMode::Paths(mine), ScopeMode::Paths(theirs)) => mine.iter().all(|p| {
                let p = normalize(p);
                theirs.iter().any(|q| {
                    let q = normalize(q);
                    q.is_empty() || q == "." || p == q || p.starts_with(&format!("{}/", q))
                })
            }),
        }
    }

    /// 目的：校验路径条目。
    pub fn validate(&self, label: &str) -> Result<(), String> {
        if let ScopeMode::Paths(paths) = self {
            for e in paths {
                validate_entry(e).map_err(|why| format!("{}：{}", label, why))?;
            }
        }
        Ok(())
    }
}

fn default_all() -> ScopeMode {
    ScopeMode::All
}

impl Serialize for ScopeMode {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            ScopeMode::All => s.serialize_str("all"),
            ScopeMode::None => s.serialize_str("none"),
            ScopeMode::Paths(paths) => paths.serialize(s),
        }
    }
}

impl<'de> Deserialize<'de> for ScopeMode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<ScopeMode, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = ScopeMode;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("all / none / 路径数组")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<ScopeMode, E> {
                match v {
                    "all" => Ok(ScopeMode::All),
                    "none" => Ok(ScopeMode::None),
                    other => Err(E::custom(format!(
                        "共享区范围只认 all / none / 路径数组，收到：{}",
                        other
                    ))),
                }
            }
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<ScopeMode, A::Error> {
                let mut paths = Vec::new();
                while let Some(v) = seq.next_element::<String>()? {
                    paths.push(v);
                }
                Ok(ScopeMode::Paths(paths))
            }
        }
        d.deserialize_any(V)
    }
}

/// 目的：一份生效的共享区归属：读范围、写范围、黑名单。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedScope {
    /// 目的：读范围。
    #[serde(default = "default_all")]
    pub read: ScopeMode,
    /// 目的：写范围（提交的准入门槛）。
    #[serde(default = "default_all")]
    pub write: ScopeMode,
    /// 目的：黑名单（全局硬底线 + 会话追加）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny: Vec<String>,
}

impl Default for SharedScope {
    fn default() -> Self {
        SharedScope {
            read: ScopeMode::All,
            write: ScopeMode::All,
            deny: Vec::new(),
        }
    }
}

impl SharedScope {
    /// 目的：共享区一条相对路径能不能读。
    pub fn read_ok(&self, rel: &str) -> bool {
        scope_ok(&self.read, &self.deny, rel)
    }

    /// 目的：共享区一条相对路径能不能提交。
    pub fn write_ok(&self, rel: &str) -> bool {
        scope_ok(&self.write, &self.deny, rel)
    }

    /// 目的：读范围清单（拒绝文案用）。
    pub fn read_roots_listing(&self) -> String {
        self.read.listing()
    }

    /// 目的：写范围清单（拒绝文案用）。
    pub fn write_roots_listing(&self) -> String {
        self.write.listing()
    }

    /// 目的：这一席能不能读共享区（只回答"这一面开着没有"，逐路径还要过 `read_ok`）。
    pub fn may_read(&self) -> bool {
        !matches!(self.read, ScopeMode::None)
    }

    /// 目的：这一席能不能提交到共享区（只回答"这一面开着没有"，逐路径还要过 `write_ok`）。
    pub fn may_commit(&self) -> bool {
        !matches!(self.write, ScopeMode::None)
    }

    /// 目的：应用根会话的覆盖（读 / 写范围整体替换，黑名单追加）。
    pub fn apply_root(&self, edit: &SharedEdit) -> Result<SharedScope, String> {
        let mut out = self.clone();
        if let Some(v) = &edit.read {
            out.read = v.clone();
        }
        if let Some(v) = &edit.write {
            out.write = v.clone();
        }
        append_deny(&mut out.deny, &edit.deny);
        out.validate()?;
        Ok(out)
    }

    /// 目的：应用子会话的收窄（只能收窄，黑名单照旧追加）。
    pub fn apply_narrowing(&self, edit: &SharedEdit) -> Result<SharedScope, String> {
        let mut out = self.clone();
        if let Some(v) = &edit.read {
            if !v.is_subset_of(&out.read) {
                return Err("子会话的共享区读范围只能收窄".to_string());
            }
            out.read = out.read.intersect(v);
        }
        if let Some(v) = &edit.write {
            if !v.is_subset_of(&out.write) {
                return Err("子会话的共享区写范围只能收窄".to_string());
            }
            out.write = out.write.intersect(v);
        }
        append_deny(&mut out.deny, &edit.deny);
        out.validate()?;
        Ok(out)
    }

    /// 目的：并上全局硬黑名单。
    pub fn with_hard_deny(mut self, deny: &[String]) -> SharedScope {
        append_deny(&mut self.deny, deny);
        self
    }

    /// 目的：校验范围与黑名单条目的写法。
    pub fn validate(&self) -> Result<(), String> {
        self.read.validate("shared.read")?;
        self.write.validate("shared.write")?;
        for e in &self.deny {
            validate_entry(e).map_err(|why| format!("shared.deny：{}", why))?;
        }
        Ok(())
    }
}

/// 目的：一层会话对共享区归属的覆盖；None = 不覆盖，Some(None) = 显式无权限。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedEdit {
    /// 目的：读范围。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<ScopeMode>,
    /// 目的：写范围。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<ScopeMode>,
    /// 目的：追加黑名单。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny: Vec<String>,
}

/// 目的：命中判定：点（或空）代表整棵共享区；其余按路径组件匹配目录子树。
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

/// 目的：统一书写形式：去首尾空白与斜杠、反斜杠归一成斜杠。
pub fn normalize(s: &str) -> String {
    s.trim().replace('\\', "/").trim_matches('/').to_string()
}

/// 目的：把一组路径条目写成回执文案（空 = 整棵共享区）。
fn listing(entries: &[String]) -> String {
    if entries.is_empty() {
        return "（整棵共享区）".to_string();
    }
    entries
        .iter()
        .map(|e| normalize(e))
        .collect::<Vec<_>>()
        .join("、")
}

/// 目的：两条路径条目是否有交集（一条是另一条的前缀子树或同一条）。
fn overlaps(a: &str, b: &str) -> bool {
    let a = normalize(a);
    let b = normalize(b);
    if a.is_empty() || a == "." || b.is_empty() || b == "." {
        return true;
    }
    a == b || a.starts_with(&format!("{}/", b)) || b.starts_with(&format!("{}/", a))
}

/// 目的：两条路径条目里更窄的那一条。
fn narrower(a: &str, b: &str) -> String {
    let a = normalize(a);
    let b = normalize(b);
    if a.is_empty() || a == "." {
        return b;
    }
    if b.is_empty() || b == "." {
        return a;
    }
    let ac = a.split('/').count();
    let bc = b.split('/').count();
    if ac >= bc { a } else { b }
}

/// 目的：校验一条共享区相对路径条目：非空、相对、无上级段与空段；点合法（整棵）。
pub fn validate_entry(entry: &str) -> Result<(), String> {
    let raw = entry.trim();
    if raw.is_empty() {
        return Err("路径条目不能为空".to_string());
    }
    let flat = raw.replace('\\', "/");
    if flat.starts_with('/') || flat.starts_with('\\') {
        return Err(format!("路径条目必须是相对共享区根的路径：{}", raw));
    }
    let bytes = flat.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && (bytes[0] as char).is_ascii_alphabetic() {
        return Err(format!("路径条目必须是相对共享区根的路径：{}", raw));
    }
    for seg in flat.split('/') {
        if seg.is_empty() {
            return Err(format!("路径条目含空段（连续分隔符）：{}", raw));
        }
        if seg == ".." {
            return Err(format!("路径条目不能含上级段：{}", raw));
        }
    }
    Ok(())
}

fn scope_ok(scope: &ScopeMode, deny: &[String], rel: &str) -> bool {
    if hit(deny, rel) {
        return false;
    }
    scope.allows(rel)
}

fn append_deny(out: &mut Vec<String>, add: &[String]) {
    for entry in add {
        if !out.iter().any(|e| normalize(e) == normalize(entry)) {
            out.push(entry.clone());
        }
    }
}
