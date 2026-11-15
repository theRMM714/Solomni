//! 目的：模块动作 id 的纯解析——`module.<模块id>.<工具名>` 与 `module.<模块id>.<服务名>.<操作名>`。
//! 管：`module_tool`（静态工具，含模块与工具声明）与 `service_op`（常驻服务的操作三段）两条解析。
//! 不管：清单从哪来（在 `ports.rs` / `detail/`）；解析出来以后怎么执行（在 conductor 的模块动作里）。
//! 联动：由 `modules/api.rs` 重导出；conductor 取好 `Roster` 后经能力面调用。

use super::module::{Module, Roster, ToolDecl};

/// 目的：把 `module.<模块id>.<工具名>` 解析成清单里的模块、工具名与工具声明。
/// 参数：`roster` = 当前清单；`id` = 动作 id。
/// 返回：命中的模块、工具名与声明；模块 id 可能含 `.`，按清单顺序逐段匹配。
/// 错误：清单里没有这条工具时给固定文案（动作 id 的写法一并说清）。
pub fn module_tool(roster: &Roster, id: &str) -> Result<(Module, String, ToolDecl), String> {
    let rest = id.strip_prefix("module.").unwrap_or("");
    for m in &roster.modules {
        let prefix = format!("{}.", m.manifest.id);
        if let Some(tool) = rest.strip_prefix(&prefix)
            && let Some(decl) = m.manifest.tools.get(tool)
        {
            return Ok((m.clone(), tool.to_string(), decl.clone()));
        }
    }
    Err(format!(
        "清单里没有这个模块工具：{}（动作 id 形如 module.<模块id>.<工具名>）",
        id
    ))
}

/// 目的：把 `module.<模块id>.<服务名>.<操作名>` 解析成（模块 id、服务名、操作名）。
/// 参数：`roster` = 当前清单；`id` = 动作 id。
/// 返回：命中的三段；None = 不是**已声明**常驻服务的操作（交给静态模块工具那条路）。
/// 约束：模块 id 与服务名都可能含 `.`，按声明逐段匹配，操作名必须非空。
pub fn service_op(roster: &Roster, id: &str) -> Option<(String, String, String)> {
    let rest = id.strip_prefix("module.").unwrap_or("");
    for m in &roster.modules {
        let Some(after_m) = rest.strip_prefix(&format!("{}.", m.manifest.id)) else {
            continue;
        };
        for name in m.manifest.services.keys() {
            if let Some(op) = after_m.strip_prefix(&format!("{}.", name))
                && !op.is_empty()
            {
                return Some((m.manifest.id.clone(), name.clone(), op.to_string()));
            }
        }
    }
    None
}
