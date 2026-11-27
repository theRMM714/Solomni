//! 目的：五个工具来源的实例——各自按回合上下文交工具 id；会话只按 id 挂来源，不认识实现。
//! 管：`AgentSource` / `ModuleSource` / `LoopSource` / `WorkspaceSource` / `ProfileSource` 与 `build_sources`。
//! 不管：工具形状与执行（tools / 模块 / 共享区）；授权（权限端口）；装配（`assemble`）。
//! 联动：会话类型表声明挂哪些来源；`assemble` 按 id 造实例并求并集。

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::units::business::agentloop::domain::member_tools::ModuleTools;
use crate::units::business::session_model::api::{
    EXEC, FS_READ, FS_WRITE, SOURCE_AGENT, SOURCE_LOOP, SOURCE_MODULE, SOURCE_PROFILE,
    SOURCE_WORKSPACE, ToolSource, TurnCtx,
};
use crate::units::business::tools::api::{DELETE, EDIT, GREP, LIST, PATCH, READ, SHELL, WRITE};

/// 目的：agent 自己的能力组来源：读面 / 工作面 / 执行各按能力组发放。
pub struct AgentSource;

impl ToolSource for AgentSource {
    fn id(&self) -> &str {
        SOURCE_AGENT
    }

    fn offer(&self, ctx: &TurnCtx) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut push = |id: &str| {
            if !out.iter().any(|t| t == id) {
                out.push(id.to_string());
            }
        };
        if ctx.has(FS_READ) {
            for id in [READ, LIST, GREP] {
                push(id);
            }
        }
        if ctx.has(FS_WRITE) {
            for id in [WRITE, EDIT, PATCH, DELETE] {
                push(id);
            }
        }
        if ctx.has(EXEC) {
            push(SHELL);
        }
        out
    }
}

/// 目的：agent 装的模块工具来源：交出限定 id `<模块>.<工具>` 与 `<模块>.<服务>.<操作>`。
/// 约束：ids 在装配期按模块工具表算一次——来源只回答在场，不再问模块是否启用。
pub struct ModuleSource {
    ids: Vec<String>,
}

impl ModuleSource {
    /// 目的：按模块工具表（含已就绪服务）算出这一席的模块工具 id。
    pub fn new(table: &BTreeMap<String, ModuleTools>) -> ModuleSource {
        let mut ids: Vec<String> = Vec::new();
        for (module, tools) in table {
            for name in tools.commands.keys() {
                ids.push(format!("{}.{}", module, name));
            }
            for name in tools.services.keys() {
                ids.push(format!("{}.{}", module, name));
            }
        }
        ModuleSource { ids }
    }
}

impl ToolSource for ModuleSource {
    fn id(&self) -> &str {
        SOURCE_MODULE
    }

    fn offer(&self, _ctx: &TurnCtx) -> Vec<String> {
        self.ids.clone()
    }
}

/// 目的：当前流程段交的机制动作：装配期按「段 + 角色」在声明表里查一次，拍平成值。
/// 约束：段或角色位没在册时装配期交空面（流程段表校验已拦下）；同一角色在不同段可以拿不同的机制动作。
pub struct LoopSource {
    actions: Vec<String>,
}

impl LoopSource {
    /// 目的：按这一席所在段的角色位拍平出段来源（动作 id 已在装配期取好）。
    pub fn new(actions: &[String]) -> LoopSource {
        LoopSource {
            actions: actions.to_vec(),
        }
    }
}

impl ToolSource for LoopSource {
    fn id(&self) -> &str {
        SOURCE_LOOP
    }

    fn offer(&self, _ctx: &TurnCtx) -> Vec<String> {
        self.actions.clone()
    }
}

/// 目的：共享区（工作区）声明的工具来源：交工作台策略给这一席的共享区动作（有工作区才问策略）。
/// 约束：名单由策略在装配期算好；这里只回答在场，不判能力组、不看段。
pub struct WorkspaceSource {
    tools: Vec<String>,
}

impl WorkspaceSource {
    /// 目的：按策略给这一席的共享区动作 id 造来源（空 = 这一席没有共享区动作）。
    pub fn new(tools: &[String]) -> WorkspaceSource {
        WorkspaceSource {
            tools: tools.to_vec(),
        }
    }
}

impl ToolSource for WorkspaceSource {
    fn id(&self) -> &str {
        SOURCE_WORKSPACE
    }

    fn offer(&self, _ctx: &TurnCtx) -> Vec<String> {
        self.tools.clone()
    }
}

/// 目的：会话类型展开进会话的 system 工具来源（只在会话 meta 里有；成员回合没有）。
pub struct ProfileSource {
    tools: Vec<String>,
}

impl ToolSource for ProfileSource {
    fn id(&self) -> &str {
        SOURCE_PROFILE
    }

    fn offer(&self, _ctx: &TurnCtx) -> Vec<String> {
        self.tools.clone()
    }
}

/// 目的：按会话类型声明的来源 id 造实例；未知 id 跳过（装配期 `Profiles::validate` 已拦下）。
/// 参数：ids = 来源 id；table = 模块工具表（含已就绪服务）；system_tools = 会话 meta 里的 system 工具；
///   loop_tools = 当前段当前角色的机制动作（装配期拍平，段表不在来源里查）；
///   shared_tools = 工作台策略给这一席的共享区动作（空 = 这一席没有）。
pub fn build_sources(
    ids: &[String],
    table: &BTreeMap<String, ModuleTools>,
    system_tools: &[String],
    loop_tools: &[String],
    shared_tools: &[String],
) -> Vec<Arc<dyn ToolSource>> {
    let mut out: Vec<Arc<dyn ToolSource>> = Vec::new();
    for id in ids {
        match id.as_str() {
            SOURCE_AGENT => out.push(Arc::new(AgentSource)),
            SOURCE_MODULE => out.push(Arc::new(ModuleSource::new(table))),
            SOURCE_LOOP => out.push(Arc::new(LoopSource::new(loop_tools))),
            SOURCE_WORKSPACE => out.push(Arc::new(WorkspaceSource::new(shared_tools))),
            SOURCE_PROFILE => out.push(Arc::new(ProfileSource {
                tools: system_tools.to_vec(),
            })),
            _ => {}
        }
    }
    out
}
