//! 目的：一份生效权限——只剩网络开关；文件面由围栏按固定规则推导，共享区路径归属在 workspace。
//! 管：`PermissionSet` 的构造与父子两层合并（根可换档，子只能更严）。
//! 不管：声明形状（`profile.rs`）、共享区范围（units/business/workspace/domain/scope.rs）。
//! 联动：workspace 装配 `Sandbox`、tools 派生围栏都读它（docs/permission/README.md）。

use serde::Serialize;

use super::*;

/// 目的：一份生效权限：只有网络开关。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PermissionSet {
    /// 目的：生效档位（离线 / 联网）。
    pub profile: ProfileId,
}

impl Default for PermissionSet {
    fn default() -> PermissionSet {
        PermissionDefaults::default().to_permission_set()
    }
}

impl PermissionSet {
    /// 目的：这一席能不能出站联网（围栏的 net 开关问这一条）。
    pub fn allows_network(&self) -> bool {
        self.profile == ProfileId::Online
    }

    /// 目的：应用根会话的 ceiling（换档）。
    pub fn apply_root(&self, layer: &PermissionLayer) -> Result<PermissionSet, String> {
        let mut out = self.clone();
        if let Some(profile) = layer.profile {
            out.profile = profile;
        }
        Ok(out)
    }

    /// 目的：应用子会话的 narrowing（只能换更严的档）。
    pub fn apply_narrowing(&self, layer: &PermissionLayer) -> Result<PermissionSet, String> {
        let mut out = self.clone();
        if let Some(profile) = layer.profile {
            if profile.rank() > self.profile.rank() {
                return Err("子会话不能选择比父会话更宽的档位".to_string());
            }
            out.profile = profile;
        }
        Ok(out)
    }
}
