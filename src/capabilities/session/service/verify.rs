//! 目的：核心核实用的小工具环境——**只读**、根是本次工作的共享区。
//! 管：`VerifySpec`（核实环境的全部事实与端口）与 `verify_tools`（装配成 `CoreVerify`）。
//! 不管：核心操作的回路（在 `service::core_operation`）；谁该核实（在 `collab` / `slate` 的调用点）。
//! 联动：`collab` 的各关卡在调核心操作前按角色装配一份；工具面只发**该角色的只读核实工具**。

use std::sync::Arc;

use crate::capabilities::llm::api::{Llm, ToolMode};
use crate::capabilities::session::api::{CoreVerify, ToolFace};
use crate::capabilities::session::domain::tools::MemberTools;
use crate::capabilities::tools::api::{Observations, ToolBook, ToolExec, ToolNotes, Tools};
use crate::capabilities::vm::api::Vm;
use crate::capabilities::workspace::api::Sandbox;
use crate::kernel::api::{FenceSpec, Tier};
use crate::kernel::ports::Log;

/// 目的：装配核心核实环境所需的全部事实与端口：沙箱、角色、通道形态、网络、工具总表、说明块素材与四个端口。
pub struct VerifySpec<'a> {
    /// 目的：这一场工作的沙箱（调用方给 `sandboxes.list.first()`）；核实的根是它的共享区。
    pub sandbox: &'a Sandbox,
    /// 目的：这一席的角色 id（`systools/roles.yaml` 的键）：只发它名下的只读核实工具。
    pub role: &'a str,
    /// 目的：核心通道的工具调用形态（原生才声明工具；信封通道看提示词里的说明）。
    pub mode: ToolMode,
    /// 目的：本次执行是否放行出站网络（随围栏交给机制层）。
    pub net: bool,
    /// 目的：工具总表（角色面过滤与内置工具的参数契约都按它来）。
    pub book: ToolBook,
    /// 目的：工具说明块的标题模板（来自提示词段 `tools_this_turn`）。
    pub tools_this_turn: &'a str,
    /// 目的：工具总表与角色表的能力面：发放名单从它读。
    pub systools: Arc<dyn Tools>,
    /// 目的：llm 用例面（核心操作的工具循环要造修信封的句柄）。
    pub llm: Arc<dyn Llm + Send + Sync>,
    /// 目的：运行日志：模型输出被长度截断这类"看不见的事实"要落盘。
    pub log: Arc<dyn Log + Send + Sync>,
    /// 目的：工具执行面：只读核实工具经它跑。
    pub tools: Arc<dyn ToolExec + Send + Sync>,
    /// 目的：虚拟机档的 guest 用例面。
    pub vm: Arc<dyn Vm + Send + Sync>,
}

/// 目的：装配核心核实环境：沙箱改成"核心"（共享区当私有区、没有模块），工具面只发只读核实工具。
///   没有它，核心操作（出方案 / 节点验收…）一想核实就被判"没调用 X"而整步中断。
pub fn verify_tools(spec: &VerifySpec<'_>) -> Option<Box<dyn CoreVerify>> {
    let mut sb = spec.sandbox.clone();
    sb.agent = "核心".to_string();
    sb.private = sb.shared.clone();
    sb.modules.clear();
    sb.modules_with_userdata.clear();
    let book = spec.book.clone();
    let allowed: Vec<String> = spec
        .systools
        .tool_face(spec.role)
        .map(|f| {
            f.into_iter()
                .map(|(id, _)| id.to_string())
                .filter(|id| {
                    book.get(id)
                        .map(|s| {
                            sb.permissions
                                .offers(id, s.effect(), &s.max_reach(), s.scope)
                        })
                        .unwrap_or(true)
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Box::new(MemberTools {
        face: ToolFace {
            mode: spec.mode,
            role: spec.role.to_string(),
            modules: std::collections::BTreeMap::new(),
            tier: Tier::Host,
            guest_lease: String::new(),
            sandbox: sb.clone(),
            tools_this_turn: spec.tools_this_turn.to_string(),
            builtin_tools: book,
            unavailable: std::collections::BTreeMap::new(),
            fence: FenceSpec::for_session(&sb, &[], spec.net),
            // 核心核实用的是只读内置工具，没有模块工具进程，也就没有注入项。
            module_env: Default::default(),
            runtime_path: None,
            runtime_roots: Vec::new(),
            allowed,
            with_modules: false,
            notes: ToolNotes::default(),
        },
        llm: Arc::clone(&spec.llm),
        log: Arc::clone(&spec.log),
        tools: Arc::clone(&spec.tools),
        vm: Arc::clone(&spec.vm),
        residents: Arc::new(crate::capabilities::residents::api::NoResidents),
        // 核心核实的工具面只读：没有共享区提交这回事。
        workspace: None,
        observations: Observations::default(),
        shell_live: None,
        reply_seq: 0,
        line: Default::default(),
        handlers: Vec::new(),
    }))
}
