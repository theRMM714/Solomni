//! 目的：会话装配的纯规则——角色工具面按权限裁剪、已就绪服务操作的归并、服务操作写进模块工具表。
//! 管：`face_tools`（角色发放的名字清单 → 实际发放）、`service_ops_of`（服务视图 → 按模块分组的操作）、
//!   `serviced_tools`（操作写进工具表，并给出模块工具行与服务参数说明行）。
//! 不管：取事实（权限面 / 沙箱 / 服务视图在调用方）；会话对象怎么建（`service/assemble.rs`）；不碰 IO。
//! 联动：`service/assemble.rs` 的 `member_tools` 依次调用它们；输出的提示词行由 `tool_notes` 消费。

use std::collections::BTreeMap;

use crate::capabilities::permission::api::PermissionSet;
use crate::capabilities::residents::api::{ServiceState, ServiceView};
use crate::capabilities::session::api::{ModuleTools, ServiceOp};
use crate::capabilities::tools::api::ToolBook;

/// 目的：角色表发放的名字清单按当前权限面裁剪：deny 的工具**不发放**（不是发放后再拒），ask 的照常发放。
/// 约束：总表里没有的名字照旧发放——认不出的名字由总表自检负责，不在这一层拦。
pub fn face_tools(
    role_face: Vec<String>,
    book: &ToolBook,
    permissions: &PermissionSet,
) -> Vec<String> {
    role_face
        .into_iter()
        .filter(|id| {
            book.get(id)
                .map(|s| permissions.offers(id, s.effect(), &s.max_reach(), s.scope))
                .unwrap_or(true)
        })
        .collect()
}

/// 目的：**已就绪**常驻服务的操作，按模块分组（模块 id → 该模块的服务操作）。
/// 约束：只收已就绪的服务——没启动的没有操作可发现，不该出现在模型工具面里；操作名 = 服务名点操作名。
pub fn service_ops_of(views: &[ServiceView]) -> BTreeMap<String, Vec<ServiceOp>> {
    let mut out: BTreeMap<String, Vec<ServiceOp>> = BTreeMap::new();
    for view in views {
        if view.state != ServiceState::Ready {
            continue;
        }
        for op in &view.operations {
            out.entry(view.module.clone()).or_default().push(ServiceOp {
                tool: format!("{}.{}", view.name, op.name),
                service: view.name.clone(),
                op: op.name.clone(),
                description: op.description.clone(),
                parameters: op.parameters.clone(),
            });
        }
    }
    out
}

/// 目的：把服务操作写进本席的模块工具表，并给出两段提示词行。
/// 返回：`(模块工具行, 服务参数说明行)`——前者每模块一行"- 模块：操作、操作"，后者每个带参数契约的操作
///   "- 模块.操作\n参数说明"；参数契约是服务给的 JSON Schema 原样，没有可列参数就跳过。
/// 约束：只写进**本席工具面里有**的模块（表里没有的模块不凭空出现）。
pub fn serviced_tools(
    started: &BTreeMap<String, Vec<ServiceOp>>,
    table: &mut BTreeMap<String, ModuleTools>,
) -> (Vec<String>, Vec<String>) {
    let mut module_tool_lines: Vec<String> = Vec::new();
    let mut service_param_lines: Vec<String> = Vec::new();
    for (id, ops) in started {
        if let Some(mt) = table.get_mut(id) {
            for op in ops {
                mt.services.insert(op.tool.clone(), op.clone());
            }
        }
        let names = ops
            .iter()
            .map(|o| o.tool.clone())
            .collect::<Vec<_>>()
            .join("、");
        module_tool_lines.push(format!("- {}：{}", id, names));
        for op in ops {
            let Some(schema) = &op.parameters else {
                continue;
            };
            let signature = crate::capabilities::tools::api::json_schema_lines(schema).join("\n");
            if signature.is_empty() {
                continue;
            }
            service_param_lines.push(format!("- {}.{}\n{}", id, op.tool, signature));
        }
    }
    (module_tool_lines, service_param_lines)
}
