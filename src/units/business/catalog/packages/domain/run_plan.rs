//! 目的：运行计划的派生规则——按执行规格、模块入口与包选型算出一份可执行计划（本机档与虚拟机档同一套目标选择）。
//! 管：`ExecPlan` 形状，`plan` / `diagnose_text_for` / `plan_summary`，`library_for` / `host_plan_for` / `plan_for_spec` / `unavailable_modules`。
//! 不管：取事实（根、宿主探针、包库、模块清单、镜像由调用方给）、选包实现（`domain/plan.rs`）、guest 生命周期（`vm`）。
//! 联动：被 `work` 的建会话 / 改配 / 视图与 `maintenance` 的构建消费；镜像派生在 `sandbox::api`。

use crate::kernel::ports::HostProbe;
use crate::units::business::catalog::modules::api::{EntryProblem, MissingEntry, Module};
use crate::units::business::catalog::packages::domain::manifest::{Library, library_for_guest};
use crate::units::business::catalog::packages::domain::plan::{
    BaseFacts, Facts, HostExec, PackagePlan, PinPolicy, PlannedPackage, diagnose_text, plan_guest,
    plan_host,
};
use crate::units::business::sandbox::api::{guest_platform_of, guest_runtimes_of};
use crate::units::pure::exec_model::api::{Side, TargetSpec};
use std::collections::BTreeMap;
use std::path::Path;

/// 目的：执行计划：运行时派生，从不落盘（装配阶段按 (id, version) 去包里定位内容）。
#[derive(Debug, Clone)]
pub struct ExecPlan {
    pub side: Side,
    pub net: bool,
    /// 目的：这次执行用的虚拟根名（本机档为空）——装配阶段按它解析出那一格目录。
    pub guest: Option<String>,
    /// 目的：装载顺序：先独立前缀（互不干扰），后写进系统路径的包（叠层）。
    pub packages: Vec<PlannedPackage>,
}

/// 目的：派生执行计划：入口判据与档位无关；运行包由选包规则选好，这里只负责落进 `ExecPlan`。
/// 约束：本机档的 `packages` 为空（运行包由 packages 注入本次 PATH，不进 guest 装配）；虚拟机档的包选择与诊断在 `plan_guest`。
pub fn plan(
    spec: &TargetSpec,
    packages: Vec<PlannedPackage>,
    missing: &[MissingEntry],
    net: bool,
) -> Result<ExecPlan, Vec<EntryProblem>> {
    // 入口判据与档位无关：模块根下没有那个能直接跑的产物，两档都当场拒绝（一次把话说全）。
    let entries: Vec<EntryProblem> = missing
        .iter()
        .map(|m| EntryProblem::MissingEntry {
            module: m.module.clone(),
            tool: m.tool.clone(),
            command: m.command.clone(),
            buildable: m.buildable,
        })
        .collect();
    if !entries.is_empty() {
        return Err(entries);
    }
    match spec.side {
        Side::Host => Ok(ExecPlan {
            side: Side::Host,
            net,
            guest: None,
            packages: Vec::new(),
        }),
        Side::Guest => Ok(ExecPlan {
            side: Side::Guest,
            net,
            guest: spec.guest.clone(),
            packages,
        }),
    }
}

/// 目的：同上，但"怎么补"按**目标环境**说：虚拟机档的入口要在那个虚拟根里编（宿主上编出来的产物在 guest 里跑不了）。
/// 参数：guest = 这次会话的虚拟根名（None = 本机档，在宿主编）。
pub fn diagnose_text_for(diags: &[EntryProblem], guest: Option<&str>) -> String {
    let lines: Vec<String> = diags
        .iter()
        .map(|d| match d {
            EntryProblem::MissingEntry {
                module,
                tool,
                command,
                buildable,
            } => {
                let how = match (buildable, guest) {
                    (true, Some(g)) => format!(
                        "跑 solomni --build={} --guest={} 让系统在虚拟根里照模块声明编一次（宿主上编出来的产物在 guest 里跑不了）",
                        module, g
                    ),
                    (true, None) => {
                        format!("跑 solomni --build={} 让系统照模块声明的构建方法编一次", module)
                    }
                    (false, _) => {
                        "模块要自带这个产物，或先按模块自己的 README 构建一次".to_string()
                    }
                };
                format!("模块 {} 的工具 {} 入口不在场：{}（{}）", module, tool, command, how)
            }
        })
        .collect();
    lines.join("；")
}

/// 目的：计划的一句话摘要（日志用）：只报事实，不含真实路径。
pub fn plan_summary(plan: &ExecPlan) -> String {
    let net = if plan.net { "放行" } else { "不放行" };
    if plan.side == Side::Host {
        return format!("执行档位 本机（不装载运行包）；网络 {}", net);
    }
    let pkgs: Vec<String> = plan
        .packages
        .iter()
        .map(|p| format!("{}@{}（{}）", p.id, p.version, p.kind))
        .collect();
    format!(
        "执行档位 虚拟机；虚拟根 {}；装载 {} 个包：{}；网络 {}",
        plan.guest.as_deref().unwrap_or("（未指定）"),
        plan.packages.len(),
        pkgs.join("、"),
        net
    )
}

/// 目的：这次会话该用的包库——按 guest 平台筛过（分平台包只留与 guest 匹配的那份）。
pub fn library_for(
    lib: &Library,
    root: &Path,
    probe: &dyn HostProbe,
    spec: &TargetSpec,
) -> Library {
    library_for_guest(lib, &guest_platform_of(root, probe, spec))
}

/// 目的：本机档的包计划：按宿主目标选包、产出 PATH/只读根；会话用 Require，无会话面用 First。
pub fn host_plan_for(
    lib: &Library,
    base: &BaseFacts,
    probe: &dyn HostProbe,
    spec: &TargetSpec,
    modules: &[Module],
    policy: PinPolicy,
) -> PackagePlan {
    let current_path = probe.path();
    let facts = Facts {
        probe: &HostExec(probe),
        current_path: &current_path,
        base: &base.active,
        host_fallback: &base.fallback,
    };
    plan_host(spec, modules, lib, policy, &facts)
}

/// 目的：按一份执行规格与模块清单派生计划：入口判据先于包选型（与旧的单一计划函数同一顺序）。
#[allow(clippy::too_many_arguments)]
pub fn plan_for_spec(
    root: &Path,
    probe: &dyn HostProbe,
    lib: &Library,
    spec: &TargetSpec,
    modules: &[Module],
    missing: &[MissingEntry],
    guest: Option<&str>,
    net: bool,
) -> Result<ExecPlan, String> {
    // 一次把话说全：入口判据在前，包选型问题在后（与旧的单一计划函数同一顺序）。
    let entry_diags = plan(spec, Vec::new(), missing, net)
        .err()
        .unwrap_or_default();
    let package_plan = if spec.side == Side::Guest {
        plan_guest(
            spec,
            modules,
            &library_for(lib, root, probe, spec),
            &guest_runtimes_of(root, probe, spec),
        )
    } else {
        PackagePlan::default()
    };
    let mut parts: Vec<String> = Vec::new();
    if !entry_diags.is_empty() {
        parts.push(diagnose_text_for(&entry_diags, guest));
    }
    if !package_plan.problems.is_empty() {
        parts.push(diagnose_text(&package_plan.problems));
    }
    if !parts.is_empty() {
        return Err(parts.join("；"));
    }
    plan(spec, package_plan.packages, &[], net).map_err(|diags| diagnose_text_for(&diags, guest))
}

/// 目的：本档位下不能执行工具的模块（模块 id → 缺的能力名）：建会话与重建时收口给工具环境。
pub fn unavailable_modules(
    root: &Path,
    probe: &dyn HostProbe,
    lib: &Library,
    base: &BaseFacts,
    spec: &TargetSpec,
    modules: &[Module],
) -> BTreeMap<String, Vec<String>> {
    if spec.side == Side::Guest {
        plan_guest(
            spec,
            modules,
            &library_for(lib, root, probe, spec),
            &guest_runtimes_of(root, probe, spec),
        )
        .unusable
    } else {
        host_plan_for(lib, base, probe, spec, modules, PinPolicy::Require).unusable
    }
}
