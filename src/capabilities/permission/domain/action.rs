//! 目的：动作授权的纯判据——动作声明的调用者清单里有没有这次的身份。
//! 管：`authorize_action` 一条：命中放行，未命中给带动作与身份的拒绝文案。
//! 不管：调用者身份从哪来、拒绝后怎么记审计（在 conductor 的入站分发里）；动作声明的解析（在 tools 能力）。
//! 联动：由 `permission/api.rs` 重导出；conductor 的动作分发与测试都经能力面调用。

/// 目的：授权判据——`callers` 里有 `token` 才放行。
/// 参数：`callers` = 动作声明的调用者身份清单；`token` = 本次调用者身份；`id` = 动作 id（只进拒绝文案）。
/// 返回：放行 `Ok(())`；拒绝带动作 id 与身份，说明越出授权范围。
pub fn authorize_action<S: AsRef<str>>(callers: &[S], token: &str, id: &str) -> Result<(), String> {
    if callers.iter().any(|c| c.as_ref() == token) {
        Ok(())
    } else {
        Err(format!(
            "动作 {} 不接受这个调用者：{}（越出授权范围）",
            id, token
        ))
    }
}
