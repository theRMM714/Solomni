//! 目的：共享区路径范围的声明形状——全局默认、一层覆盖、三态收窄。
//! 管：`ScopeMode`（含手写 serde：数组 / null / 空三类形态写回）、`SharedScope` / `SharedEdit` / `DecisionEdit`。
//! 不管：命中的实际判定（`path.rs` 的 `hit` 与 `normalize`）；生效态的求交在 `set.rs`。
//! 联动：`PermissionDefaults` / `PermissionLayer` 内嵌这些类型，设置与会话 meta 按此落盘（docs/permission/README.md）。

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

use super::*;

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

/// 目的：全局默认的共享区范围（读写必须显式给出，空数组不再兼作整棵）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedScope {
    /// 目的：读范围。
    #[serde(default = "default_all")]
    pub read: ScopeMode,
    /// 目的：写范围。
    #[serde(default = "default_all")]
    pub write: ScopeMode,
    /// 目的：追加黑名单。
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

/// 目的：一层会话覆盖的共享区范围；None = 不覆盖，Some(None) = 显式无权限。
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

/// 目的：一层显式 decision 覆盖。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionEdit {
    /// 目的：读类。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<Decision>,
    /// 目的：写类。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<Decision>,
    /// 目的：执行类、sandbox scope。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exec_sandbox: Option<Decision>,
    /// 目的：执行类、external scope。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exec_external: Option<Decision>,
}
