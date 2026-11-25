//! 目的：核心核实用的小工具环境——**只读**、根是本次工作的共享区。
//! 管：`VerifySpec`（核实环境的全部事实与端口）与 `verify_tools`（装配成 `CoreVerify`）。
//! 不管：核心操作的回路（在 `service::core_operation`）；谁该核实（在 `collab` / `slate` 的调用点）。
//! 联动：`collab` 的各关卡在调核心操作前按席位装配一份；工具面只发**该席位声明里的工具**。

use std::sync::Arc;

use crate::units::conversation::session::api::{CoreVerify, ToolFace};
use crate::units::conversation::session::domain::assemble::{FaceInput, assemble};
use crate::units::conversation::session::domain::tools::MemberTools;
use crate::units::llm::api::{Llm, ToolMode};
use crate::units::prompt::api::Prompt;
use crate::units::tools::api::{Observations, ToolBook, ToolExec};
use crate::units::vm::api::Vm;
use crate::units::workspace::api::Sandbox;
use crate::kernel::api::fence::FencePlan;
use crate::kernel::api::types::Tier;
use crate::kernel::ports::Log;

/// 目的：装配核心核实环境所需的全部事实与端口：沙箱、席位、通道形态、网络、工具总表、说明块素材与四个端口。
pub struct VerifySpec<'a> {
    /// 目的：这一场工作的沙箱（调用方给 `sandboxes.list.first()`）；核实的根是它的共享区。
    pub sandbox: &'a Sandbox,
    /// 目的：这一席的席位 id（机制声明的席位）：只发它名下的工具。
    pub seat: &'a str,
    /// 目的：核心通道的工具调用形态（原生才声明工具；信封通道看提示词里的说明）。
    pub mode: ToolMode,
    /// 目的：本次执行是否放行出站网络（随围栏交给机制层）。
    pub net: bool,
    /// 目的：工具总表（席位面过滤与内置工具的参数契约都按它来）。
    pub book: ToolBook,
    /// 目的：工具说明块的标题模板（来自提示词段 `tools_this_turn`）。
    pub tools_this_turn: &'a str,
    /// 目的：提示词段面（工具说明块素材由装配机现取；核实环境与成员同源，不自己拼文案）。
    pub prompt: Arc<dyn Prompt>,
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
    let residents = crate::units::residents::api::NoResidents;
    let seat = assemble(FaceInput {
        seat: spec.seat,
        modules: &[],
        sandbox: &sb,
        book: &book,
        prompt: &*spec.prompt,
        residents: &residents,
        // 核心核实只发内置只读工具：不发模块工具表。
        modules_allowed: false,
        // 核心核实的工具面只读：没有共享区提交这回事。
        workspace: None,
    });
    Some(Box::new(MemberTools {
        face: ToolFace {
            mode: spec.mode,
            seat: seat.seat,
            modules: seat.modules,
            tier: Tier::Host,
            guest_lease: String::new(),
            sandbox: sb.clone(),
            tools_this_turn: spec.tools_this_turn.to_string(),
            builtin_tools: book,
            unavailable: std::collections::BTreeMap::new(),
            fence: FencePlan::for_session(&sb, &[], spec.net),
            // 核心核实用的是只读内置工具，没有模块工具进程，也就没有注入项。
            module_env: Default::default(),
            runtime_path: None,
            runtime_roots: Vec::new(),
            allowed: seat.allowed,
            with_modules: seat.with_modules,
            notes: seat.notes,
        },
        llm: Arc::clone(&spec.llm),
        log: Arc::clone(&spec.log),
        tools: Arc::clone(&spec.tools),
        vm: Arc::clone(&spec.vm),
        residents: Arc::new(crate::units::residents::api::NoResidents),
        // 核心核实的工具面只读：没有共享区提交这回事。
        workspace: None,
        observations: Observations::default(),
        shell_live: None,
        reply_seq: 0,
        line: Default::default(),
        handlers: seat.handlers,
        // 核实环境不换席（席位在装配时定死），模块表也恒空。
        modules: Vec::new(),
        prompt: Arc::clone(&spec.prompt),
    }))
}
