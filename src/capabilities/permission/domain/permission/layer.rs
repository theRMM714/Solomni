//! 目的：权限的声明层——全局默认、一层会话覆盖、会话 meta 的权限段，以及单次调用的 scope 请求。
//! 管：`CallScope` / `PermissionDefaults` / `PermissionLayer` / `SessionSandbox` 及各自的形状校验。
//! 不管：求交后的生效态（`PermissionSet`，在 `set.rs`）；路径条目的命中与合法性（`path.rs`）。
//! 联动：设置与会话 meta 直接反序列化成这些类型，`set.rs` 把它们求交成生效态（docs/permission/README.md）。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::*;

/// 目的：一次调用的 scope 请求（只对声明了 scope 的执行类工具有意义）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallScope {
    /// 目的：非执行类工具：没有 scope。
    None,
    /// 目的：只碰私有沙箱。
    Sandbox,
    /// 目的：沙箱之外再加共享区 / 模块目录 / 主机只读根 / 网络（私有沙箱仍保留）。
    External,
}

impl CallScope {
    /// 目的：解析模型给的 scope 参数；省略 = sandbox。
    pub fn from_request(raw: Option<&str>) -> Result<CallScope, String> {
        match raw.map(str::trim).filter(|s| !s.is_empty()) {
            None | Some("sandbox") => Ok(CallScope::Sandbox),
            Some("external") => Ok(CallScope::External),
            Some(other) => Err(format!("scope 只认 sandbox / external，收到：{}", other)),
        }
    }

    /// 目的：这个 scope 请求对应的 reach。
    pub fn reach(self) -> ReachSet {
        match self {
            CallScope::None => ReachSet::empty(),
            CallScope::Sandbox => ReachSet::from([Reach::Private]),
            CallScope::External => ReachSet::from([
                Reach::Private,
                Reach::SharedRead,
                Reach::ModuleRead,
                Reach::HostRead,
                Reach::Network,
            ]),
        }
    }
}

/// 目的：全局默认权限（设置里的 sandbox.defaults）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionDefaults {
    /// 目的：默认档位。
    #[serde(default)]
    pub profile: ProfileId,
    /// 目的：默认共享区范围。
    #[serde(default)]
    pub shared: SharedScope,
    /// 目的：默认逐工具决定（工具名 → decision）。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tools: BTreeMap<String, Decision>,
}

impl Default for PermissionDefaults {
    fn default() -> Self {
        PermissionDefaults {
            profile: ProfileId::Workspace,
            shared: SharedScope::default(),
            tools: BTreeMap::new(),
        }
    }
}

impl PermissionDefaults {
    /// 目的：把全局默认变成一份生效权限（不含全局硬黑名单）。
    pub fn to_permission_set(&self) -> PermissionSet {
        let spec = self.profile.spec();
        PermissionSet {
            profile: self.profile,
            reach: spec.reach,
            decisions: spec.decisions,
            shared_read: self.shared.read.clone(),
            shared_write: self.shared.write.clone(),
            deny: self.shared.deny.clone(),
            tools: self.tools.clone(),
        }
    }
}

/// 目的：一层会话权限（根 ceiling 或子 narrowing）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionLayer {
    /// 目的：档位；根 = 选择预设，子 = 只能换更低档。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<ProfileId>,
    /// 目的：reach 收窄；子必须是父的子集。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reach: Option<ReachSet>,
    /// 目的：共享区范围。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared: Option<SharedEdit>,
    /// 目的：decision 覆盖；子只能更严。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decisions: Option<DecisionEdit>,
    /// 目的：逐工具决定；子会话不能再改（避免绕过 tool effect 的校验）。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tools: BTreeMap<String, Decision>,
}

impl PermissionLayer {
    /// 目的：校验条目形状（不判断父子宽窄）。
    pub fn validate(&self) -> Result<(), String> {
        if let Some(shared) = &self.shared {
            if let Some(v) = &shared.read {
                v.validate("shared.read")?;
            }
            if let Some(v) = &shared.write {
                v.validate("shared.write")?;
            }
            for e in &shared.deny {
                validate_entry(e).map_err(|why| format!("shared.deny：{}", why))?;
            }
        }
        for name in self.tools.keys() {
            if name.trim().is_empty() {
                return Err("tools 里的工具名不能为空".to_string());
            }
        }
        Ok(())
    }
}

/// 目的：会话 meta 的权限段——根只有 ceiling，子只有 narrowing。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionSandbox {
    /// 目的：根会话的上限。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ceiling: Option<PermissionLayer>,
    /// 目的：子会话的收窄层。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narrowing: Option<PermissionLayer>,
}

impl SessionSandbox {
    /// 目的：取根会话的 ceiling；子会话带了 ceiling = 非法。
    pub fn root_layer(&self) -> Result<Option<&PermissionLayer>, String> {
        if self.narrowing.is_some() {
            return Err("根会话的 sandbox 不能有 narrowing".to_string());
        }
        Ok(self.ceiling.as_ref())
    }

    /// 目的：取子会话的 narrowing；根会话带了 narrowing = 非法。
    pub fn child_layer(&self) -> Result<Option<&PermissionLayer>, String> {
        if self.ceiling.is_some() {
            return Err("子会话的 sandbox 不能有 ceiling".to_string());
        }
        Ok(self.narrowing.as_ref())
    }
}
