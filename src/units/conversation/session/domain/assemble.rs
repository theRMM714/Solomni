//! 目的：会话装配的纯派生与**唯一的工具面装配机**——席位面合成、已就绪服务操作的归组、模块工具行与服务参数行。
//! 管：`seat_face` / `resolve`（席位面合成）与 `assemble`（一席工具面的整装），以及 `face_tools` / `service_ops_of` / `serviced_tools` 这几个纯函数。
//! 不管：端口本身的实现（`tools` / `residents` / `workspace`）；工具总表与机制席位声明（在 `session::domain::mechanism`）；会话对象的拼装（在 `session::service::assemble`）。
//! 联动：`session::service::assemble`、核心操作回路、核心核实、回合换席（`MemberTools::install_seat`）都调 `assemble`，不得各写一份。

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::kernel::ports::ToolHandler;
use crate::units::catalog::modules::api::Module;
use crate::units::conversation::session::domain::mechanism::{self, Seat};
use crate::units::conversation::session::domain::tools::{
    ModuleTools, ServiceOp, service_ops_for_modules, tool_table,
};
use crate::units::prompt::api::Prompt;
use crate::units::residents::api::{ResidentOps, ServiceState, ServiceView};
use crate::units::tools::api::{
    DELETE, EDIT, GREP, LIST, PATCH, READ, SHELL, ToolBook, ToolNotes, ToolSchema, WRITE,
    tool_notes,
};
use crate::units::workspace::api::{Sandbox, WORK_TOOLS, Workspace};

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
            let signature = crate::units::tools::api::json_schema_lines(schema).join("\n");
            if signature.is_empty() {
                continue;
            }
            service_param_lines.push(format!("- {}.{}\n{}", id, op.tool, signature));
        }
    }
    (module_tool_lines, service_param_lines)
}

/// 目的：一个席位这一回合的**工具面**（id 清单，按四源并集）：
///   读面恒发（`read` / `list` / `grep` / `shell`）→ `write` 决定写面 → `publish` 决定发布面 → 机制动作。
/// 约束：读面恒发是安全的——围栏只给私有沙箱写权，`shell` 也改不到共享区；真正能改共享区的只有 `work_*`。
pub fn seat_face(seat: &Seat) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    let mut push = |id: &'static str| {
        if !out.contains(&id) {
            out.push(id);
        }
    };
    for id in [READ, LIST, GREP, SHELL] {
        push(id);
    }
    if seat.write {
        for id in [WRITE, EDIT, PATCH, DELETE] {
            push(id);
        }
    }
    if seat.publish {
        for id in WORK_TOOLS {
            push(id);
        }
    }
    for id in seat.tools {
        push(id);
    }
    out
}

/// 目的：把席位的工具面逐项解析成工具声明（顺序即 `seat_face` 的顺序）——装配与会话包装只用这一个口径。
/// 错误：席位引用了总表里没有的工具（装配错误，如实报错，不静默跳过）。
pub fn resolve<'a>(
    seat: &Seat,
    book: &'a ToolBook,
) -> Result<Vec<(&'static str, &'a ToolSchema)>, String> {
    let mut out = Vec::new();
    for id in seat_face(seat) {
        let schema = book
            .get(id)
            .ok_or_else(|| format!("席位 {} 的工具面引用了总表里没有的工具：{}", seat.id, id))?;
        out.push((id, schema));
    }
    Ok(out)
}

/// 目的：装配一席工具面要的**全部输入**：席位声明 + 会话侧的模块/沙箱 + 三个端口面。
/// 约束：席位没登记时不报错（装配期已由 `mechanism::problems` 拦下），退化成空面。
pub struct FaceInput<'a> {
    pub seat: &'a str,
    pub modules: &'a [Module],
    pub sandbox: &'a Sandbox,
    pub book: &'a ToolBook,
    pub prompt: &'a dyn Prompt,
    pub residents: &'a dyn ResidentOps,
    /// 目的：本环境是否连模块工具一起发——核实环境只发内置只读工具，不发模块表（席位声明仍一样）。
    pub modules_allowed: bool,
    /// 目的：发布面工具的宿主——核实环境没有工作区（None = 一个 `work_*` 也不发）。
    pub workspace: Option<&'a Arc<dyn Workspace + Send + Sync>>,
}

/// 目的：一席工具面装配出的**纯值**：发放名单、模块工具表、说明块与发布工具的执行者。
/// 约束：`modules` 与 `with_modules` 必须同一次装配产出——模块表发了、说明块却说没有，模型会瞎试。
pub struct SeatFace {
    pub seat: String,
    pub allowed: Vec<String>,
    pub modules: BTreeMap<String, ModuleTools>,
    pub with_modules: bool,
    pub notes: ToolNotes,
    pub handlers: Vec<Arc<dyn ToolHandler>>,
}

/// 目的：**会话唯一的工具面装配机**：席位声明 → 发放名单 → 发布工具执行者 → 模块工具表与已就绪服务 → 说明块。
/// 约束：别的路径（核心操作、核心核实、回合换席）都调这里，不得各写一份；`install_seat` 也用它现拼一席。
pub fn assemble(input: FaceInput<'_>) -> SeatFace {
    let seat = mechanism::seat(input.seat);
    let allowed = match seat {
        Some(seat) => match resolve(seat, input.book) {
            Ok(items) => face_tools(
                items.into_iter().map(|(id, _)| id.to_string()).collect(),
                input.book,
            ),
            Err(_) => Vec::new(),
        },
        None => Vec::new(),
    };
    let with_modules = input.modules_allowed && seat.map(|s| s.module_tools).unwrap_or(false);
    let handlers = match input.workspace {
        Some(ws) => crate::units::workspace::api::work_handlers(ws, input.sandbox, &allowed),
        None => Vec::new(),
    };
    let mut table = tool_table(input.modules);
    let started = if with_modules {
        service_ops_for_modules(
            service_ops_of(&input.residents.services().unwrap_or_default()),
            input.modules,
        )
    } else {
        BTreeMap::new()
    };
    let (module_tool_lines, service_param_lines) = serviced_tools(&started, &mut table);
    let mut notes = tool_notes(
        input.prompt,
        input.sandbox,
        if with_modules { input.modules } else { &[] },
        &service_param_lines,
    );
    if !module_tool_lines.is_empty() {
        notes.module_tools = format!("{}\n{}", notes.module_tools, module_tool_lines.join("\n"));
    }
    SeatFace {
        seat: input.seat.to_string(),
        allowed,
        modules: table,
        with_modules,
        notes,
        handlers,
    }
}
