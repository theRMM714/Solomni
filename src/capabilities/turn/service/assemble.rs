//! 目的：一席工具面与单 agent 会话对象的装配——把能力面的事实拼成能跑的会话。
//! 管：`SessionDeps` / `MemberSpec` / `SingleSpec` 三个参数对象，`member_tools` / `single_session` 两个装配入口，
//!   以及 `Turn::build_single`（单 agent 会话的整场装配）。
//! 不管：落盘（`session` 能力）、生成驱动（`conductor`）、纯派生规则（在 `turn::domain::assemble`）。
//! 联动：`work` 的建工作两段经 `Turn::build_single` 装配会话；能力面与 shell 槽由 `Turn` 提供。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::capabilities::env::api::{MemberFacts, member};
use crate::capabilities::llm::api::{Channel, Llm, ToolMode};
use crate::capabilities::modules::api::Module;
use crate::capabilities::packages::api::PackagePlan;
use crate::capabilities::prompt::api::{Prompt, Segment};
use crate::capabilities::registry::api::Registry;
use crate::capabilities::residents::api::ResidentOps;
use crate::capabilities::secrets::api::SecretOps;
use crate::capabilities::session::api::{
    AgentMeta, AgentSession, CompactChatFactory, MemberTools, SessionEvent, SessionParams,
    ShellSlot, ToolFace, service_ops_for_modules, tool_table,
};
use crate::capabilities::tools::api::{ToolExec, Tools, tool_notes};
use crate::capabilities::turn::domain::assemble::{face_tools, service_ops_of, serviced_tools};
use crate::capabilities::turn::service::session::Turn;
use crate::capabilities::vm::api::{GuestOs, Vm};
use crate::capabilities::workspace::api::{Sandbox, Workspace};
use crate::kernel::api::Tier;
use crate::kernel::ports::Log;

/// 目的：装配一席要用的能力面（与 `Turn` 持有的那几份同一批）。
pub struct SessionDeps {
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
    /// 目的：隐秘字段管理面：装配期按模块解析注入项。
    pub secrets: Arc<dyn SecretOps + Send + Sync>,
    /// 目的：压缩通道工厂：新会话与重建会话共用同一份。
    pub compact: Arc<dyn CompactChatFactory>,
}

/// 目的：装配一席工具面所需的全部事实（模块、沙箱、形态、档位、角色、围栏素材与 shell 槽）。
pub struct MemberSpec<'a> {
    /// 目的：这一席挂的模块（工具表与隐私注入项按它算）。
    pub modules: &'a [Module],
    /// 目的：这一席的沙箱（工作名、agent 实例名与生效权限）。
    pub sandbox: &'a Sandbox,
    /// 目的：本档位下缺运行包、不能执行工具的模块（机制侧据此拒绝执行）。
    pub unavailable: BTreeMap<String, Vec<String>>,
    /// 目的：会话是否放行出站网络（随围栏交给机制层）。
    pub net: bool,
    /// 目的：工具形态（原生 / 信封）。
    pub mode: ToolMode,
    /// 目的：这一席的身份（用户建的单 agent 会话 = solo，协作子会话 = executor）。
    pub role: &'a str,
    /// 目的：档位（本机 / 虚拟机）。
    pub tier: Tier,
    /// 目的：这一席的 guest 租约（本机档为空串）。
    pub guest_lease: String,
    /// 目的：运行包计划（工具链根与围栏素材按它算）。
    pub plan: &'a PackagePlan,
    /// 目的：项目内基础工具链的只读安装根。
    pub base_roots: &'a [PathBuf],
    /// 目的：用户显式授权 + 本机运行包包根的只读根。
    pub host_read_roots: Vec<PathBuf>,
    /// 目的：这一席的 shell 活进程槽（成员工具面与状态栏共用同一份）。
    pub shell: Arc<ShellSlot>,
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

/// 目的：装配一席的工具环境：发放名单过滤 → 工作区工具执行者 → 模块工具表与已就绪服务 → 说明块 → 工具面。
pub fn member_tools(deps: &SessionDeps, spec: &MemberSpec<'_>) -> MemberTools {
    let sb = spec.sandbox;
    // 角色表发放的系统工具 id 清单（工具面的名字部分），再按权限档位过滤。
    let book = deps.systools.book();
    let allowed = face_tools(
        deps.systools
            .tool_face(spec.role)
            .map(|f| f.into_iter().map(|(id, _)| id.to_string()).collect())
            .unwrap_or_default(),
        &book,
        &sb.permissions,
    );
    let handlers =
        crate::capabilities::workspace::api::work_handlers(&deps.workspace, sb, &allowed);
    // 工具面：静态模块工具 + **已就绪**常驻服务的操作（操作是服务跑起来才发现的，按模块现填）。
    let with_modules = deps.systools.allows_module_tools(spec.role)
        && !sb.permissions.decisions.exec_sandbox.is_deny();
    let mut table = tool_table(spec.modules);
    // 服务清单是全局的：只把**这个 agent 工具面里**的模块服务写进提示词。
    let started = if with_modules {
        service_ops_for_modules(
            service_ops_of(&deps.residents.services().unwrap_or_default()),
            spec.modules,
        )
    } else {
        BTreeMap::new()
    };
    let (module_tool_lines, service_param_lines) = serviced_tools(&started, &mut table);
    let mut notes = tool_notes(
        &*deps.prompt,
        sb,
        if with_modules { spec.modules } else { &[] },
        &service_param_lines,
    );
    if !module_tool_lines.is_empty() {
        notes.module_tools = format!("{}\n{}", notes.module_tools, module_tool_lines.join("\n"));
    }
    // 各模块隐私字段的注入项：装配期按模块解析一次，起工具进程时只走环境（解析失败 = 该模块没有可用值）。
    let module_vars: BTreeMap<String, Vec<(String, String)>> = spec
        .modules
        .iter()
        .map(|m| {
            (
                m.manifest.id.clone(),
                deps.secrets.resolve(&m.manifest.id).unwrap_or_default(),
            )
        })
        .collect();
    // 一席的工具环境事实（围栏 / 运行路径 / 工具链根 / 模块注入项）由 env 一处派生：
    // 只读根 = 用户显式授权 + 本机运行包包根；工具链基础根只在宿主档放行。
    let menv = member(&MemberFacts {
        sandbox: sb,
        host_read_roots: &spec.host_read_roots,
        net: spec.net,
        tier: spec.tier,
        base_roots: spec.base_roots,
        plan: spec.plan,
        module_vars: &module_vars,
    });
    MemberTools {
        face: ToolFace {
            mode: spec.mode,
            modules: table,
            tier: spec.tier,
            guest_lease: spec.guest_lease.clone(),
            sandbox: sb.clone(),
            tools_this_turn: deps.prompt.text(Segment::ToolsThisTurn).to_string(),
            builtin_tools: deps.systools.book(),

            unavailable: spec.unavailable.clone(),
            // 围栏、模块注入项、运行路径与工具链根都由 env 从事实派生（本文件不给第二份）。
            fence: menv.fence,
            module_env: menv.module_env,
            runtime_path: menv.runtime_path,
            runtime_roots: menv.runtime_roots,
            // 这一席的系统工具面**由角色表发放**（越权校验的唯一判据）：给什么写什么，代码里不留第二份名单。
            allowed,
            role: spec.role.to_string(),
            // 能不能用自己模块的工具由角色表发放；说明块素材在装配期算好（服务操作已在上面并进来）。
            with_modules,
            notes,
        },
        llm: Arc::clone(&deps.llm),
        log: Arc::clone(&deps.log),
        tools: Arc::clone(&deps.tools),
        vm: Arc::clone(&deps.vm),
        // 常驻服务的统一管理面：模块的 services 操作经它调用。
        residents: Arc::clone(&deps.residents),
        workspace: Some(Arc::clone(&deps.workspace)),
        observations: crate::capabilities::tools::api::Observations::default(),
        shell_live: Some(Arc::clone(&spec.shell)),
        // 从零开始；按落盘转录重建时由调用方按转录里的最大值续号（见 rebuild_session）。
        reply_seq: 0,
        line: Default::default(),
        handlers,
    }
}

/// 目的：装配单 agent 会话对象（不插入会话中心；插入与落盘由调用方统一做）。
pub fn single_session(
    deps: &SessionDeps,
    spec: &SingleSpec<'_>,
) -> (AgentSession, Vec<SessionEvent>) {
    let a = spec.agent;
    let chat = deps.llm.member_channel(&spec.channel, &a.name);
    deps.log.info(
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
    let tools = member_tools(deps, &spec.member);
    let mut s = AgentSession::new(
        &a.name,
        params,
        chat,
        Some(tools),
        deps.prompt.text(Segment::CompactPrompt).to_string(),
    );
    // 自动压缩的预算按**这个 agent 的模型**窗口算（见 session/session-model.md 六）。
    let (window, percent, keep_percent) = spec.context;
    s.set_context_budget(window, percent, keep_percent);
    s.set_model(a.model.clone().or_else(|| spec.core_model.clone()));
    s.set_compact_factory(Arc::clone(&deps.compact));
    (s, Vec::new())
}

/// 目的：单 agent 会话的整场装配——取本席事实、解析形态与预算，交给 `single_session`。
/// 约束：不插入会话中心；插入与落盘由调用方（`work` 的提交段）统一做。
impl Turn {
    // 会话装配把这一场单 agent 会话一次说全；收口成参数对象只是把参数挪个地方。
    // 这是有意的设计取舍（见 docs/testing/quality-isolation.md 的 allow 清单），不是没修。
    #[allow(clippy::too_many_arguments)]
    pub fn build_single(
        &self,
        registry: &dyn Registry,
        a: &AgentMeta,
        modules: &[Module],
        channel: Channel,
        sb: &Sandbox,
        unavailable: BTreeMap<String, Vec<String>>,
        net: bool,
        tier: Tier,
        guest: Option<GuestOs>,
        guest_lease: String,
        host_plan: &PackagePlan,
    ) -> (AgentSession, Vec<SessionEvent>) {
        // 形态按登记处解析；通道已在前置判据里解析成功，这里不再有"没有通道"的分支。
        let spec = SingleSpec {
            agent: a,
            member: MemberSpec {
                modules,
                sandbox: sb,
                unavailable,
                net,
                mode: registry.tool_mode(a.model.as_deref()),
                // 单 agent 工作：用户自己开的那场对话（不是任务链的节点）——身份是 solo。
                role: "solo",
                tier,
                guest_lease,
                plan: host_plan,
                base_roots: &self.base_roots,
                host_read_roots: crate::capabilities::env::api::host_read_roots(registry.app()),
                shell: self.shell_slot(&sb.session),
            },
            channel,
            guest,
            context: self.context_budget(registry, a.model.as_deref()),
            core_model: registry.core_model(),
        };
        single_session(&self.session_deps(), &spec)
    }
}
