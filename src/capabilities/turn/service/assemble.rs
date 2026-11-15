//! 目的：一席会话的装配构件——把各能力的能力面与已取事实装成成员工具面与单 agent 会话对象，
//!   并把历史会话按落盘重建为活动会话（`ensure_session`）。
//! 管：`SessionDeps`（装配要的十个能力面）、`MemberSpec` / `SingleSpec`（这一席 / 这一场的事实）、
//!   `member_tools`（工具面 + 围栏 + 工作工具处理器）、`single_session`（单 agent 会话对象）、
//!   `ensure_session`（在世表里没有就按落盘重建一份）。
//! 不管：事实从哪来（`facts.rs` / `plan.rs` / `tree.rs` 现取）、落盘、生成驱动、工作工具的执行。
//! 联动：`conductor` 经 `turn::api` 取用；纯规则在 `turn::domain::assemble`；围栏派生在 `env` 能力；
//!   成员侧工作工具处理器按放行名单现取 `workspace::api::work_handlers`。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::capabilities::llm::api::{Channel, Llm, ToolMode};
use crate::capabilities::modules::api::Module;
use crate::capabilities::packages::api::PackagePlan;
use crate::capabilities::prompt::api::{Prompt, Segment};
use crate::capabilities::registry::api::Registry;
use crate::capabilities::residents::api::ResidentOps;
use crate::capabilities::secrets::api::SecretOps;
use crate::capabilities::session::api::{
    AgentMeta, AgentSession, CompactChatFactory, MemberTools, SessionEvent, ShellSlot,
};
use crate::capabilities::tools::api::{ToolExec, Tools};
use crate::capabilities::turn::domain::assemble::{face_tools, service_ops_of, serviced_tools};
use crate::capabilities::vm::api::{GuestOs, Vm};
use crate::capabilities::workspace::api::{Sandbox, Workspace};
use crate::kernel::api::Tier;
use crate::kernel::ports::{HostProbe, Log};

/// 目的：装配要的十个能力面——组合根注入一次，装配期只按面取用（不碰端口）。
pub struct SessionDeps {
    pub llm: Arc<dyn Llm + Send + Sync>,
    pub log: Arc<dyn Log + Send + Sync>,
    pub prompt: Arc<dyn Prompt>,
    pub systools: Arc<dyn Tools>,
    pub tools: Arc<dyn ToolExec + Send + Sync>,
    pub vm: Arc<dyn Vm + Send + Sync>,
    pub residents: Arc<dyn ResidentOps + Send + Sync>,
    pub workspace: Arc<dyn Workspace + Send + Sync>,
    pub secrets: Arc<dyn SecretOps + Send + Sync>,
    // 压缩通道工厂：会话在压缩触发时现取，不刷新会话。
    pub compact: Arc<dyn CompactChatFactory>,
}

/// 目的：一席工具面要用的事实——模块、沙箱、缺包、网络、形态、角色、档位、租约与装配落点。
/// 约束：事实全部只读借用；成员侧工作工具处理器由 `member_tools` 按放行名单与本席沙箱现取。
pub struct MemberSpec<'a> {
    // 这一席声明的模块（模块工具与隐私注入项按它算）。
    pub modules: &'a [Module],
    // 该 agent 的沙箱（可达范围与权限面）。
    pub sandbox: &'a Sandbox,
    // 本档位下缺运行包、不能执行工具的模块（机制侧据此拒绝执行）。
    pub unavailable: BTreeMap<String, Vec<String>>,
    // 会话是否放行出站网络。
    pub net: bool,
    // 工具形态（信封 / 原生）。
    pub mode: ToolMode,
    // 这一席的身份（solo / executor / core_proxy…）。
    pub role: &'a str,
    // 执行档位（宿主 / 虚拟机）。
    pub tier: Tier,
    // 共享 guest 租约（空 = 本机档 / 没有 guest）。
    pub guest_lease: String,
    // 本机运行包的安装计划（包根与运行路径）。
    pub plan: &'a PackagePlan,
    // 基础工具链根（只在宿主档放行）。
    pub base_roots: &'a [PathBuf],
    // 用户显式授权的主机只读根。
    pub host_read_roots: Vec<PathBuf>,
    // 本会话的 shell 状态槽（同一会话共享一份）。
    pub shell: Arc<ShellSlot>,
}

/// 目的：一场单 agent 会话要用的事实。
pub struct SingleSpec<'a> {
    // 要装配的 agent（名字、人设、模块、模型）。
    pub agent: &'a AgentMeta,
    // 这一席的工具面事实。
    pub member: MemberSpec<'a>,
    // 已解析好的模型通道。
    pub channel: Channel,
    // 这场会话跑在哪种 guest 里（本机档 = None）。
    pub guest: Option<GuestOs>,
    // 上下文预算：窗口、压缩阈值百分比、保留百分比。
    pub context: (u64, u8, u8),
    // 核心默认模型（agent 没指定模型时用它）。
    pub core_model: Option<String>,
}

/// 目的：装一席工具面：发放的系统工具 + 模块工具 + 已就绪服务操作 + 围栏 + 处理器。
/// 约束：deny 的工具不发放；模块工具只在角色表允许且执行不被禁时出现。
pub fn member_tools(deps: &SessionDeps, spec: &MemberSpec<'_>) -> MemberTools {
    let book = deps.systools.book();
    let role_face = deps
        .systools
        .tool_face(spec.role)
        .map(|f| f.into_iter().map(|(id, _)| id.to_string()).collect())
        .unwrap_or_default();
    let allowed = face_tools(role_face, &book, &spec.sandbox.permissions);
    // 成员侧工作工具：按本回合放行的名字现取（只认 work_*，其余名字不归它管）。
    let handlers =
        crate::capabilities::workspace::api::work_handlers(&deps.workspace, spec.sandbox, &allowed);
    // 工具面：静态模块工具 + **已就绪**常驻服务的操作（操作是服务跑起来才发现的，按模块现填）。
    let with_modules = deps.systools.allows_module_tools(spec.role)
        && !spec.sandbox.permissions.decisions.exec_sandbox.is_deny();
    let mut table = crate::capabilities::session::api::tool_table(spec.modules);
    // 服务清单是全局的：只把**这个 agent 工具面里**的模块服务写进提示词。
    let started = if with_modules {
        crate::capabilities::session::api::service_ops_for_modules(
            service_ops_of(&deps.residents.services().unwrap_or_default()),
            spec.modules,
        )
    } else {
        BTreeMap::new()
    };
    let (module_tool_lines, service_param_lines) = serviced_tools(&started, &mut table);
    let mut notes = crate::capabilities::tools::api::tool_notes(
        &*deps.prompt,
        spec.sandbox,
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
    let member =
        crate::capabilities::env::api::member(&crate::capabilities::env::api::MemberFacts {
            sandbox: spec.sandbox,
            host_read_roots: &spec.host_read_roots,
            net: spec.net,
            tier: spec.tier,
            base_roots: spec.base_roots,
            plan: spec.plan,
            module_vars: &module_vars,
        });
    MemberTools {
        mode: spec.mode,
        modules: table,
        observations: crate::capabilities::tools::api::Observations::default(),
        llm: Arc::clone(&deps.llm),
        log: Arc::clone(&deps.log),
        tools: Arc::clone(&deps.tools),
        tier: spec.tier,
        guest_lease: spec.guest_lease.clone(),
        vm: Arc::clone(&deps.vm),
        sandbox: spec.sandbox.clone(),
        tools_this_turn: deps.prompt.text(Segment::ToolsThisTurn).to_string(),
        builtin_tools: book,
        unavailable: spec.unavailable.clone(),
        // 围栏：可达范围 + 断网 + 环境白名单的落点，全部由该 agent 的沙箱派生（机制在 adapters）；
        // 只读根来自用户显式授权（`sandbox.policy.host_read_roots`）与本机运行包的包根。
        fence: member.fence,
        module_env: member.module_env,
        runtime_path: member.runtime_path,
        shell_live: Some(Arc::clone(&spec.shell)),
        workspace: Some(Arc::clone(&deps.workspace)),
        // 基础工具链根只在宿主档放行：VM 的工具链由 guest 的只读挂载提供，宿主路径在 guest 里不存在。
        runtime_roots: member.runtime_roots,
        // 从零开始；按落盘转录重建时由调用方按转录里的最大值续号。
        reply_seq: 0,
        line: Default::default(),
        // 这一席的系统工具面**由角色表发放**（越权校验的唯一判据）：给什么写什么，代码里不留第二份名单。
        allowed,
        role: spec.role.to_string(),
        // 能不能用自己模块的工具由角色表发放；说明块素材在装配期算好（服务操作已并进来）。
        with_modules,
        notes,
        handlers,
        // 常驻服务的统一管理面：模块的 services 操作经它调用。
        residents: Arc::clone(&deps.residents),
    }
}

/// 目的：装配单 agent 会话（不插入会话中心；插入与落盘由调用方统一做）。
/// 约束：会话参数每回合现渲染身份块（不存进消息列表）；自动压缩预算按**这个 agent 的模型**窗口算。
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
    let params = crate::capabilities::session::api::SessionParams::from_workspace(
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
    let mut s = crate::capabilities::session::api::AgentSession::new(
        &a.name,
        params,
        chat,
        Some(tools),
        deps.prompt.text(Segment::CompactPrompt).to_string(),
    );
    s.set_context_budget(spec.context.0, spec.context.1, spec.context.2);
    s.set_model(a.model.clone().or_else(|| spec.core_model.clone()));
    s.set_compact_factory(Arc::clone(&deps.compact));
    (s, Vec::new())
}

impl crate::capabilities::turn::service::session::Turn {
    /// 目的：历史会话转为活动会话（跨重启续跑 / 回档的前提）：按转录重建，状态全部派生。
    /// 参数：`sid` = 会话 id；`registry` = 登记处；`probe` = 宿主能力探测。
    /// 错误：会话不在盘上、重建失败时原样报错（文案由重建路径给出）。
    /// 约束：会话对象正被工作线程拿着时**不能**从盘上再建一份（会变成两个实例）——如实报"正在生成中"。
    pub(crate) fn ensure_session(
        &mut self,
        sid: &str,
        registry: &dyn Registry,
        probe: &dyn HostProbe,
    ) -> Result<(), String> {
        if self.holds(sid) {
            return Ok(());
        }
        // 会话对象在工作线程上（生成中）：**不能**从盘上再建一份（会变成两个实例）。
        if self.is_running(sid) {
            return Err(crate::capabilities::turn::service::session::Turn::running_refusal(sid));
        }
        let (meta, events) = crate::capabilities::lifecycle::api::open(&*self.history, sid)?;
        let rebuilt = self.rebuild_session(&meta, &events, registry, probe)?;
        self.put(sid, rebuilt);
        Ok(())
    }
}
