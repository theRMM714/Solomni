//! 目的：会话的权限链派生——一条会话的生效网络档位按「全局默认 → 根 ceiling → 每层 narrowing」算出来。
//! 管：`permissions`（沿链求交，子会话只能更窄）。
//! 不管：档位表本身（在 `permission`）、共享区归属（在工作台单元）、围栏怎么落（在 `tools` 与内核）。
//! 联动：`turn` 取好会话链与全局默认后调用；入参只收 `PermissionDefaults`，不整包收设置（否则多一条对登记处的依赖）。

use crate::units::business::session_model::domain::history::SessionMeta;
use crate::units::pure::permission::api::{PermissionDefaults, PermissionSet};

/// 目的：一条会话的生效网络档位：全局默认 → 根 ceiling → 每层 narrowing（子只能更窄）。
/// 参数：`chain` = 根在前的会话链；`defaults` = 设置里的 `sandbox.defaults`。
/// 错误：链为空报「会话权限链为空」；某层同时写 ceiling 与 narrowing 由 `permission` 如实拒绝。
pub fn permissions(
    chain: &[SessionMeta],
    defaults: &PermissionDefaults,
) -> Result<PermissionSet, String> {
    let root = chain.first().ok_or_else(|| "会话权限链为空".to_string())?;
    let mut out = defaults.to_permission_set();
    if let Some(layer) = root.sandbox.root_layer()? {
        out = out.apply_root(layer)?;
    }
    for child in chain.iter().skip(1) {
        if let Some(layer) = child.sandbox.child_layer()? {
            out = out.apply_narrowing(layer)?;
        }
    }
    Ok(out)
}
