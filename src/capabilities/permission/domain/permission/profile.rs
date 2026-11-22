//! 目的：只有网络的两档档位——档位预设、全局默认、一层覆盖与会话 meta 的权限段。
//! 管：`ProfileId` / `ProfileSpec` / `PermissionDefaults` / `PermissionLayer` / `SessionSandbox`。
//! 不管：生效态（`base.rs`）、共享区范围（workspace 的 `scope.rs`）。
//! 联动：设置与会话 meta 直接反序列化成这些类型，`env::access` 把它们合并成生效态（docs/permission/README.md）。

use serde::{Deserialize, Serialize};

use super::*;

/// 目的：两档用户可切换的会话网络档位（名称是呈现层文案，id 是契约）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProfileId {
    /// 目的：不出站（默认）。
    #[default]
    Offline,
    /// 目的：允许出站。
    Online,
}

impl ProfileId {
    /// 目的：档位宽窄排序（收窄校验用；子会话不能换更宽的档）。
    pub fn rank(self) -> u8 {
        match self {
            ProfileId::Offline => 0,
            ProfileId::Online => 1,
        }
    }

    /// 目的：稳定 id（配置、状态栏与审计）。
    pub fn as_str(self) -> &'static str {
        match self {
            ProfileId::Offline => "offline",
            ProfileId::Online => "online",
        }
    }

    /// 目的：用户标签。
    pub fn label(self) -> &'static str {
        match self {
            ProfileId::Offline => "离线",
            ProfileId::Online => "联网",
        }
    }

    /// 目的：一句话说明这个档位能做什么。
    pub fn summary(self) -> &'static str {
        match self {
            ProfileId::Offline => "沙箱、模块目录与主机只读根照常可用；容器不许出站。",
            ProfileId::Online => "在离线之上放行出站网络；放行与否仍受全局网络开关约束。",
        }
    }

    /// 目的：从配置串解析档位（只认两个 id）。
    pub fn parse(raw: &str) -> Result<ProfileId, String> {
        match raw.trim() {
            "offline" => Ok(ProfileId::Offline),
            "online" => Ok(ProfileId::Online),
            other => Err(format!("未知权限档位：{}（只认 offline / online）", other)),
        }
    }

    /// 目的：两档按从窄到宽排列（配置界面与校验共用）。
    pub fn all() -> [ProfileId; 2] {
        [ProfileId::Offline, ProfileId::Online]
    }

    /// 目的：这个档位的预设。
    pub fn spec(self) -> ProfileSpec {
        ProfileSpec {
            id: self,
            label: self.label(),
            summary: self.summary(),
            net: self == ProfileId::Online,
        }
    }
}

/// 目的：一档 profile 的完整预设（配置界面直接照抄，不在前端另写一份）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProfileSpec {
    /// 目的：档位 id。
    pub id: ProfileId,
    /// 目的：用户标签。
    pub label: &'static str,
    /// 目的：一句话说明。
    pub summary: &'static str,
    /// 目的：这一档是否放行出站。
    pub net: bool,
}

/// 目的：全局默认权限（设置里的 sandbox.defaults）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionDefaults {
    /// 目的：默认档位。
    #[serde(default)]
    pub profile: ProfileId,
}

impl Default for PermissionDefaults {
    fn default() -> Self {
        PermissionDefaults {
            profile: ProfileId::Offline,
        }
    }
}

impl PermissionDefaults {
    /// 目的：把全局默认变成一份生效权限。
    pub fn to_permission_set(&self) -> PermissionSet {
        PermissionSet {
            profile: self.profile,
        }
    }
}

/// 目的：一层会话权限（根 ceiling 或子 narrowing）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionLayer {
    /// 目的：档位；根 = 选择预设，子 = 只能换更严的档。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<ProfileId>,
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
