//! 目的：会话装配要用的**执行事实**——guest 镜像与平台、包库、按规格派生的执行计划、模块可用性与宿主只读根。
//! 管：从 vm / packages / registry 取事实后派生 `ExecPlan`、guest 路径翻译器、网络开关与只读根。
//! 不管：计划怎么被执行（vm 能力）、依赖怎么装（provision）、报告视图怎么呈现（仍在 `conductor`）。
//! 联动：`assemble.rs` 与 `rebuild.rs` 用这些事实装会话；`conductor` 的运行包报告与建工作流程经 `turn::api` 取用。

use crate::capabilities::modules::api::Module;
use crate::capabilities::packages::api as pkgs;
use crate::capabilities::registry::api::Registry;
use crate::capabilities::session::api::SessionMeta;
use crate::kernel::ports::HostProbe;

use crate::capabilities::turn::service::session::Turn;

impl Turn {
    /// 目的：这份规格的 guest 镜像自带哪些运行能力（本机档或判不出来 = 空）。
    pub(crate) fn guest_runtimes_of(
        &self,
        spec: &crate::kernel::api::ExecSpec,
        probe: &dyn HostProbe,
    ) -> Vec<String> {
        self.guest_image_of(spec, probe)
            .map(|image| image.runtimes)
            .unwrap_or_default()
    }

    /// 目的：这次会话的 guest 镜像（本机档或选定虚拟根判不出来 = None）——平台名与 guest OS 都从它派生。
    /// 约束：虚拟根名先经 `resolve_guest` 解析成那一格目录，再按布局判定；两者任一失败都给 None。
    fn guest_image_of(
        &self,
        spec: &crate::kernel::api::ExecSpec,
        probe: &dyn HostProbe,
    ) -> Option<crate::capabilities::vm::api::GuestImage> {
        if spec.tier != crate::kernel::api::Tier::Vm {
            return None;
        }
        let name = spec.guest.as_deref()?;
        let dir = crate::capabilities::vm::api::resolve_guest(&self.root, name).ok()?;
        crate::capabilities::vm::api::classify_base(&dir, probe).ok()
    }

    /// 目的：这次虚拟机档会话的 guest 平台名（os-arch）；本机档或判不出来给空串。
    fn guest_platform_of(
        &self,
        spec: &crate::kernel::api::ExecSpec,
        probe: &dyn HostProbe,
    ) -> String {
        self.guest_image_of(spec, probe)
            .map(|image| crate::capabilities::vm::api::guest_platform(image.os, image.arch))
            .unwrap_or_default()
    }

    /// 目的：这场会话的路径翻译器（虚拟机档 guest↔宿主；本机档视图盘打开时宿主↔视图，否则 None = 恒等）。
    /// 约束：它只把模型给的写法译成宿主路径**再送寻址**；安全判断仍在 `workspace::Sandbox::resolve`。
    pub(crate) fn path_codec_of(
        &self,
        spec: &crate::kernel::api::ExecSpec,
        probe: &dyn HostProbe,
    ) -> Option<std::sync::Arc<dyn crate::kernel::ports::PathCodec>> {
        let Some(guest) = self.guest_os_of(spec, probe) else {
            return crate::kernel::api::view_codec();
        };
        Some(std::sync::Arc::new(
            crate::capabilities::vm::api::PathBridge::for_session(
                std::env::consts::OS,
                Some(guest.dialect()),
            ),
        ))
    }

    /// 目的：这次虚拟机档会话的 guest OS（本机档或判不出来 = None）——工作环境块按它选 guest 档。
    pub(crate) fn guest_os_of(
        &self,
        spec: &crate::kernel::api::ExecSpec,
        probe: &dyn HostProbe,
    ) -> Option<crate::capabilities::vm::api::GuestOs> {
        self.guest_image_of(spec, probe).map(|image| image.os)
    }

    /// 目的：这次会话该用的包库——按 guest 平台筛过（分平台包只留与 guest 匹配的那份）。
    fn library_for(
        &self,
        spec: &crate::kernel::api::ExecSpec,
        probe: &dyn HostProbe,
    ) -> pkgs::Library {
        pkgs::library_for_guest(
            &self.packages.library(),
            &self.guest_platform_of(spec, probe),
        )
    }

    /// 目的：本机档的包计划：按宿主目标选包、产出 PATH/只读根；会话用 Require，无会话面用 First。
    pub(crate) fn host_plan_for(
        &self,
        spec: &crate::kernel::api::ExecSpec,
        modules: &[Module],
        policy: pkgs::PinPolicy,
        probe: &dyn HostProbe,
    ) -> pkgs::PackagePlan {
        let current_path = std::env::var_os("PATH").unwrap_or_default();
        let has_exe = |name: &str| probe.has_exe(name);
        let bin_ok = |path: &std::path::Path| probe.is_dir(path);
        let base = self.packages.base();
        let facts = pkgs::Facts {
            has_exe: &has_exe,
            bin_ok: &bin_ok,
            current_path: &current_path,
            base: &base.active,
            host_fallback: &base.fallback,
        };
        pkgs::plan_host(spec, modules, &self.packages.library(), policy, &facts)
    }

    /// 目的：按一份执行规格与模块清单派生计划：入口判据先于包选型（与旧 `plan` 的错误顺序一致）。
    pub(crate) fn plan_for_spec(
        &self,
        spec: &crate::kernel::api::ExecSpec,
        modules: &[Module],
        missing: &[crate::capabilities::vm::api::MissingEntry],
        guest_root: Option<&str>,
        probe: &dyn HostProbe,
        registry: &dyn Registry,
    ) -> Result<crate::capabilities::vm::api::ExecPlan, String> {
        // 一次把话说全：入口判据在前，包选型问题在后（与旧的单一计划函数同一顺序）。
        let entry_diags = crate::capabilities::vm::api::plan(
            spec,
            Vec::new(),
            missing,
            self.network_allowed(registry),
        )
        .err()
        .unwrap_or_default();
        let package_plan = if spec.tier == crate::kernel::api::Tier::Vm {
            pkgs::plan_guest(
                spec,
                modules,
                &self.library_for(spec, probe),
                &self.guest_runtimes_of(spec, probe),
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
        crate::capabilities::vm::api::plan(
            spec,
            package_plan.packages,
            &[],
            self.network_allowed(registry),
        )
        .map_err(|diags| crate::capabilities::vm::api::diagnose_text_for(&diags, guest_root))
    }

    pub(crate) fn plan_for_meta_with(
        &self,
        meta: &SessionMeta,
        roster: &crate::capabilities::modules::api::Roster,
        probe: &dyn HostProbe,
        registry: &dyn Registry,
    ) -> Result<crate::capabilities::vm::api::ExecPlan, String> {
        let mut ids: Vec<&str> = meta
            .agents
            .iter()
            .flat_map(|a| a.modules.iter().map(String::as_str))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        let session_modules: Vec<Module> = roster
            .modules
            .iter()
            .filter(|m| ids.iter().any(|id| *id == m.manifest.id))
            .cloned()
            .collect();
        let guest = self.guest_os_of(&meta.exec, probe).map(|g| g.as_str());
        let target_os = guest.unwrap_or(std::env::consts::OS);
        let missing: Vec<crate::capabilities::vm::api::MissingEntry> =
            crate::capabilities::vm::api::missing_entries_for(&session_modules, target_os, &|p| {
                probe.is_executable(p)
                    && (target_os == std::env::consts::OS
                        || crate::capabilities::vm::api::runs_on_target(p, target_os))
            });
        self.plan_for_spec(
            &meta.exec,
            &session_modules,
            &missing,
            meta.exec.guest.as_deref(),
            probe,
            registry,
        )
    }

    /// 目的：本档位下不能执行工具的模块（模块 id → 缺的能力名）：建会话与重建时收口给工具环境。
    pub(crate) fn unavailable_modules(
        &self,
        spec: &crate::kernel::api::ExecSpec,
        modules: &[Module],
        probe: &dyn HostProbe,
    ) -> std::collections::BTreeMap<String, Vec<String>> {
        if spec.tier == crate::kernel::api::Tier::Vm {
            pkgs::plan_guest(
                spec,
                modules,
                &self.library_for(spec, probe),
                &self.guest_runtimes_of(spec, probe),
            )
            .unusable
        } else {
            self.host_plan_for(spec, modules, pkgs::PinPolicy::Require, probe)
                .unusable
        }
    }

    /// 目的：全局网络开关（工具 / guest / 常驻服务 / 构建都看它）。
    pub(crate) fn network_allowed(&self, registry: &dyn Registry) -> bool {
        registry.app().sandbox.policy.network
    }

    /// 目的：用户显式授权的主机只读根（`settings.yaml` 的 `sandbox.policy.host_read_roots`）。
    ///  策略层只带事实：哪些目录只读可达由用户定，只读位怎么落由适配层定。
    ///  空 = 一个都不放行（默认不动本机任何权限项）。
    pub(crate) fn host_read_roots(&self, registry: &dyn Registry) -> Vec<std::path::PathBuf> {
        registry
            .app()
            .sandbox
            .policy
            .host_read_roots
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(std::path::PathBuf::from)
            .collect()
    }
}
