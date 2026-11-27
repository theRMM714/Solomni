//! 目的：会话装配的纯派生与**唯一的工具面装配机**——来源交 id、去重成面、已就绪服务操作归组、说明块。
//! 管：`assemble`（一席工具面的整装）与 `service_ops_of` / `serviced_tools` 这两个纯函数。
//! 不管：来源的实现（`sources`）；工具总表与机制声明（在 `session_model`）；会话对象的拼装（在 `super::assembly`）。
//! 联动：`super::assembly`、核心操作回路、核心核实、回合换席（`MemberTools::install_seat`）都调 `assemble`，不得各写一份。

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::kernel::ports::ToolHandler;
use crate::units::business::agentloop::domain::member_tools::{
    ModuleTools, ServiceOp, service_ops_for_modules, tool_table,
};
use crate::units::business::agentloop::domain::sources::build_sources;
use crate::units::business::catalog::modules::api::Module;
use crate::units::business::prompt::api::Prompt;
use crate::units::business::residents::api::{ResidentOps, ServiceState, ServiceView};
use crate::units::business::session_model::api::{Loops, SOURCE_MODULE, ToolSource, TurnCtx};
use crate::units::business::tools::api::{ToolNotes, tool_notes};
use crate::units::business::workbench::api::Bench;

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
            let signature =
                crate::units::business::tools::api::json_schema_lines(schema).join("\n");
            if signature.is_empty() {
                continue;
            }
            service_param_lines.push(format!("- {}.{}\n{}", id, op.tool, signature));
        }
    }
    (module_tool_lines, service_param_lines)
}

/// 目的：装配一席工具面要的**全部输入**：回合上下文 + 来源 id + 会话侧的模块/沙箱 + 三个端口面。
pub struct FaceInput<'a> {
    /// 目的：这一回合在哪个流程段（loop 来源按它查角色位的机制动作）。
    pub loop_id: &'a str,
    /// 目的：这一回合的角色 id。
    pub role: &'a str,
    /// 目的：当前回合是哪个 agent。
    pub agent: &'a str,
    /// 目的：工作区 id；没有工作区为空。
    pub workspace_name: Option<&'a str>,
    /// 目的：这个 agent 的能力组。
    pub abilities: &'a [String],
    /// 目的：会话类型 id。
    pub profile: &'a str,
    /// 目的：挂哪些工具来源（来源 id）。
    pub source_ids: &'a [String],
    /// 目的：会话类型展开进会话的 system 工具 id。
    pub system_tools: &'a [String],
    /// 目的：流程段与角色的声明表（loop 来源在装配期按它拍平；核实环境给 None）。
    pub loops: Option<&'a Loops>,
    pub modules: &'a [Module],
    pub bench: &'a Bench,
    pub prompt: &'a dyn Prompt,
    pub residents: &'a dyn ResidentOps,
    /// 目的：版本库用例面（一席的 `work_*` 执行者按它造）；没有工作区的席位给 None（一个 `work_*` 也不发）。
    pub versions:
        Option<&'a Arc<dyn crate::units::business::versions::api::Versions + Send + Sync>>,
}

/// 目的：一席工具面装配出的**纯值**：发放名单、模块工具表、说明块与共享区工具的执行者。
/// 约束：`modules` 与 `with_modules` 必须同一次装配产出——模块表发了、说明块却说没有，模型会瞎试。
pub struct SeatFace {
    pub role: String,
    pub allowed: Vec<String>,
    pub modules: BTreeMap<String, ModuleTools>,
    pub with_modules: bool,
    /// 目的：这次装配挂了哪些来源（诊断用：`tools explain` 与日志据此说清 id 从哪来）。
    /// 约束：现在只由装配写给将来；还没有读它的诊断命令。
    #[allow(dead_code)]
    pub sources: Vec<String>,
    pub notes: ToolNotes,
    pub handlers: Vec<Arc<dyn ToolHandler>>,
}

/// 目的：**会话唯一的工具面装配机**：来源交 id → 去重成面 → 共享区执行者 → 模块工具表与已就绪服务 → 说明块。
/// 约束：别的路径（核心操作、核心核实、回合换席）都调这里，不得各写一份；`install_seat` 也用它现拼一席。
pub fn assemble(input: FaceInput<'_>) -> SeatFace {
    let ctx = TurnCtx {
        loop_id: Some(input.loop_id).filter(|id| !id.is_empty()),
        agent_id: input.agent,
        abilities: input.abilities,
        profile: input.profile,
        workspace: input.workspace_name,
    };
    let with_modules = input.source_ids.iter().any(|id| id == SOURCE_MODULE);
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
    // 段来源的动作在装配期拍平成值：来源只回答在场，运行期不再查段表。
    let loop_tools = input
        .loops
        .and_then(|loops| loops.cast_of(input.loop_id, input.role))
        .map(|cast| cast.tools.clone())
        .unwrap_or_default();
    let sources = build_sources(input.source_ids, &table, input.system_tools, &loop_tools);
    let (allowed, used) = face_of(&ctx, &sources);
    let handlers = match (input.workspace_name.is_some(), input.versions) {
        (true, Some(versions)) => {
            crate::units::business::versions::api::work_handlers(versions, input.bench, &allowed)
        }
        _ => Vec::new(),
    };
    let mut notes = tool_notes(
        input.prompt,
        input.bench,
        if with_modules { input.modules } else { &[] },
        &service_param_lines,
    );
    if !module_tool_lines.is_empty() {
        notes.module_tools = format!("{}\n{}", notes.module_tools, module_tool_lines.join("\n"));
    }
    SeatFace {
        role: input.role.to_string(),
        allowed,
        modules: table,
        with_modules,
        sources: used,
        notes,
        handlers,
    }
}

/// 目的：每个来源这一刻交出的 id（按来源顺序，交空的不记）——`face_of` 与工具面诊断共用这一处求值。
/// 参数：ctx = 回合上下文；sources = 这一席挂的来源实例。
/// 返回：(来源 id, 它交的 id)；同一个 id 出现在多个来源里即「同 id 多来源」。
pub fn offers_of(ctx: &TurnCtx<'_>, sources: &[Arc<dyn ToolSource>]) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for source in sources {
        let ids = source.offer(ctx);
        if ids.is_empty() {
            continue;
        }
        out.push((source.id().to_string(), ids));
    }
    out
}

/// 目的：把各来源交出的 id 并成这一回合的工具面：按来源顺序去重，并记下真正交过 id 的来源。
/// 参数：offers = `offers_of` 的结果（顺序即面序）。
/// 返回：(工具面 id, 交过 id 的来源 id)。
pub fn merge_offers(offers: &[(String, Vec<String>)]) -> (Vec<String>, Vec<String>) {
    let mut allowed: Vec<String> = Vec::new();
    let mut used: Vec<String> = Vec::new();
    for (source, ids) in offers {
        if ids.is_empty() {
            continue;
        }
        if !used.iter().any(|s| s == source) {
            used.push(source.clone());
        }
        for id in ids {
            if !allowed.contains(id) {
                allowed.push(id.clone());
            }
        }
    }
    (allowed, used)
}

/// 目的：把来源交出的 id 并成这一回合的工具面：按来源顺序去重，并记下真正交过 id 的来源。
/// 参数：ctx = 回合上下文；sources = 这一席挂的来源实例（顺序即面序）。
/// 返回：(工具面 id, 交过 id 的来源 id)。
pub fn face_of(ctx: &TurnCtx<'_>, sources: &[Arc<dyn ToolSource>]) -> (Vec<String>, Vec<String>) {
    merge_offers(&offers_of(ctx, sources))
}

/// 目的：测试用——按某个角色在它所在段里的全能力口径算工具面 id（生产路径只有 assemble）。
/// 约束：段按角色反查（取第一个含它的段）；模块来源恒空（测试只看声明与能力组），有工作区、无 system 工具。
#[cfg(test)]
pub fn role_face(loops: &Loops, role_id: &str) -> Vec<String> {
    use crate::units::business::session_model::api::{SOURCE_AGENT, SOURCE_LOOP, SOURCE_WORKSPACE};
    let found = loops
        .loops
        .iter()
        .find(|l| l.cast.iter().any(|c| c.role == role_id));
    let Some(loop_id) = found.map(|l| l.id.as_str()) else {
        return Vec::new();
    };
    let abilities: Vec<String> = crate::units::pure::registry_model::api::DEFAULT_ABILITIES
        .iter()
        .map(|s| s.to_string())
        .collect();
    let ctx = TurnCtx {
        loop_id: Some(loop_id),
        agent_id: "test",
        abilities: &abilities,
        profile: "single",
        workspace: Some("test"),
    };
    let ids: Vec<String> = [SOURCE_AGENT, SOURCE_LOOP, SOURCE_WORKSPACE]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let loop_tools = loops
        .cast_of(loop_id, role_id)
        .map(|c| c.tools.clone())
        .unwrap_or_default();
    let sources = build_sources(&ids, &BTreeMap::new(), &[], &loop_tools);
    face_of(&ctx, &sources).0
}
