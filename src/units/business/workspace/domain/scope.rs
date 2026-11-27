//! 目的：把会话落盘的策略参数解释成生效的共享区归属（全局默认 → 根覆盖 → 每层收窄 → 全局硬黑名单）。
//! 管：`edit_of` / `edit_params` / `scope_from` 三个纯函数——本单元是 `workspace` 策略参数形状的唯一解释处。
//! 不管：归属规则本身与值类型（环 1 `permission` 的 `ScopeMode` / `SharedScope` / `SharedEdit`）；IO 与落盘。
//! 联动：`domain/seats.rs` 用 `scope_from` 算一席的生效态；`session_model` 只搬参数，不解释里面的字段。

use crate::units::pure::permission::api::{SharedEdit, SharedScope};

/// 目的：从策略参数里读出一层共享区覆盖；没写过参数 = 这一层不覆盖。
/// 参数：`params` = 会话落盘的不透明参数（只有本单元解释它）。
/// 错误：参数不是这一层认的形状时如实报错（文案带解析原因）。
pub fn edit_of(params: &serde_json::Value) -> Result<SharedEdit, String> {
    if params.is_null() {
        return Ok(SharedEdit::default());
    }
    serde_json::from_value(params.clone()).map_err(|e| format!("参数不是这个策略认的形状：{e}"))
}

/// 目的：把一层共享区覆盖序列化成策略参数；会话侧只搬值，不解释里面的字段。
/// 参数：`edit` = 要落的一层覆盖。
/// 错误：序列化失败时如实报错（理论上不会发生）。
pub fn edit_params(edit: &SharedEdit) -> Result<serde_json::Value, String> {
    serde_json::to_value(edit).map_err(|e| format!("共享区声明写不进参数：{e}"))
}

/// 目的：一条参数链的生效共享区归属：全局默认 → 根覆盖 → 每层收窄 → 全局硬黑名单。
/// 参数：`params` = 根在前的策略参数链；`base` = 设置里的全局默认范围；`hard_deny` = 全局硬黑名单。
/// 错误：链为空报「会话权限链为空」；某层试图放宽父层时由 `apply_narrowing` 如实拒绝。
pub fn scope_from(
    params: &[serde_json::Value],
    base: &SharedScope,
    hard_deny: &[String],
) -> Result<SharedScope, String> {
    let root = params.first().ok_or_else(|| "会话权限链为空".to_string())?;
    let mut out = base.apply_root(&edit_of(root)?)?;
    for child in params.iter().skip(1) {
        out = out.apply_narrowing(&edit_of(child)?)?;
    }
    Ok(out.with_hard_deny(hard_deny))
}
