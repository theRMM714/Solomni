//! 目的：运行计划的派生规则——按执行规格与已取事实算出 guest 镜像 / 平台 / OS / 路径翻译器 / 自带运行能力、
//!   宿主与 guest 的包选型、模块可用性与完整执行计划。
//! 管：`guest_image_of` / `guest_platform_of` / `guest_os_of` / `path_codec_of` / `guest_runtimes_of` /
//!   `library_for` / `host_plan_for` / `plan_for_spec` / `unavailable_modules`——全部纯派生，事实由调用方显式传入。
//! 不管：取事实（根、宿主探针、包库、模块清单由调用方给）、选包实现（`packages` 能力）、guest 生命周期（`service.rs`）。
//! 联动：被 `runtime` 的建工作 / 改配 / 视图 / 重建会话消费；经 `vm::api` 导出。

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use crate::capabilities::modules::api::Module;
use crate::capabilities::packages::api as pkgs;
use crate::capabilities::packages::api::{BaseFacts, Library, PackagePlan, PinPolicy};
use crate::kernel::api::{ExecSpec, Tier};
use crate::kernel::ports::{HostProbe, PathCodec};

/// 目的：这次会话的 guest 镜像（本机档或选定虚拟根判不出来 = None）——平台名与 guest OS 都从它派生。
/// 约束：虚拟根名先经 `resolve_guest` 解析成那一格目录，再按布局判定；两者任一失败都给 None。
pub fn guest_image_of(
    root: &Path,
    probe: &dyn HostProbe,
    spec: &ExecSpec,
) -> Option<crate::capabilities::vm::api::GuestImage> {
    if spec.tier != Tier::Vm {
        return None;
    }
    let name = spec.guest.as_deref()?;
    let dir = crate::capabilities::vm::api::resolve_guest(root, name).ok()?;
    crate::capabilities::vm::api::classify_base(&dir, probe).ok()
}

/// 目的：这次虚拟机档会话的 guest 平台名（os-arch）；本机档或判不出来给空串。
pub fn guest_platform_of(root: &Path, probe: &dyn HostProbe, spec: &ExecSpec) -> String {
    guest_image_of(root, probe, spec)
        .map(|image| crate::capabilities::vm::api::guest_platform(image.os, image.arch))
        .unwrap_or_default()
}

/// 目的：这次虚拟机档会话的 guest OS（本机档或判不出来 = None）——工作环境块按它选 guest 档。
pub fn guest_os_of(
    root: &Path,
    probe: &dyn HostProbe,
    spec: &ExecSpec,
) -> Option<crate::capabilities::vm::api::GuestOs> {
    guest_image_of(root, probe, spec).map(|image| image.os)
}

/// 目的：这场会话的路径翻译器（虚拟机档 guest↔宿主；本机档视图盘打开时宿主↔视图，否则 None = 恒等）。
/// 约束：它只把模型给的写法译成宿主路径**再送寻址**；安全判断仍在 `workspace::Sandbox::resolve`。
pub fn path_codec_of(
    root: &Path,
    probe: &dyn HostProbe,
    spec: &ExecSpec,
) -> Option<Arc<dyn PathCodec>> {
    let Some(guest) = guest_os_of(root, probe, spec) else {
        return crate::kernel::api::view_codec();
    };
    Some(Arc::new(
        crate::capabilities::vm::api::PathBridge::for_session(
            std::env::consts::OS,
            Some(guest.dialect()),
        ),
    ))
}

/// 目的：这份规格的 guest 镜像自带哪些运行能力（本机档或判不出来 = 空）。
pub fn guest_runtimes_of(root: &Path, probe: &dyn HostProbe, spec: &ExecSpec) -> Vec<String> {
    guest_image_of(root, probe, spec)
        .map(|image| image.runtimes)
        .unwrap_or_default()
}

/// 目的：这次会话该用的包库——按 guest 平台筛过（分平台包只留与 guest 匹配的那份）。
pub fn library_for(lib: &Library, root: &Path, probe: &dyn HostProbe, spec: &ExecSpec) -> Library {
    pkgs::library_for_guest(lib, &guest_platform_of(root, probe, spec))
}

/// 目的：本机档的包计划：按宿主目标选包、产出 PATH/只读根；会话用 Require，无会话面用 First。
pub fn host_plan_for(
    lib: &Library,
    base: &BaseFacts,
    probe: &dyn HostProbe,
    spec: &ExecSpec,
    modules: &[Module],
    policy: PinPolicy,
) -> PackagePlan {
    let current_path = std::env::var_os("PATH").unwrap_or_default();
    let has_exe = |name: &str| probe.has_exe(name);
    let bin_ok = |path: &std::path::Path| probe.is_dir(path);
    let facts = pkgs::Facts {
        has_exe: &has_exe,
        bin_ok: &bin_ok,
        current_path: &current_path,
        base: &base.active,
        host_fallback: &base.fallback,
    };
    pkgs::plan_host(spec, modules, lib, policy, &facts)
}

/// 目的：按一份执行规格与模块清单派生计划：入口判据先于包选型（与旧 `plan` 的错误顺序一致）。
#[allow(clippy::too_many_arguments)]
pub fn plan_for_spec(
    root: &Path,
    probe: &dyn HostProbe,
    lib: &Library,
    spec: &ExecSpec,
    modules: &[Module],
    missing: &[crate::capabilities::vm::api::MissingEntry],
    guest_root: Option<&str>,
    net: bool,
) -> Result<crate::capabilities::vm::api::ExecPlan, String> {
    // 一次把话说全：入口判据在前，包选型问题在后（与旧的单一计划函数同一顺序）。
    let entry_diags = crate::capabilities::vm::api::plan(spec, Vec::new(), missing, net)
        .err()
        .unwrap_or_default();
    let package_plan = if spec.tier == Tier::Vm {
        pkgs::plan_guest(
            spec,
            modules,
            &library_for(lib, root, probe, spec),
            &guest_runtimes_of(root, probe, spec),
        )
    } else {
        pkgs::PackagePlan::default()
    };
    let mut parts: Vec<String> = Vec::new();
    if !entry_diags.is_empty() {
        parts.push(crate::capabilities::vm::api::diagnose_text_for(
            &entry_diags,
            guest_root,
        ));
    }
    if !package_plan.problems.is_empty() {
        parts.push(pkgs::diagnose_text(&package_plan.problems));
    }
    if !parts.is_empty() {
        return Err(parts.join("；"));
    }
    crate::capabilities::vm::api::plan(spec, package_plan.packages, &[], net)
        .map_err(|diags| crate::capabilities::vm::api::diagnose_text_for(&diags, guest_root))
}

/// 目的：本档位下不能执行工具的模块（模块 id → 缺的能力名）：建会话与重建时收口给工具环境。
pub fn unavailable_modules(
    root: &Path,
    probe: &dyn HostProbe,
    lib: &Library,
    base: &BaseFacts,
    spec: &ExecSpec,
    modules: &[Module],
) -> BTreeMap<String, Vec<String>> {
    if spec.tier == Tier::Vm {
        pkgs::plan_guest(
            spec,
            modules,
            &library_for(lib, root, probe, spec),
            &guest_runtimes_of(root, probe, spec),
        )
        .unusable
    } else {
        host_plan_for(lib, base, probe, spec, modules, pkgs::PinPolicy::Require).unusable
    }
}
