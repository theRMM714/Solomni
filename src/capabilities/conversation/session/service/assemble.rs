//! 目的：一席工具面与单 agent 会话对象的装配——把能力面的事实拼成能跑的会话。
//! 管：`SessionPorts` / `MemberEnvParts` / `MemberSpec` / `SingleSpec` 四个参数对象，
//!   `member_tools` / `single_session` 两个装配入口，以及重建与代理两场的整场装配（`rebuild_single` / `proxy_session`）。
//! 不管：落盘（本能力的 `service.rs`）、生成驱动（`runtime`）、纯派生规则（在 `session::domain::assemble`）。
//! 联动：`turn` 侧取事实、算 `env` 并组 spec 后经本文件装配；`MemberTools`（工具面的活对象）**只活在会话里**，不出本能力。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::capabilities::catalog::modules::api::Module;
use crate::capabilities::catalog::registry::api::AgentMeta;
use crate::capabilities::conversation::session::api::{
    AgentSession, CompactChatFactory, ContextUsage, SessionEvent, SessionParams, ShellSlot,
    ToolFace, service_ops_for_modules, tool_table,
};
use crate::capabilities::conversation::session::domain::assemble::{
    face_tools, service_ops_of, serviced_tools,
};
use crate::capabilities::conversation::session::domain::tools::MemberTools;
use crate::capabilities::llm::api::{Channel, Llm, Msg, ToolMode};
use crate::capabilities::permission::api::CallScope;
use crate::capabilities::prompt::api::{Prompt, Segment};
use crate::capabilities::residents::api::ResidentOps;
use crate::capabilities::tools::api::{Observations, ToolExec, Tools, tool_notes};
use crate::capabilities::vm::api::{GuestOs, Vm};
use crate::capabilities::workspace::api::{ReadFact, Sandbox, Workspace};
use crate::kernel::api::{FenceSpec, Tier};
use crate::kernel::ports::Log;

/// 目的：装配一场会话要用的能力面（组合根注入的同一批端口）。
pub struct SessionPorts {
    /// 目的：llm 用例面：一席的收发句柄按它造。
    pub llm: Arc<dyn Llm + Send + Sync>,
    /// 目的：日志面：装配时记一条「单 agent 工作」。
    pub log: Arc<dyn Log + Send + Sync>,
    /// 目的：提示词册：工具说明与压缩提示从它取段。
    pub prompt: Arc<dyn Prompt>,
    /// 目的：工具总表与角色表：发放名单与内置工具表从它读。
    pub systools: Arc<dyn Tools>,
    /// 目的：工具执行面：一席的工具环境要它跑外部 / 内置工具。
    pub tools: Arc<dyn ToolExec + Send + Sync>,
    /// 目的：虚拟机用例面：一席的 guest 事实随工具面交给机制层。
    pub vm: Arc<dyn Vm + Send + Sync>,
    /// 目的：常驻服务管理面：已就绪服务的操作并入一席的工具面。
    pub residents: Arc<dyn ResidentOps + Send + Sync>,
    /// 目的：工作区用例面：一席的 `work_*` 工具执行者按它造。
    pub workspace: Arc<dyn Workspace + Send + Sync>,
    /// 目的：压缩通道工厂：新会话与重建会话共用同一份。
    pub compact: Arc<dyn CompactChatFactory>,
}

/// 目的：一席的工具环境事实（围栏 / 模块注入项 / 运行路径 / 工具链根）——由 `env` 能力从
///   （沙箱、权限、运行包计划、模块隐私）派生后**逐字段拷进来**的值结构。
/// 约束：本能力是第 4 层，不引 `env`；派生留在 turn 侧，这里只收结果，装配时不再重算。
#[derive(Debug, Clone)]
pub struct MemberEnvParts {
    /// 目的：本成员工具进程的围栏（可达范围 + 断网）。
    pub fence: FenceSpec,
    /// 目的：模块 id → 该模块隐私字段的注入项（env 名 → 值）；起进程时只走环境，不进命令行。
    pub module_env: BTreeMap<String, Vec<(String, String)>>,
    /// 目的：本机档运行包的生效 PATH（由 packages 计划给出；虚拟机档 / 没有包 = None）。
    pub runtime_path: Option<std::ffi::OsString>,
    /// 目的：shell 的只读工具链根：项目内基础 profile + 该 agent 模块的运行包根。
    pub runtime_roots: Vec<PathBuf>,
}

/// 目的：装配一席工具面所需的全部事实（模块、沙箱、形态、档位、角色、shell 槽与工具环境事实）。
pub struct MemberSpec<'a> {
    /// 目的：这一席挂的模块（工具表与隐私注入项按它算）。
    pub modules: &'a [Module],
    /// 目的：这一席的沙箱（工作名、agent 实例名与生效权限）。
    pub sandbox: &'a Sandbox,
    /// 目的：本档位下缺运行包、不能执行工具的模块（机制侧据此拒绝执行）。
    pub unavailable: BTreeMap<String, Vec<String>>,
    /// 目的：工具形态（原生 / 信封）。
    pub mode: ToolMode,
    /// 目的：这一席的身份（用户建的单 agent 会话 = solo，协作子会话 = executor）。
    pub role: &'a str,
    /// 目的：档位（本机 / 虚拟机）。
    pub tier: Tier,
    /// 目的：这一席的 guest 租约（本机档为空串）。
    pub guest_lease: String,
    /// 目的：这一席的 shell 活进程槽（成员工具面与状态栏共用同一份）。
    pub shell: Arc<ShellSlot>,
    /// 目的：工具环境事实（围栏 / 注入项 / 运行路径 / 工具链根），由 turn 侧的 `env` 派生后拷入。
    pub env: MemberEnvParts,
}

/// 目的：装配一份单 agent 会话所需的全部事实（agent、席事实、通道、guest、上下文预算与核心默认模型）。
pub struct SingleSpec<'a> {
    /// 目的：这一场的 agent 登记事实。
    pub agent: &'a AgentMeta,
    /// 目的：这一席的工具面事实。
    pub member: MemberSpec<'a>,
    /// 目的：解析好的模型通道。
    pub channel: Channel,
    /// 目的：虚拟机档的 guest 形态（本机档为 None）。
    pub guest: Option<GuestOs>,
    /// 目的：上下文预算三元组（窗口、压缩阈值百分比、保留百分比）。
    pub context: (u64, u8, u8),
    /// 目的：登记处的核心默认模型（agent 没填模型时用它）。
    pub core_model: Option<String>,
}

/// 目的：重建一席单 agent 会话所需的全部事实（参数、席事实、通道、回放出的对话与簿记、续号、预算与用量）。
pub struct RebuildSpec<'a> {
    /// 目的：会话参数：身份块每回合由它现渲染（与建立时同一口径）。
    pub params: SessionParams,
    /// 目的：这一席的工具面事实。
    pub member: MemberSpec<'a>,
    /// 目的：解析好的模型通道。
    pub channel: Channel,
    /// 目的：回放出的对话（只有真正发生过的事）。
    pub dialogue: Vec<Msg>,
    /// 目的：每行对应的历史长度（回档按它截断）。
    pub marks: Vec<usize>,
    /// 目的：每行属于哪次模型回复（回档按回复原子截断靠它）。
    pub line_reply: Vec<u64>,
    /// 目的：每一行对应的行 id（与 `marks` / `line_reply` 同长）。
    pub line_ids: Vec<u64>,
    /// 目的：压缩点（`up_to` 之前的行由摘要代替）。
    pub compacted_upto: u64,
    /// 目的：压缩记录行的 id。
    pub compacted_line: u64,
    /// 目的：回复 id 的续号起点（转录里的最大值）。
    pub reply_seq: u64,
    /// 目的：上下文预算三元组（窗口、压缩阈值百分比、保留百分比）。
    pub context: (u64, u8, u8),
    /// 目的：本会话的模型（agent 没填就是核心默认）。
    pub model: Option<String>,
    /// 目的：过期内容自动清理的开关（落盘在 meta）。
    pub freshness: bool,
    /// 目的：读台账（正文 + 读事实），重建后与实时一致。
    pub reads: Vec<(String, ReadFact)>,
    /// 目的：最近一次调用的用量（状态栏与压缩判定读它）。
    pub usage: Option<ContextUsage>,
}

/// 目的：装配一条代理会话所需的全部事实（参数、席事实、通道、预算、模型与过期清理开关）。
pub struct ProxySpec<'a> {
    /// 目的：会话参数（身份是角色提示词 `core_proxy`）。
    pub params: SessionParams,
    /// 目的：这一席的工具面事实。
    pub member: MemberSpec<'a>,
    /// 目的：解析好的核心通道。
    pub channel: Channel,
    /// 目的：上下文预算三元组（窗口、压缩阈值百分比、保留百分比）。
    pub context: (u64, u8, u8),
    /// 目的：本会话的模型（None = 核心默认）。
    pub model: Option<String>,
    /// 目的：过期内容自动清理的开关（落盘在 meta）。
    pub freshness: bool,
}

/// 目的：装配一席的工具环境：发放名单过滤 → 工作区工具执行者 → 模块工具表与已就绪服务 → 说明块 → 工具面。
pub fn member_tools(ports: &SessionPorts, spec: &MemberSpec<'_>) -> MemberTools {
    let sb = spec.sandbox;
    // 角色表发放的系统工具 id 清单（工具面的名字部分），再按权限档位过滤。
    let book = ports.systools.book();
    let allowed = face_tools(
        ports
            .systools
            .tool_face(spec.role)
            .map(|f| f.into_iter().map(|(id, _)| id.to_string()).collect())
            .unwrap_or_default(),
        &book,
        &sb.permissions,
    );
    let handlers =
        crate::capabilities::workspace::api::work_handlers(&ports.workspace, sb, &allowed);
    // 工具面：静态模块工具 + **已就绪**常驻服务的操作（操作是服务跑起来才发现的，按模块现填）。
    let with_modules = ports.systools.allows_module_tools(spec.role)
        && sb.permissions.may_exec(CallScope::Sandbox);
    let mut table = tool_table(spec.modules);
    // 服务清单是全局的：只把**这个 agent 工具面里**的模块服务写进提示词。
    let started = if with_modules {
        service_ops_for_modules(
            service_ops_of(&ports.residents.services().unwrap_or_default()),
            spec.modules,
        )
    } else {
        BTreeMap::new()
    };
    let (module_tool_lines, service_param_lines) = serviced_tools(&started, &mut table);
    let mut notes = tool_notes(
        &*ports.prompt,
        sb,
        if with_modules { spec.modules } else { &[] },
        &service_param_lines,
    );
    if !module_tool_lines.is_empty() {
        notes.module_tools = format!("{}\n{}", notes.module_tools, module_tool_lines.join("\n"));
    }
    MemberTools {
        face: ToolFace {
            mode: spec.mode,
            modules: table,
            tier: spec.tier,
            guest_lease: spec.guest_lease.clone(),
            sandbox: sb.clone(),
            tools_this_turn: ports.prompt.text(Segment::ToolsThisTurn).to_string(),
            builtin_tools: ports.systools.book(),
            unavailable: spec.unavailable.clone(),
            // 围栏、模块注入项、运行路径与工具链根都由 turn 侧的 env 派生后拷进来（本文件不给第二份）。
            fence: spec.env.fence.clone(),
            module_env: spec.env.module_env.clone(),
            runtime_path: spec.env.runtime_path.clone(),
            runtime_roots: spec.env.runtime_roots.clone(),
            // 这一席的系统工具面**由角色表发放**（越权校验的唯一判据）：给什么写什么，代码里不留第二份名单。
            allowed,
            role: spec.role.to_string(),
            // 能不能用自己模块的工具由角色表发放；说明块素材在装配期算好（服务操作已在上面并进来）。
            with_modules,
            notes,
        },
        llm: Arc::clone(&ports.llm),
        log: Arc::clone(&ports.log),
        tools: Arc::clone(&ports.tools),
        vm: Arc::clone(&ports.vm),
        // 常驻服务的统一管理面：模块的 services 操作经它调用。
        residents: Arc::clone(&ports.residents),
        workspace: Some(Arc::clone(&ports.workspace)),
        observations: Observations::default(),
        shell_live: Some(Arc::clone(&spec.shell)),
        // 从零开始；按落盘转录重建时由调用方按转录里的最大值续号（见 rebuild_single）。
        reply_seq: 0,
        line: Default::default(),
        handlers,
    }
}

/// 目的：只取装配出的工具面（纯值，不带工具面的活对象）：给只看围栏 / 工具链根的测试用。
///   约束：生产路径都直接建会话（工具面归会话），所以它只在测试里编译。
#[cfg(test)]
pub fn member_face(ports: &SessionPorts, spec: &MemberSpec<'_>) -> ToolFace {
    member_tools(ports, spec).face
}

/// 目的：装配单 agent 会话对象（不插入会话中心；插入与落盘由调用方统一做）。
pub fn single_session(
    ports: &SessionPorts,
    spec: &SingleSpec<'_>,
) -> (AgentSession, Vec<SessionEvent>) {
    let a = spec.agent;
    let chat = ports.llm.member_channel(&spec.channel, &a.name);
    ports.log.info(
        "turn::session",
        &format!(
            "单 agent 工作：agent {}，模块 {}，模型 {}",
            a.name,
            a.modules.join("+"),
            spec.channel.model
        ),
    );
    // **会话参数**：身份块每回合由它现渲染（不存进消息列表）。
    let params = SessionParams::from_workspace(
        &a.name,
        spec.member.sandbox,
        spec.member.modules,
        "single",
        "solo",
    )
    .with_chara(a.chara.clone())
    .with_guest(spec.guest);
    // 单 agent 工作：用户自己开的那场对话（不是任务链的节点）——身份是 solo。
    let tools = member_tools(ports, &spec.member);
    let mut s = AgentSession::new(
        &a.name,
        params,
        chat,
        Some(tools),
        ports.prompt.text(Segment::CompactPrompt).to_string(),
    );
    // 自动压缩的预算按**这个 agent 的模型**窗口算（见 session/session-model.md 六）。
    let (window, percent, keep_percent) = spec.context;
    s.set_context_budget(window, percent, keep_percent);
    s.set_model(a.model.clone().or_else(|| spec.core_model.clone()));
    s.set_compact_factory(Arc::clone(&ports.compact));
    (s, Vec::new())
}

/// 目的：按落盘转录重建一席单 agent 会话的整场装配（不插入会话中心；插入与落盘由调用方统一做）。
/// 约束：回复 id 跨重启单调——从转录里的最大值续号，否则新回复会与旧回复并成一组。
pub fn rebuild_single(ports: &SessionPorts, spec: RebuildSpec<'_>) -> AgentSession {
    let id = spec.params.agent.clone();
    let chat = ports.llm.member_channel(&spec.channel, &id);
    let mut tools = member_tools(ports, &spec.member);
    tools.reply_seq = spec.reply_seq;
    let mut s = AgentSession::restore(
        &id,
        spec.params,
        spec.dialogue,
        spec.marks,
        spec.line_reply,
        spec.line_ids,
        spec.compacted_upto,
        spec.compacted_line,
        chat,
        Some(tools),
        ports.prompt.text(Segment::CompactPrompt).to_string(),
    );
    // 重建也要恢复上下文预算与最近一次用量：否则重启后自动压缩与状态栏都失忆。
    let (window, percent, keep_percent) = spec.context;
    s.set_context_budget(window, percent, keep_percent);
    s.set_model(spec.model);
    s.set_compact_factory(Arc::clone(&ports.compact));
    s.set_context_freshness(spec.freshness);
    for (body, fact) in spec.reads {
        s.note_read_at(body, fact);
    }
    s.set_last_usage(spec.usage);
    s
}

/// 目的：装配一条代理会话（核心身份、私有沙箱、core 通道）的整场装配。
pub fn proxy_session(ports: &SessionPorts, spec: ProxySpec<'_>) -> AgentSession {
    let id = spec.params.agent.clone();
    let chat = ports.llm.core_channel(&spec.channel);
    let tools = member_tools(ports, &spec.member);
    let mut s = AgentSession::new(
        &id,
        spec.params,
        chat,
        Some(tools),
        ports.prompt.text(Segment::CompactPrompt).to_string(),
    );
    let (window, percent, keep_percent) = spec.context;
    s.set_context_budget(window, percent, keep_percent);
    s.set_context_freshness(spec.freshness);
    s.set_model(spec.model);
    s.set_compact_factory(Arc::clone(&ports.compact));
    s
}
