//! 目的：环境事实的派生规则——把调用方已取好的事实算成权限面、沙箱清单、围栏、进程环境与工具链根。
//! 管：`member`（一席的围栏 / 运行路径 / 工具链根 / 模块注入项）、`sandboxes`（每个 agent 的沙箱）、
//!   `permissions`（权限链）、`workspace_grant`（共享区授权粒度）、`network_allowed` 与
//!   `host_read_roots`（全局策略派生的网络开关与主机只读根）。
//! 不管：取事实（history / workspace / registry / 清单在 `conductor`）、落盘、启停 guest 与工具进程。
//! 联动：`conductor/service/env.rs` 取好事实后转调这里；行为与错误文案必须逐字一致。

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use crate::capabilities::modules::api::Roster;
use crate::capabilities::packages::api::PackagePlan;
use crate::capabilities::permission::api::PermissionSet;
use crate::capabilities::registry::api::AppSettings;
use crate::capabilities::session::api::{AgentMeta, SessionMeta, WorkspaceGrant};
use crate::capabilities::workspace::api::{Sandbox, Sandboxes, WorkRoots};
use crate::kernel::api::{FenceSpec, Tier};
use crate::kernel::ports::PathCodec;

/// 目的：派生一席工具环境所需的全部已取事实。
pub struct MemberFacts<'a> {
    /// 目的：该 agent 的沙箱（可达范围与权限面）。
    pub sandbox: &'a Sandbox,
    /// 目的：用户显式授权的主机只读根。
    pub host_read_roots: &'a [PathBuf],
    /// 目的：该会话是否放行出站网络。
    pub net: bool,
    /// 目的：执行档位（宿主 / 虚拟机）。
    pub tier: Tier,
    /// 目的：基础工具链根（只在宿主档放行）。
    pub base_roots: &'a [PathBuf],
    /// 目的：本机运行包的安装计划（包根与运行路径）。
    pub plan: &'a PackagePlan,
    /// 目的：各模块已解析的隐私注入项。
    pub module_vars: &'a BTreeMap<String, Vec<(String, String)>>,
}

/// 目的：一席工具环境里由事实派生的部分。
pub struct MemberEnv {
    /// 目的：该 agent 的围栏（可达范围 + 断网 + 环境白名单落点）。
    pub fence: FenceSpec,
    /// 目的：工具进程的 PATH 入口（本机运行包换过 PATH 时才有）。
    pub runtime_path: Option<OsString>,
    /// 目的：工具链根：宿主档 = 基础根 + 包根，虚拟机档 = 包根。
    pub runtime_roots: Vec<PathBuf>,
    /// 目的：各模块隐私字段的注入项（模块 id → 变量对）。
    pub module_env: BTreeMap<String, Vec<(String, String)>>,
}

/// 目的：组装沙箱清单所需的已取事实。
pub struct SandboxesFacts<'a> {
    /// 目的：顶层工作名（整棵树只有一个 work/）。
    pub work: &'a str,
    /// 目的：会话名（写进每份沙箱）。
    pub session: &'a str,
    /// 目的：这份会话的 agent 名单。
    pub agents: &'a [AgentMeta],
    /// 目的：工作区端口给出的根（共享区、会话根、每个 agent 的私有根）。
    pub roots: &'a WorkRoots,
    /// 目的：模块清单（模块 id → 目录与是否有 userdata）。
    pub roster: &'a Roster,
    /// 目的：虚拟机档给 guest↔宿主的路径翻译器。
    pub codec: Option<Arc<dyn PathCodec>>,
    /// 目的：外部工作区的授权粒度。
    pub grant: WorkspaceGrant,
}

/// 目的：派生一席的围栏、运行路径、工具链根与模块注入项；纯函数，不取事实、不落盘。
/// 参数：`facts` = 调用方已取好的事实（沙箱、只读根、网络、档位、基础根、包计划、模块机密）。
/// 返回：围栏（只读树并进包根、去重保序）、PATH 入口、工具链根、模块注入项。
pub fn member(facts: &MemberFacts<'_>) -> MemberEnv {
    let mut fence = FenceSpec::for_session(facts.sandbox, facts.host_read_roots, facts.net);
    fence.ro_tree.extend(facts.plan.roots.iter().cloned());
    fence.ro_tree.sort();
    fence.ro_tree.dedup();
    let mut runtime_roots = if facts.tier == Tier::Host {
        facts.base_roots.to_vec()
    } else {
        Vec::new()
    };
    runtime_roots.extend(facts.plan.roots.iter().cloned());
    runtime_roots.sort();
    runtime_roots.dedup();
    MemberEnv {
        fence,
        runtime_path: facts.plan.path.clone(),
        runtime_roots,
        module_env: facts.module_vars.clone(),
    }
}

/// 目的：按 agent 名单组装沙箱清单；模块目录只对使用它的 agent 可达。
/// 参数：`facts` = 已取好的工作名、名单、工作区根、模块清单、路径翻译器与授权粒度；
///   `permissions` = 某个 agent 的生效权限（协作主会话的发言席要按子会话 narrowing 取）。
/// 错误：工作区没给出某个 agent 的私有根时逐字报「工作区没有给出 agent {名} 的沙箱路径」。
pub fn sandboxes(
    facts: &SandboxesFacts<'_>,
    permissions: &dyn Fn(&str) -> Result<PermissionSet, String>,
) -> Result<Sandboxes, String> {
    let mut list: Vec<Sandbox> = Vec::new();
    for a in facts.agents {
        let private = facts
            .roots
            .agents
            .get(&a.name)
            .cloned()
            .ok_or_else(|| format!("工作区没有给出 agent {} 的沙箱路径", a.name))?;
        let mut modules = BTreeMap::new();
        let mut modules_with_userdata = BTreeSet::new();
        for id in &a.modules {
            if let Some(m) = facts.roster.modules.iter().find(|m| &m.manifest.id == id) {
                modules.insert(id.clone(), m.root.clone());
                if m.has_userdata {
                    modules_with_userdata.insert(id.clone());
                }
            }
        }
        list.push(Sandbox {
            work_name: facts.work.to_string(),
            session: facts.session.to_string(),
            agent: a.name.clone(),
            shared: facts.roots.shared.clone(),
            private,
            modules,
            modules_with_userdata,
            permissions: permissions(&a.name)?,
            path_codec: facts.codec.clone(),
            shared_grant: facts.grant,
        });
    }
    Ok(Sandboxes {
        shared: facts.roots.shared.clone(),
        session_root: facts.roots.session_root.clone(),
        list,
        codec: facts.codec.clone(),
    })
}

/// 目的：一条会话的生效权限：全局默认 → 根 ceiling → 每层 narrowing → 全局硬黑名单。
/// 约束：授权粒度为 `none` 时共享区读写一并收紧为 none（内置读工具与 shell 拿到同一份事实）。
pub fn permissions(chain: &[SessionMeta], app: &AppSettings) -> Result<PermissionSet, String> {
    let root = chain.first().ok_or_else(|| "会话权限链为空".to_string())?;
    let mut out = app.sandbox.defaults.to_permission_set();
    if let Some(layer) = root.sandbox.root_layer()? {
        out = out.apply_root(layer)?;
    }
    for child in chain.iter().skip(1) {
        if let Some(layer) = child.sandbox.child_layer()? {
            out = out.apply_narrowing(layer)?;
        }
    }
    let out = out.with_hard_deny(&app.sandbox.policy.deny);
    if workspace_grant(chain) == WorkspaceGrant::None {
        return Ok(out.without_shared());
    }
    Ok(out)
}

/// 目的：这条配置是否放行出站网络（工具 / guest / 常驻服务 / 构建都看它）。
pub fn network_allowed(app: &AppSettings) -> bool {
    app.sandbox.policy.network
}

/// 目的：用户显式授权的主机只读根（`settings.yaml` 的 `sandbox.policy.host_read_roots`）。
/// 约束：逐条去空白、丢掉空项再转路径——空串不是"根"，一个都不放行时给空表
///   （只读位怎么落由适配层定，这里只给事实）。
pub fn host_read_roots(app: &AppSettings) -> Vec<PathBuf> {
    app.sandbox
        .policy
        .host_read_roots
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// 目的：这条会话生效的外部工作区授权粒度：链上最近一份绑定说了算，没有绑定就是 `session` 缺省。
pub fn workspace_grant(chain: &[SessionMeta]) -> WorkspaceGrant {
    chain
        .iter()
        .rev()
        .find_map(|m| m.workspace.as_ref().map(|b| b.grant))
        .unwrap_or(WorkspaceGrant::Session)
}
