//! 目的：会话装配的纯派生——席位工具面的过滤、已就绪服务操作的归组、模块工具行与服务参数行。
//! 管：`face_tools` / `service_ops_of` / `serviced_tools` 三个纯函数（无 IO、无端口）。
//! 不管：端口读取与最终对象拼装（在 `session::service::assemble`）；工具总表与机制席位本身（在 `tools` 与 `session::domain::mechanism`）。
//! 联动：`session::service::assemble` 装配一席工具面时调用；数据来自 `tools` / `residents` / `session` 的 api。

use std::collections::BTreeMap;

use crate::capabilities::conversation::session::api::{ModuleTools, ServiceOp};
use crate::capabilities::residents::api::{ServiceState, ServiceView};
use crate::capabilities::tools::api::ToolBook;

/// 目的：席位面合成出的工具 id 再去掉总表里没有的（发放了却查不到，等于发了个空 id）。
/// 参数：seat_face = 席位面合成的 id 清单；book = 工具总表。
/// 返回：过滤后的 id 清单（发放顺序不变）。
/// 约束：席位面**就是**授权面——档位只管网络开关，不再逐工具裁剪。
pub fn face_tools(seat_face: Vec<String>, book: &ToolBook) -> Vec<String> {
    seat_face
        .into_iter()
        .filter(|id| book.get(id).is_some())
        .collect()
}

/// 目的：**已就绪**常驻服务的操作，按模块分组（模块 → 该模块的服务操作）。
/// 约束：只收已就绪的服务——没启动的没有操作可发现，不该出现在模型工具面里。
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

/// 目的：把已就绪服务的操作并进模块工具表，并拼出提示词要的两组行（模块工具行、服务参数行）。
/// 参数：started = 模块 → 服务操作；table = 模块工具表（就地补 `services`）。
/// 返回：(模块工具行, 服务参数行)；顺序与 `started` 的键序一致。
pub fn serviced_tools(
    started: &BTreeMap<String, Vec<ServiceOp>>,
    table: &mut BTreeMap<String, ModuleTools>,
) -> (Vec<String>, Vec<String>) {
    let mut module_tool_lines: Vec<String> = Vec::new();
    // 这些操作**自带参数契约**（服务给的 JSON Schema 原样）：信封通道也要把参数说清，
    // 否则模型只知道名字、只能瞎试参数（真实模型上真发生过）。
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
